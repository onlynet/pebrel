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

#[cfg(test)]
mod fetch_tests;
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

    /// 测试探针：内容是否已落地（Loaded 态）。
    #[cfg(test)]
    fn overlay_loaded(&self) -> bool {
        matches!(self.phase, Phase::Loaded(_))
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

    /// 获取结果落地：generation 不匹配一律丢弃（不覆盖新状态）。
    pub(in crate::gpui_shell::workspace) fn apply_fetch_result(
        &mut self,
        generation: u64,
        fetched: Result<fetch::Fetched, ()>,
        cx: &mut Context<Self>,
    ) {
        if self.diff_overlay.generation != generation || self.diff_overlay.target.is_none() {
            return;
        }
        self.diff_overlay.phase = match fetched {
            Ok(fetch::Fetched::Patch(patch)) => {
                let mut rows = Vec::new();
                let mut pairs = Vec::new();
                for file in &patch.files {
                    rows.extend(pair::unified_rows(file));
                    pairs.extend(pair::split_rows(file));
                }
                if rows.is_empty() {
                    // 例如仅权限位变化：没有文本行，显示「无文本改动」。
                    Phase::NoChanges
                } else {
                    Phase::Loaded(Box::new(Loaded { rows, pairs, truncated: false }))
                }
            },
            Ok(fetch::Fetched::Untracked { lines, truncated }) => Phase::Loaded(Box::new(Loaded {
                rows: pair::untracked_unified(&lines),
                pairs: pair::untracked_split(&lines),
                truncated,
            })),
            Ok(fetch::Fetched::Raw { lines, truncated }) => Phase::Raw { lines, truncated },
            Ok(fetch::Fetched::Binary) => Phase::Binary,
            Err(()) => Phase::Error,
        };
        // 列表计数只在落地时可知；这里同步 reset，避免渲染路径改 ListState。
        // uniform 高度提示：未测量项按 22px 计，否则滚动条按已测范围钳制。
        let item_count = match &self.diff_overlay.phase {
            Phase::Loaded(loaded) => Some(loaded.rows.len()),
            Phase::Raw { lines, .. } => Some(lines.len()),
            _ => None,
        };
        if let Some(count) = item_count {
            self.diff_overlay.scroll.reset_with_uniform_height(count, px(22.0));
        }
        cx.notify();
    }

    /// Error 态重试：同一目标重发获取，generation 递增作废在途响应。
    fn retry_diff_fetch(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.diff_overlay.target.clone() else { return };
        self.diff_overlay.phase = Phase::Loading;
        self.diff_overlay.generation = self.diff_overlay.generation.wrapping_add(1);
        fetch::spawn(self.diff_overlay.generation, &target, cx);
        cx.notify();
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
                Phase::Binary => centered_note(
                    theme.muted_foreground,
                    language.text(Message::VcsDiffBinary),
                ),
                Phase::NoChanges => centered_note(
                    theme.muted_foreground,
                    language.text(Message::VcsDiffNoChanges),
                ),
                Phase::Error => v_flex()
                    .id("diff-overlay-error")
                    .debug_selector(|| "diff-overlay-error".to_string())
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child(language.text(Message::VcsDiffFailed))
                    .child(
                        Button::new("diff-retry")
                            .label(language.text(Message::CommonRetry))
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.retry_diff_fetch(cx);
                            })),
                    )
                    .into_any_element(),
                // Task 7 只渲染 unified（mode 切换在 Task 8 接管分派）。
                Phase::Loaded(loaded) => {
                    let tail = truncated_tail(language, loaded.truncated);
                    self.render_scrolled_overlay_body(tail, cx).into_any_element()
                },
                Phase::Raw { lines, truncated } => {
                    let tail = truncated_tail(language, *truncated);
                    self.render_scrolled_overlay_body(tail, cx).into_any_element()
                },
            })
            .into_any_element()
    }

    /// Loaded/Raw 共用的滚动体：虚拟列表（行渲染按 phase 分派）+ 截断尾行。
    fn render_scrolled_overlay_body(
        &self,
        tail: Option<String>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let scroll = self.diff_overlay.scroll.clone();
        let owner = cx.entity().downgrade();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("diff-overlay-body")
                    .debug_selector(|| "diff-overlay-body".to_string())
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        gpui::list(scroll, move |index, _window, cx| {
                            owner
                                .update(cx, |workspace, cx| workspace.render_overlay_row(index, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        })
                        .size_full(),
                    ),
            )
            .when_some(tail, |body, tail| {
                body.child(
                    h_flex()
                        .flex_shrink_0()
                        .h(px(26.0))
                        .items_center()
                        .justify_center()
                        .text_color(muted)
                        .child(tail),
                )
            })
    }

    /// 虚拟列表行：Loaded → unified 行（Task 7 只此一种）；Raw → 等宽原文。
    fn render_overlay_row(&mut self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let (row, raw) = match &self.diff_overlay.phase {
            Phase::Loaded(loaded) => (loaded.rows.get(index), None),
            Phase::Raw { lines, .. } => (None, lines.get(index)),
            _ => return div().into_any_element(),
        };
        if let Some(line) = raw {
            return h_flex()
                .id(SharedString::from(format!("diff-overlay-row-raw-{index}")))
                .debug_selector(move || format!("diff-overlay-row-raw-{index}"))
                .w_full()
                .px_3()
                .py_1()
                .items_start()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.font_size - px(1.0))
                .text_color(theme.foreground)
                .line_height(gpui::relative(1.35))
                .child(line.clone())
                .into_any_element();
        }
        let Some(row) = row else { return div().into_any_element() };
        match row {
            pair::DisplayRow::FullWidth { text } => {
                // hunk 头高亮；note（"\ No newline..."）按弱化说明处理。
                // 带 hunk 上下文的头可达数百字符：单行截断，不换行不撑高。
                let hunk = text.starts_with("@@");
                h_flex()
                    .id(SharedString::from(format!("diff-overlay-row-unified-{index}")))
                    .debug_selector(move || format!("diff-overlay-row-unified-{index}"))
                    .w_full()
                    .px_3()
                    .h(px(22.0))
                    .flex_shrink_0()
                    .items_center()
                    .when(hunk, |row| row.bg(theme.primary.opacity(0.08)))
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.font_size - px(1.0))
                    .text_color(if hunk { theme.primary } else { theme.muted_foreground })
                    .child(div().flex_1().min_w_0().truncate().child(text.clone()))
                    .into_any_element()
            },
            pair::DisplayRow::Context { old, new, text } => {
                unified_line_row(index, old.or(*new), text, theme.foreground, None, theme)
            },
            pair::DisplayRow::Added { new, text } => unified_line_row(
                index,
                *new,
                text,
                theme.success,
                Some(theme.success.opacity(0.13)),
                theme,
            ),
            pair::DisplayRow::Deleted { old, text } => unified_line_row(
                index,
                *old,
                text,
                theme.danger,
                Some(theme.danger.opacity(0.13)),
                theme,
            ),
        }
    }
}

/// unified 代码行：行号槽（右对齐弱化）+ 可换行文本（不截断，软换行）。
fn unified_line_row(
    index: usize,
    no: Option<u32>,
    text: &str,
    fg: gpui::Hsla,
    bg: Option<gpui::Hsla>,
    theme: &gpui_component::Theme,
) -> gpui::AnyElement {
    h_flex()
        .id(SharedString::from(format!("diff-overlay-row-unified-{index}")))
        .debug_selector(move || format!("diff-overlay-row-unified-{index}"))
        .w_full()
        .pl_3()
        .pr_2()
        .items_start()
        .when_some(bg, |row, bg| row.bg(bg))
        .child(
            div()
                .w(px(48.0))
                .flex_shrink_0()
                .text_right()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.font_size - px(1.0))
                .text_color(theme.muted_foreground)
                .child(no.map(|n| n.to_string()).unwrap_or_default()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.font_size - px(1.0))
                .text_color(fg)
                .line_height(gpui::relative(1.35))
                .child(text.to_string()),
        )
        .into_any_element()
}

/// 二进制/无改动等占位态：居中弱化说明。
fn centered_note(color: gpui::Hsla, text: &str) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(color)
        .child(text.to_string())
        .into_any_element()
}

/// 截断尾行（超限提示，行数/字节两种成因通用）；未截断返回 None。
fn truncated_tail(language: crate::i18n::UiLanguage, truncated: bool) -> Option<String> {
    truncated.then(|| language.text(Message::VcsDiffTooLarge).to_string())
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
