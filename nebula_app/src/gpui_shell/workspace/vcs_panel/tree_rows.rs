//! Git 变更 section 的目录树行构建：只消费 `(status, 相对路径)` 列表，不问
//! 本地还是远端——两种来源共用同一 `GitInfo` 载体，行模型天然同构。
//!
//! 展开状态由调用方（`VcsList.collapsed_dirs`）持有：集合记录**已显式折叠**
//! 的目录键，默认全展开；点击目录行翻转键后重建行。
use super::list_model::{RowOps, VcsRow};
use super::*;
use crate::i18n::Message;
use std::collections::HashSet;

/// 折叠状态键：section 前缀 + 完整目录相对路径。
pub(super) fn dir_key(section_id: &str, dir_path: &str) -> String {
    format!("{section_id}:{dir_path}")
}

/// 目录子树聚合：子目录（名称 → 子树）与本目录直接文件的条目序号。
struct DirNode {
    dirs: Vec<(String, DirNode)>,
    files: Vec<usize>,
}

/// 递归 emit 需要的 section 上下文（参数收拢，避免函数踩参数上限）。
struct SectionCtx<'a> {
    id: &'static str,
    entries: &'a [(char, String)],
    ops: RowOps,
    collapsed: &'a HashSet<String>,
}

impl DirNode {
    fn new() -> Self {
        Self { dirs: Vec::new(), files: Vec::new() }
    }

    fn insert(&mut self, parts: &[&str], index: usize) {
        let Some((first, rest)) = parts.split_first() else { return };
        if rest.is_empty() {
            self.files.push(index);
            return;
        }
        let slot = self.dirs.iter_mut().find(|(name, _)| name == first);
        let node = match slot {
            Some((_, node)) => node,
            None => {
                self.dirs.push((first.to_string(), DirNode::new()));
                &mut self.dirs.last_mut().unwrap().1
            },
        };
        node.insert(rest, index);
    }

    /// 该目录下（含嵌套）的文件条目总数。
    fn total_files(&self) -> usize {
        self.files.len() + self.dirs.iter().map(|(_, node)| node.total_files()).sum::<usize>()
    }
}

/// 把一个 section 的平铺条目构建成「标题 + 目录行 + 文件行」序列，追加到
/// `rows`。标题文案由调用方传入（本模块不做 section_id→文案映射）。
/// `Change.index` 严格等于条目在原列表中的序号（debug selector 与
/// 行内操作按它寻址，不得重排）。
pub(super) fn append_tree_section(
    rows: &mut Vec<VcsRow>,
    collapsed: &HashSet<String>,
    section_id: &'static str,
    message: Message,
    entries: &[(char, String)],
    ops: RowOps,
) {
    if entries.is_empty() {
        return;
    }
    let mut root = DirNode::new();
    for (index, (_, path)) in entries.iter().enumerate() {
        let parts: Vec<&str> = path.split('/').collect();
        root.insert(&parts, index);
    }
    rows.push(VcsRow::Heading { message: Some(message), count: entries.len() });
    let section = SectionCtx { id: section_id, entries, ops, collapsed };
    append_dir(rows, &section, &root, "", 0);
}

fn append_dir(
    rows: &mut Vec<VcsRow>,
    section: &SectionCtx<'_>,
    node: &DirNode,
    parent_path: &str,
    depth: usize,
) {
    // 目录在前、同级按名称排序；文件行按条目序号（git status 语义）。
    let mut dirs = node.dirs.iter().collect::<Vec<_>>();
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut files = node.files.clone();
    files.sort_unstable();
    for (name, child) in dirs {
        let path =
            if parent_path.is_empty() { name.clone() } else { format!("{parent_path}/{name}") };
        let is_expanded = !section.collapsed.contains(&dir_key(section.id, &path));
        rows.push(VcsRow::Dir {
            section_id: section.id,
            depth,
            path: path.clone(),
            name: name.clone(),
            count: child.total_files(),
            expanded: is_expanded,
        });
        if is_expanded {
            append_dir(rows, section, child, &path, depth + 1);
        }
    }
    for index in files {
        let (status, relative_path) = &section.entries[index];
        rows.push(VcsRow::Change {
            section_id: section.id,
            index,
            status: *status,
            relative_path: relative_path.clone(),
            ops: section.ops,
        });
    }
}

impl NebulaWorkspace {
    /// 目录行：chevron + 目录名 + 子树文件计数，按层级缩进；单击折叠/
    /// 展开（`toggle_dir` 翻转键并重建行模型）。高度与 Heading 行对齐。
    pub(super) fn render_vcs_dir_row(
        &mut self,
        section_id: &'static str,
        path: &str,
        name: &str,
        depth: usize,
        count: usize,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let hover = theme.list_hover;
        let path = path.to_string();
        h_flex()
            .id(SharedString::from(format!("git-tree-dir-{section_id}-{path}")))
            .debug_selector(|| format!("git-tree-dir-{section_id}-{path}"))
            .h(px(26.0))
            .w_full()
            .px_2()
            .pl(px(8.0) + px(14.0) * depth as f32)
            .gap_1()
            .items_center()
            .rounded_md()
            .hover(|row| row.bg(hover))
            .child(div().w(px(12.0)).text_xs().text_color(muted).child(if expanded {
                "▾"
            } else {
                "▸"
            }))
            .child(div().text_sm().min_w_0().truncate().child(name.to_string()))
            .child(div().text_xs().text_color(muted).child(count.to_string()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.vcs_list.toggle_dir(section_id, &path);
                cx.notify();
            }))
            .into_any_element()
    }
}
