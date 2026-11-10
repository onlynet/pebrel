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
        &entries(&[
            ('M', "src/main.rs"),
            ('M', "src/ui/mod.rs"),
            ('A', "docs/guide.md"),
        ]),
        RowOps::Unstaged,
    );
    let dirs: Vec<_> = rows.iter().filter_map(|r| match r {
        VcsRow::Dir { path, count, .. } => Some((path.clone(), *count)),
        _ => None,
    }).collect();
    assert!(dirs.contains(&("src".to_string(), 2)));
    assert!(dirs.contains(&("src/ui".to_string(), 1)));
    assert!(dirs.contains(&("docs".to_string(), 1)));
    // 文件行的 index 保持平铺条目序号（上游 list_tests 的 selector 依赖它）
    let change = rows.iter().find_map(|r| match r {
        VcsRow::Change { index, relative_path, .. } if relative_path == "src/ui/mod.rs" => Some(*index),
        _ => None,
    });
    assert_eq!(change, Some(1));
}

#[test]
fn collapsed_dir_hides_descendants() {
    let list = entries(&[('M', "src/a.rs"), ('M', "src/b.rs")]);
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(&mut rows, &collapsed, "changes", &list, RowOps::Unstaged);
    assert_eq!(rows.iter().filter(|r| matches!(r, VcsRow::Change { .. })).count(), 2);

    let mut rows = Vec::new();
    let mut collapsed = std::collections::HashSet::new();
    collapsed.insert(tree_rows::dir_key("changes", "src"));
    tree_rows::append_tree_section(&mut rows, &collapsed, "changes", &list, RowOps::Unstaged);
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
        &entries(&[('?', "README.md")]),
        RowOps::Unstaged,
    );
    assert!(!rows.iter().any(|r| matches!(r, VcsRow::Dir { .. })));
}
