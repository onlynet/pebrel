//! Git diff 浮层：盖在中间终端卡上的全幅对比视图。
//!
//! 生命周期独立于 tab——打开不建任务条目、关闭不动终端；数据按点击惰性获取，
//! generation 递增作废晚到响应（与 remote_git 同一合同）。本文件先落状态机
//! 与挂载几何；行渲染（Task 6-8）与关闭路径（Task 9）随后续任务填充。
use super::*;
use crate::display::side_panel::GitInfo;
use crate::i18n::Message;

pub(super) mod fetch;
mod pair;

#[cfg(all(test, feature = "gpui-test-support"))]
mod tests;

pub(crate) struct DiffOverlayState {
    /// `None` = 关闭。
    target: Option<OverlayTarget>,
    mode: DiffMode,
    generation: u64,
    phase: Phase,
    /// 内容获取时所基于的快照；render 入口用它检测快照更新（自动重取/关）。
    seen_snapshot: Option<Arc<GitInfo>>,
    /// 打开时的 (active tab, settings_open)；变化即自动关闭。
    tab_fingerprint: (usize, bool),
    scroll: gpui::ListState,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DiffMode {
    Split,
    Unified,
}

#[derive(Clone)]
struct OverlayTarget {
    site: fetch::Site,
    /// 相对仓库根的路径（显示与命令都用它）。
    path: String,
    semantics: fetch::Semantics,
    status: char,
    /// 快照来源标记（点击时由 rows.rs 传入）：true = 远端 SSH Git 视图。
    remote: bool,
}

enum Phase {
    Loading,
    Loaded(Box<Loaded>),
    /// 原文降级（解析失败/超限；Task 7 起会抵达）。
    Raw {
        lines: Vec<String>,
        truncated: bool,
    },
    Binary,
    /// diff 无文本改动（Task 7 起会抵达）。
    NoChanges,
    Error,
}

struct Loaded {
    rows: Vec<pair::DisplayRow>,
    pairs: Vec<pair::SplitPair>,
    truncated: bool,
}

impl Default for DiffOverlayState {
    fn default() -> Self {
        Self {
            target: None,
            mode: DiffMode::Split,
            generation: 0,
            phase: Phase::Loading,
            seen_snapshot: None,
            tab_fingerprint: (0, false),
            scroll: gpui::ListState::new(0, gpui::ListAlignment::Top, px(22.0)),
        }
    }
}

impl DiffOverlayState {
    fn is_open(&self) -> bool {
        self.target.is_some()
    }

    /// 测试探针：generation 递增凭证（同目标 no-op 断言用）。
    #[cfg(test)]
    fn overlay_generation(&self) -> u64 {
        self.generation
    }

    /// 测试探针：当前 target 的取数来源。
    #[cfg(test)]
    fn overlay_target_site(&self) -> Option<fetch::Site> {
        self.target.as_ref().map(|t| t.site.clone())
    }

    /// 测试探针：伪装「打开时所基于的快照」，驱动 Some→None 关闭路径。
    #[cfg(test)]
    fn force_seen_snapshot(&mut self, snapshot: Option<Arc<GitInfo>>) {
        self.seen_snapshot = snapshot;
    }
}

impl NebulaWorkspace {
    /// 文件行单击入口（site/semantics/remote 由 rows.rs 解析后传入）：
    /// 同目标在开即无操作；否则进入 Loading 并发起获取。
    pub(super) fn open_diff_overlay(
        &mut self,
        site: fetch::Site,
        path: String,
        semantics: fetch::Semantics,
        status: char,
        remote: bool,
        cx: &mut Context<Self>,
    ) {
        // 同目标判等必须连来源一起比：路径 + 语义相同但 site/remote 不同
        // （本地 pane 开着、抽屉切到远端视图再点同一路径）时若误判 no-op，
        // 浮层会继续显示上一个仓库的 diff。
        let same = self.diff_overlay.target.as_ref().is_some_and(|t| {
            t.path == path && t.semantics == semantics && t.remote == remote && t.site == site
        });
        if same {
            return;
        }
        let target = OverlayTarget { site, path, semantics, status, remote };
        self.diff_overlay.target = Some(target.clone());
        self.diff_overlay.phase = Phase::Loading;
        self.diff_overlay.generation = self.diff_overlay.generation.wrapping_add(1);
        self.diff_overlay.seen_snapshot = self.current_vcs_snapshot();
        self.diff_overlay.tab_fingerprint = (self.active, self.settings_open);
        self.diff_overlay.scroll = gpui::ListState::new(0, gpui::ListAlignment::Top, px(22.0));
        fetch::spawn(self.diff_overlay.generation, &target, cx);
        cx.notify();
    }

    pub(super) fn close_diff_overlay(&mut self, cx: &mut Context<Self>) {
        if self.diff_overlay.target.take().is_some() {
            self.diff_overlay.phase = Phase::Loading;
            cx.notify();
        }
    }

    /// render.rs 挂载点。tab 指纹变化 → 关闭；快照更新 → 文件仍在则重取、
    /// 不在则关。只做状态判定与调度，不同步等待任何 IO。
    pub(super) fn render_diff_overlay(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.diff_overlay.target.is_none() {
            return None;
        }
        let fingerprint = (self.active, self.settings_open);
        if fingerprint != self.diff_overlay.tab_fingerprint {
            self.close_diff_overlay(cx);
            return None;
        }
        let snapshot = self.current_vcs_snapshot();
        if let Some(current) = snapshot.as_ref() {
            let changed = self
                .diff_overlay
                .seen_snapshot
                .as_ref()
                .map(|seen| !Arc::ptr_eq(seen, current))
                .unwrap_or(true);
            if changed {
                let target = self.diff_overlay.target.clone().unwrap();
                if !target_still_listed(current, &target.path, target.semantics) {
                    self.close_diff_overlay(cx);
                    return None;
                }
                self.diff_overlay.seen_snapshot = snapshot.clone();
                self.diff_overlay.generation = self.diff_overlay.generation.wrapping_add(1);
                self.diff_overlay.phase = Phase::Loading;
                fetch::spawn(self.diff_overlay.generation, &target, cx);
            }
        } else if self.diff_overlay.seen_snapshot.is_some() {
            // 列表来源整体消失（仓库退出/刷新失败）：按「文件已不在列表」的
            // 保守延伸直接关闭，不留指向已消失快照的浮层。
            self.close_diff_overlay(cx);
            return None;
        }
        Some(self.render_overlay_body(cx))
    }

    fn current_vcs_snapshot(&self) -> Option<Arc<GitInfo>> {
        match self.diff_overlay.target.as_ref().map(|t| t.remote) {
            Some(true) => self.remote_git.snapshot_for_overlay(),
            _ => self.side_panel.git_snapshot(),
        }
    }

    fn render_overlay_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let language = crate::gpui_shell::config::ui_language(cx);
        let status = self.diff_overlay.target.as_ref().map(|t| t.status);
        let path: SharedString =
            self.diff_overlay.target.as_ref().map(|t| t.path.clone()).unwrap_or_default().into();
        let semantics_label = self.diff_overlay.target.as_ref().map(|t| match t.semantics {
            fetch::Semantics::Unstaged => language.text(Message::VcsDiffWorktreeUnstaged),
            fetch::Semantics::Staged => language.text(Message::VcsDiffStagedHead),
            fetch::Semantics::Untracked => language.text(Message::VcsDiffUntracked),
            fetch::Semantics::ConflictVsHead => language.text(Message::VcsDiffConflictHead),
        });
        let status_color = status.map(|status| match status {
            'A' | '?' => theme.success,
            'D' | '!' => theme.danger,
            'C' | 'U' => theme.danger,
            _ => theme.warning,
        });
        div()
            .id("diff-overlay")
            .debug_selector(|| "diff-overlay".to_string())
            .absolute()
            .inset_0()
            // 与中心卡内容底色同一 token（render.rs 同款取法）：视觉上即
            // 「同一张卡上的另一层」，随主题/壁纸形态联动。
            .bg(crate::gpui_shell::theme::card_content_bg(cx))
            .flex()
            .flex_col()
            .child(
                h_flex()
                    .h(px(34.0))
                    .flex_shrink_0()
                    .gap_2()
                    .px_3()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .when_some(status.zip(status_color), |row, (status, color)| {
                        row.child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .text_size(theme.font_size - px(2.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(color)
                                .child(status.to_string()),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme.font_size - px(1.0))
                            .truncate()
                            .child(path),
                    )
                    .when_some(semantics_label, |row, label| {
                        row.child(
                            div().text_xs().text_color(theme.muted_foreground).child(label),
                        )
                    })
                    // 模式切换（Task 8）与关闭按钮（Task 9）随后续任务填充。
                    .child(
                        div()
                            .id("diff-close")
                            .text_color(theme.muted_foreground)
                            .child("✕"),
                    ),
            )
            .child(match &self.diff_overlay.phase {
                Phase::Loading => div()
                    .id("diff-overlay-loading")
                    .debug_selector(|| "diff-overlay-loading".to_string())
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child("⋯")
                    .into_any_element(),
                _ => div().flex_1().into_any_element(),
            })
            .into_any_element()
    }
}

/// 快照更新后目标文件是否仍在对应列表（`GitInfo` 在上游，判定放浮层侧）。
fn target_still_listed(info: &GitInfo, path: &str, semantics: fetch::Semantics) -> bool {
    let list = match semantics {
        fetch::Semantics::Staged => &info.staged,
        fetch::Semantics::ConflictVsHead => &info.conflicts,
        _ => &info.unstaged,
    };
    list.iter().any(|(_, p)| p == path)
}
