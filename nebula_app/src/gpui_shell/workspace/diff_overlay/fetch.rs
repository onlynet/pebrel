//! diff 取数：本地后台线程 / 远端 SSH exec（Task 7/10 实装）。
use super::*;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Debug)]
pub(in crate::gpui_shell::workspace) enum Site {
    Local { root: PathBuf },
    Remote { destination: String, root: String },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::gpui_shell::workspace) enum Semantics {
    Unstaged,
    Staged,
    Untracked,
    ConflictVsHead,
}

/// 发起一次 diff 获取。Task 7 实装（本地后台线程）；Task 10 补远端分支。
/// generation 作废合同在 apply 端。
pub(super) fn spawn(
    _generation: u64,
    _target: &super::OverlayTarget,
    _cx: &mut Context<NebulaWorkspace>,
) {
}
