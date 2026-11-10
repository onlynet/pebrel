//! Stable presentation data derived once per repository snapshot or selected view.
use super::*;
use crate::display::side_panel::{GitCommit, GitInfo, GitPanelView, VcsKind};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum RowOps {
    Unstaged,
    Staged,
    Conflict,
    Svn,
}

#[derive(Clone)]
pub(super) enum VcsRow {
    Heading {
        message: Option<Message>,
        count: usize,
    },
    Empty(Message),
    History {
        index: usize,
        commit: GitCommit,
        graph: GitGraphRow,
    },
    Change {
        section_id: &'static str,
        index: usize,
        status: char,
        relative_path: String,
        ops: RowOps,
    },
    Dir {
        section_id: &'static str,
        /// 完整目录相对路径（如 `src/ui`），展开状态键与点击都用它。
        path: String,
        /// 本级目录名（仅显示用）。
        name: String,
        depth: usize,
        /// 该目录子树内变更文件总数。
        count: usize,
        expanded: bool,
    },
}

pub(in crate::gpui_shell::workspace) struct VcsList {
    snapshot: Option<Arc<GitInfo>>,
    #[cfg(test)]
    pub(super) rendered: Vec<usize>,
    view: Option<GitPanelView>,
    root: Option<PathBuf>,
    /// 快照来自远端 pane（SSH Git 视图）：行内写操作改走远端 exec、双击开
    /// 远端文档、本地专属入口（merge 编辑器、SVN 菜单）不画。行模型不变。
    pub(super) remote: bool,
    /// Git 视图目录树的显式折叠集合（键 = `tree_rows::dir_key`）；默认
    /// 全展开，SVN/History 行不经它。折叠状态跨快照与目录切换保留
    /// （键按相对路径，同名目录视为同一折叠意图）。
    pub(super) collapsed_dirs: std::collections::HashSet<String>,
    rows: Vec<VcsRow>,
    lane_layout: (f32, f32),
    pub(super) scroll: gpui::ListState,
}

impl Default for VcsList {
    fn default() -> Self {
        Self {
            snapshot: None,
            #[cfg(test)]
            rendered: Vec::new(),
            view: None,
            root: None,
            remote: false,
            collapsed_dirs: std::collections::HashSet::new(),
            rows: Vec::new(),
            lane_layout: (0.0, 0.0),
            scroll: gpui::ListState::new(0, gpui::ListAlignment::Top, px(34.0)),
        }
    }
}

impl VcsList {
    pub(in crate::gpui_shell::workspace) fn sync(
        &mut self,
        snapshot: Option<Arc<GitInfo>>,
        view: GitPanelView,
        root: Option<PathBuf>,
        remote: bool,
    ) {
        let same_snapshot = match (&self.snapshot, &snapshot) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let same_location = self.view == Some(view) && self.root == root;
        self.remote = remote;
        if same_snapshot && same_location {
            return;
        }
        self.snapshot = snapshot;
        self.view = Some(view);
        self.root = root;
        self.rows.clear();
        if let Some(info) = self.snapshot.as_ref() {
            if info.vcs == VcsKind::Git && view == GitPanelView::History {
                let graphs = git_graph_rows(&info.history);
                self.lane_layout = git_lane_layout(&graphs);
                self.rows.extend(
                    info.history
                        .iter()
                        .cloned()
                        .zip(graphs)
                        .enumerate()
                        .map(|(index, (commit, graph))| VcsRow::History { index, commit, graph }),
                );
                if self.rows.is_empty() {
                    self.rows.push(VcsRow::Empty(Message::VcsNoHistory));
                }
            } else {
                let conflicts: std::collections::HashSet<_> =
                    info.conflicts.iter().map(|(_, p)| p.as_str()).collect();
                let is_git = info.vcs == VcsKind::Git;
                if is_git {
                    super::tree_rows::append_tree_section(
                        &mut self.rows,
                        &self.collapsed_dirs,
                        "conflicts",
                        Message::VcsMergeConflicts,
                        &info.conflicts,
                        RowOps::Conflict,
                    );
                    if view == GitPanelView::Changes {
                        // 冲突路径同时存在于多个列表；只允许在冲突段渲染
                        //（沿用旧 section 闭包 exclude_conflicts 语义）。
                        let staged: Vec<(char, String)> = info
                            .staged
                            .iter()
                            .filter(|(_, p)| !conflicts.contains(p.as_str()))
                            .cloned()
                            .collect();
                        let unstaged: Vec<(char, String)> = info
                            .unstaged
                            .iter()
                            .filter(|(_, p)| !conflicts.contains(p.as_str()))
                            .cloned()
                            .collect();
                        super::tree_rows::append_tree_section(
                            &mut self.rows,
                            &self.collapsed_dirs,
                            "staged",
                            Message::VcsStaged,
                            &staged,
                            RowOps::Staged,
                        );
                        super::tree_rows::append_tree_section(
                            &mut self.rows,
                            &self.collapsed_dirs,
                            "changes",
                            Message::VcsChanges,
                            &unstaged,
                            RowOps::Unstaged,
                        );
                    }
                } else {
                    let mut section =
                        |id, message, entries: &[(char, String)], ops, exclude_conflicts: bool| {
                            let changes: Vec<_> = entries
                                .iter()
                                .filter(|(_, p)| {
                                    !exclude_conflicts || !conflicts.contains(p.as_str())
                                })
                                .enumerate()
                                .map(|(index, (status, path))| VcsRow::Change {
                                    section_id: id,
                                    index,
                                    status: *status,
                                    relative_path: path.clone(),
                                    ops,
                                })
                                .collect();
                            if !changes.is_empty() {
                                self.rows.push(VcsRow::Heading { message, count: changes.len() });
                                self.rows.extend(changes);
                            }
                        };
                    section(
                        "conflicts",
                        Some(Message::VcsMergeConflicts),
                        &info.conflicts,
                        RowOps::Svn,
                        false,
                    );
                    match info.vcs {
                        VcsKind::Svn => section("svn", None, &info.unstaged, RowOps::Svn, true),
                        _ => {},
                    }
                }
                if self.rows.is_empty() && info.vcs != VcsKind::SvnRepository {
                    self.rows.push(VcsRow::Empty(if is_git && view == GitPanelView::Conflicts {
                        Message::VcsNoConflicts
                    } else {
                        Message::VcsNoChanges
                    }));
                }
            }
        }
        if same_location {
            let offset = self.scroll.logical_scroll_top();
            self.scroll.splice(0..self.scroll.item_count(), self.rows.len());
            self.scroll.scroll_to(offset);
        } else {
            self.scroll.reset(self.rows.len());
        }
    }

    #[cfg(test)]
    pub(super) fn rows_vcs(&self) -> &[VcsRow] {
        &self.rows
    }
    pub(super) fn row(&self, index: usize) -> Option<&VcsRow> {
        self.rows.get(index)
    }
    /// 目录行点击：翻转折叠键并重建行（快照与位置不变，仅重排当前视图）。
    pub(super) fn toggle_dir(&mut self, section_id: &str, dir_path: &str) {
        let key = super::tree_rows::dir_key(section_id, dir_path);
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key);
        }
        let snapshot = self.snapshot.clone();
        let view = self.view.unwrap_or(GitPanelView::Changes);
        let root = self.root.clone();
        let remote = self.remote;
        self.rows.clear();
        self.snapshot = None; // 强制 sync 重建
        self.sync(snapshot, view, root, remote);
    }
    pub(super) fn root(&self) -> Option<&PathBuf> {
        self.root.as_ref()
    }
    pub(super) fn lane_layout(&self) -> (f32, f32) {
        self.lane_layout
    }
}
