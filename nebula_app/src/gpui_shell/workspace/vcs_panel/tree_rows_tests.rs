use super::list_model::{RowOps, VcsRow};
use super::*;

fn entries(paths: &[(char, &str)]) -> Vec<(char, String)> {
    paths.iter().map(|(s, p)| (*s, p.to_string())).collect()
}

#[test]
fn flat_files_stay_direct_children() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        Message::VcsChanges,
        &entries(&[('M', "a.rs"), ('A', "b.rs")]),
        RowOps::Unstaged,
    );
    assert!(matches!(rows[0], VcsRow::Heading { .. }));
    assert!(matches!(rows[1], VcsRow::Change { index: 0, .. }));
    assert!(matches!(rows[2], VcsRow::Change { index: 1, .. }));
}

#[test]
fn nested_paths_get_dir_rows_with_counts() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        Message::VcsChanges,
        &entries(&[('M', "src/main.rs"), ('M', "src/ui/mod.rs"), ('A', "docs/guide.md")]),
        RowOps::Unstaged,
    );
    let dirs: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            VcsRow::Dir { path, count, .. } => Some((path.clone(), *count)),
            _ => None,
        })
        .collect();
    assert!(dirs.contains(&("src".to_string(), 2)));
    assert!(dirs.contains(&("src/ui".to_string(), 1)));
    assert!(dirs.contains(&("docs".to_string(), 1)));
    // 文件行的 index 保持平铺条目序号（上游 list_tests 的 selector 依赖它）
    let change = rows.iter().find_map(|r| match r {
        VcsRow::Change { index, relative_path, .. } if relative_path == "src/ui/mod.rs" => {
            Some(*index)
        },
        _ => None,
    });
    assert_eq!(change, Some(1));
    // 全序列顺序：目录在前、同级按名称排序，文件按条目序号殿后
    let sequence: Vec<String> = rows
        .iter()
        .map(|r| match r {
            VcsRow::Heading { .. } => "H".to_string(),
            VcsRow::Dir { path, .. } => format!("D:{path}"),
            VcsRow::Change { index, relative_path, .. } => format!("C{index}:{relative_path}"),
            _ => "?".to_string(),
        })
        .collect();
    assert_eq!(
        sequence,
        vec![
            "H",
            "D:docs",
            "C2:docs/guide.md",
            "D:src",
            "D:src/ui",
            "C1:src/ui/mod.rs",
            "C0:src/main.rs",
        ]
    );
}

#[test]
fn collapsed_dir_hides_descendants() {
    let list = entries(&[('M', "src/a.rs"), ('M', "src/b.rs")]);
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        Message::VcsChanges,
        &list,
        RowOps::Unstaged,
    );
    assert_eq!(rows.iter().filter(|r| matches!(r, VcsRow::Change { .. })).count(), 2);

    let mut rows = Vec::new();
    let mut collapsed = std::collections::HashSet::new();
    collapsed.insert(tree_rows::dir_key("changes", "src"));
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        Message::VcsChanges,
        &list,
        RowOps::Unstaged,
    );
    assert_eq!(rows.iter().filter(|r| matches!(r, VcsRow::Change { .. })).count(), 0);
    // 折叠后目录行仍在
    assert!(rows.iter().any(|r| matches!(r, VcsRow::Dir { path, .. } if path == "src")));
}

#[test]
fn expansion_keys_are_scoped_per_section() {
    assert_ne!(tree_rows::dir_key("staged", "src"), tree_rows::dir_key("changes", "src"));
}

#[test]
fn untracked_path_without_slash_has_no_dir_row() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        Message::VcsChanges,
        &entries(&[('?', "README.md")]),
        RowOps::Unstaged,
    );
    assert!(!rows.iter().any(|r| matches!(r, VcsRow::Dir { .. })));
}

#[test]
fn vcs_list_sync_builds_tree_rows_for_git_changes() {
    use crate::display::side_panel::{GitInfo, GitPanelView};
    let info = GitInfo {
        staged: vec![('M', "Cargo.toml".into())],
        unstaged: vec![
            ('M', "src/main.rs".into()),
            ('M', "src/ui/mod.rs".into()),
            ('?', "README.md".into()),
        ],
        ..GitInfo::default()
    };
    let mut list = VcsList::default();
    list.sync(Some(Arc::new(info)), GitPanelView::Changes, Some(PathBuf::from("/repo")), false);
    let dirs: Vec<String> = list
        .rows_vcs()
        .iter()
        .filter_map(|r| match r {
            VcsRow::Dir { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(dirs, vec!["src".to_string(), "src/ui".to_string()]);
    // 未跟踪文件在根层级，直接是 Change 行
    assert!(list.rows_vcs().iter().any(|r| matches!(
        r,
        VcsRow::Change { relative_path, .. } if relative_path == "README.md"
    )));
}

#[test]
fn conflict_paths_render_only_in_the_conflicts_section() {
    use crate::display::side_panel::{GitInfo, GitPanelView};
    let info = GitInfo {
        conflicts: vec![('U', "src/both.rs".into())],
        staged: vec![('M', "src/both.rs".into())],
        unstaged: vec![('M', "src/other.rs".into()), ('M', "src/both.rs".into())],
        ..GitInfo::default()
    };
    let mut list = VcsList::default();
    list.sync(Some(Arc::new(info)), GitPanelView::Changes, Some(PathBuf::from("/repo")), false);
    let rows = list.rows_vcs();
    // 冲突段（首个 Heading）里有它；staged/changes 段（后续 Heading 起）里没有
    let sections: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(ix, r)| matches!(r, VcsRow::Heading { .. }).then_some(ix))
        .collect();
    assert_eq!(
        sections.len(),
        2,
        "conflicts + changes sections (staged filtered empty), got {}",
        sections.len()
    );
    let in_conflicts = rows[..sections[1]].iter().any(|r| {
        matches!(
            r,
            VcsRow::Change { relative_path, .. } if relative_path == "src/both.rs"
        )
    });
    let in_later_sections = rows[sections[1]..].iter().any(|r| {
        matches!(
            r,
            VcsRow::Change { relative_path, .. } if relative_path == "src/both.rs"
        )
    });
    assert!(in_conflicts, "conflict path must render in conflicts section");
    assert!(!in_later_sections, "conflict path must not double-render");
}

#[test]
fn untracked_directory_entry_yields_dir_row_with_nested_change() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        Message::VcsChanges,
        &[('?', "build/".to_string())],
        RowOps::Unstaged,
    );
    // "build/" → 目录 build（计数 1），其下是路径原文的 Change 行；无空名目录行
    assert!(rows.iter().any(|r| matches!(r,
        VcsRow::Dir { path, count, .. } if path == "build" && *count == 1)));
    assert!(rows.iter().any(|r| matches!(r,
        VcsRow::Change { relative_path, .. } if relative_path == "build/")));
    assert!(!rows.iter().any(|r| matches!(r, VcsRow::Dir { path, .. } if path.is_empty())));
}
