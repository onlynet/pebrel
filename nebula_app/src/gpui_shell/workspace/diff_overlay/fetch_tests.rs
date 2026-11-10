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

/// 远端命令行构造：root 与路径一律经 ai_hook::remote::quote 单引号包裹
/// （空格等安全到达远端 shell）；staged 加 --cached、冲突加 HEAD；未跟踪
/// 不走 git diff（exec::capture 对非零退出丢 stdout，no-index 有差异时
/// 退出码 1），用 head -c 按字节上限直读，且必须与仓库根拼成绝对路径
/// ——exec 通道的工作目录是远端 HOME 而不是仓库根。
#[test]
fn remote_command_shapes_are_shell_safe() {
    let site = fetch::Site::Remote { destination: "h".into(), root: "/my repo".into() };
    let cmd = fetch::remote_command(&site, "src/a b.rs", fetch::Semantics::Staged);
    assert!(cmd.starts_with("git -C '/my repo' diff"));
    assert!(cmd.contains("--cached"));
    assert!(cmd.contains("'src/a b.rs'"));
    let conflict = fetch::remote_command(&site, "f.rs", fetch::Semantics::ConflictVsHead);
    assert!(conflict.contains(" HEAD -- 'f.rs'"));
    let untracked = fetch::remote_command(&site, "new.txt", fetch::Semantics::Untracked);
    assert!(untracked.starts_with("head -c 262144 -- '/my repo/new.txt'"));
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
