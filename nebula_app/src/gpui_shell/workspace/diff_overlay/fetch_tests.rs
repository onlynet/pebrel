//! diff 取数模型测试：真实 git 子进程 + 解析降级（不依赖 gpui-test-support，
//! 组织方式镜像 vcs_panel 的 tree_rows_tests）。
use super::fetch;

fn git(root: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git").args(args).current_dir(root).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn local_fetch_covers_staged_unstaged_untracked_binary() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@t"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(root.join("bin.dat"), "A\0B").unwrap();
    std::fs::write(root.join("new.txt"), "n1\nn2\n").unwrap();
    git(root, &["add", "a.txt"]);
    std::fs::write(root.join("a.txt"), "one\nTWO\n").unwrap();

    let local = |path: &str, semantics: fetch::Semantics| {
        fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, path, semantics)
    };

    let staged = local("a.txt", fetch::Semantics::Staged).unwrap();
    assert!(matches!(staged, fetch::Fetched::Patch(_)));
    let unstaged = local("a.txt", fetch::Semantics::Unstaged).unwrap();
    assert!(matches!(unstaged, fetch::Fetched::Patch(_)));
    // staged 与 unstaged 是不同的 diff（--cached vs 工作区）：无 HEAD 时 staged
    // 对空树（2 增 0 删），unstaged 对索引（1 增 1 删）。
    assert_ne!(format!("{staged:?}"), format!("{unstaged:?}"));

    match local("new.txt", fetch::Semantics::Untracked).unwrap() {
        fetch::Fetched::Untracked { lines, truncated } => {
            assert_eq!(lines, vec!["n1".to_string(), "n2".to_string()]);
            assert!(!truncated);
        },
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        local("bin.dat", fetch::Semantics::Untracked).unwrap(),
        fetch::Fetched::Binary
    ));
    // 读失败（文件已消失）→ Err → Error 态可重试，不误报「二进制」。
    assert!(
        local("no-such.txt", fetch::Semantics::Untracked).is_err(),
        "unreadable untracked file must surface as an error, not Binary"
    );
}

#[test]
fn oversize_or_unparseable_diff_degrades_to_raw() {
    // 空输出（无改动）→ 空 Patch（落地端归入「无文本改动」），不渲染空白原文。
    match fetch::parse_diff_text("") {
        fetch::Fetched::Patch(patch) => assert!(patch.files.is_empty()),
        other => panic!("{other:?}"),
    }
    // 解析失败 → Raw 截断 2000 行
    let fetched = fetch::parse_diff_text("not a diff at all\n");
    match fetched {
        fetch::Fetched::Raw { lines, truncated } => {
            assert_eq!(lines, vec!["not a diff at all".to_string()]);
            assert!(!truncated);
        },
        other => panic!("{other:?}"),
    }
    let huge: String = "line\n".repeat(2500);
    match fetch::parse_diff_text(&huge) {
        fetch::Fetched::Raw { lines, truncated } => {
            assert_eq!(lines.len(), 2000);
            assert!(truncated);
        },
        other => panic!("{other:?}"),
    }
    // git 的二进制行 → Binary
    assert!(matches!(
        fetch::parse_diff_text("diff --git a/x b/x\nBinary files a/x and b/x differ\n"),
        fetch::Fetched::Binary
    ));
}
