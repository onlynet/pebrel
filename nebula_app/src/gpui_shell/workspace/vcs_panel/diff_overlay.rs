//! Git diff 浮层：盖在中间终端卡上的全幅对比视图。
//!
//! 生命周期独立于 tab——打开不建任务条目、关闭不动终端；数据按点击惰性获取，
//! generation 递增作废晚到响应（与 remote_git 同一合同）。行渲染按 mode 分派
//! （unified/split，默认 split 左右对比）；关闭走 ✕ / Esc / tab 指纹变化三条
//! 路径，打开接管键盘焦点、关闭归还活动 tab（§5.3）。
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
    /// 浮层根的键盘焦点句柄：打开即接管（Esc 可达），关闭归还。
    focus: Option<gpui::FocusHandle>,
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
            focus: None,
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

    /// 测试探针：头部显示的相对路径（原地换文件的断言用）。
    #[cfg(test)]
    fn overlay_target_path(&self) -> Option<String> {
        self.target.as_ref().map(|t| t.path.clone())
    }

    /// 测试探针：头部状态字母（快照重取后的状态刷新断言用）。
    #[cfg(test)]
    fn overlay_target_status(&self) -> Option<char> {
        self.target.as_ref().map(|t| t.status)
    }

    /// 测试探针：内容是否已落地（Loaded 态）。
    #[cfg(test)]
    fn overlay_loaded(&self) -> bool {
        matches!(self.phase, Phase::Loaded(_))
    }

    /// 测试探针：内容是否落在 Error 态（远端取数不可用的回归断言用）。
    #[cfg(test)]
    fn overlay_phase_is_error(&self) -> bool {
        matches!(self.phase, Phase::Error)
    }

    /// 测试探针：伪装「打开时所基于的快照」，驱动 Some→None 关闭路径。
    #[cfg(test)]
    fn force_seen_snapshot(&mut self, snapshot: Option<Arc<GitInfo>>) {
        self.seen_snapshot = snapshot;
    }

    /// 测试探针：滚到列表末尾（模式切换后末行可达性的回归断言用）。
    #[cfg(test)]
    fn overlay_scroll_to_end(&mut self) {
        self.scroll.scroll_to_end();
    }

    /// 测试探针：键盘焦点是否落在浮层内（根句柄或其内部控件持有皆算）。
    #[cfg(test)]
    fn overlay_focused(&self, window: &gpui::Window, cx: &gpui::App) -> bool {
        self.focus.as_ref().is_some_and(|handle| handle.contains_focused(window, cx))
    }
}

impl NebulaWorkspace {
    /// 文件行单击入口（site/semantics/remote 由 rows.rs 解析后传入）：
    /// 同目标在开即无操作；否则进入 Loading、接管键盘焦点并发起获取。
    /// 焦点只在打开时给一次：每次 render 都 focus 会从模式切换钮上抢回
    /// 焦点，浮层内部控件的键盘路径就断了。
    pub(super) fn open_diff_overlay(
        &mut self,
        site: fetch::Site,
        path: String,
        semantics: fetch::Semantics,
        status: char,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 同目标判等必须连来源一起比：路径 + 语义相同但 site/remote 不同
        // （本地 pane 开着、抽屉切到远端视图再点同一路径）时若误判 no-op，
        // 浮层会继续显示上一个仓库的 diff。
        let same = self.vcs_list.diff_overlay.target.as_ref().is_some_and(|t| {
            t.path == path && t.semantics == semantics && t.remote == remote && t.site == site
        });
        if same {
            return;
        }
        let target = OverlayTarget { site, path, semantics, status, remote };
        self.vcs_list.diff_overlay.target = Some(target.clone());
        self.vcs_list.diff_overlay.phase = Phase::Loading;
        self.vcs_list.diff_overlay.generation =
            self.vcs_list.diff_overlay.generation.wrapping_add(1);
        self.vcs_list.diff_overlay.seen_snapshot = self.current_vcs_snapshot();
        self.vcs_list.diff_overlay.tab_fingerprint = (self.active, self.settings_open);
        self.vcs_list.diff_overlay.scroll =
            gpui::ListState::new(0, gpui::ListAlignment::Top, px(22.0));
        let handle =
            self.vcs_list.diff_overlay.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&handle, cx);
        fetch::spawn(self.vcs_list.diff_overlay.generation, &target, cx);
        cx.notify();
    }

    pub(super) fn close_diff_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vcs_list.diff_overlay.target.take().is_some() {
            self.vcs_list.diff_overlay.phase = Phase::Loading;
            // 焦点回活动 tab（设计 §5.3）。focus_active 对无可回落视图的
            // Empty 工作区是 no-op，此时 blur 掉浮层句柄：键盘焦点不能
            // 挂在即将卸载的元素上（否则 Esc/Tab 会派发到幽灵节点）。
            // 有 tab 时 focus_active 的 defer 焦点后于本 blur 生效，不受影响。
            self.focus_active(window, cx);
            if let Some(handle) = self.vcs_list.diff_overlay.focus.as_ref() {
                if handle.contains_focused(window, cx) {
                    window.blur();
                }
            }
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
        if self.vcs_list.diff_overlay.generation != generation
            || self.vcs_list.diff_overlay.target.is_none()
        {
            return;
        }
        self.vcs_list.diff_overlay.phase = match fetched {
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
        // 计数按当时模式的行形态取（loaded_item_count 与模式切换共用）。
        // uniform 高度提示：未测量项按 22px 计，否则滚动条按已测范围钳制。
        let item_count = match &self.vcs_list.diff_overlay.phase {
            Phase::Loaded(loaded) => {
                Some(loaded_item_count(loaded, self.vcs_list.diff_overlay.mode))
            },
            Phase::Raw { lines, .. } => Some(lines.len()),
            _ => None,
        };
        if let Some(count) = item_count {
            self.vcs_list.diff_overlay.scroll.reset_with_uniform_height(count, px(22.0));
        }
        cx.notify();
    }

    /// Error 态重试：同一目标重发获取，generation 递增作废在途响应。
    fn retry_diff_fetch(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.vcs_list.diff_overlay.target.clone() else { return };
        self.vcs_list.diff_overlay.phase = Phase::Loading;
        self.vcs_list.diff_overlay.generation =
            self.vcs_list.diff_overlay.generation.wrapping_add(1);
        fetch::spawn(self.vcs_list.diff_overlay.generation, &target, cx);
        cx.notify();
    }

    /// 布局切换只改视图偏好：不动 generation/phase（两种行形态都在 Loaded
    /// 里）。列表计数随行形态重置——pair 压缩使 pairs ≤ rows 恒成立，若沿
    /// 用旧计数，split→unified 会把尾部 unified 行挡在计数之外永不渲染；
    /// 计数重置连带位置回顶（模式持久即可，滚动位置不跨切换保留）。
    /// 同模式重复点击是 no-op（不重置计数/滚动）。
    fn toggle_diff_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        if mode == self.vcs_list.diff_overlay.mode {
            return;
        }
        self.vcs_list.diff_overlay.mode = mode;
        if let Phase::Loaded(loaded) = &self.vcs_list.diff_overlay.phase {
            let count = loaded_item_count(loaded, mode);
            self.vcs_list.diff_overlay.scroll.reset_with_uniform_height(count, px(22.0));
        }
        cx.notify();
    }

    /// render.rs 挂载点。tab 指纹变化 → 关闭；快照更新 → 文件仍在则重取、
    /// 不在则关。只做状态判定与调度，不同步等待任何 IO。
    /// 可见性放宽到 workspace：模块挂 vcs_panel 下，但挂载点在父层 render.rs。
    pub(in crate::gpui_shell::workspace) fn render_diff_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.vcs_list.diff_overlay.target.is_none() {
            return None;
        }
        let fingerprint = (self.active, self.settings_open);
        if fingerprint != self.vcs_list.diff_overlay.tab_fingerprint {
            self.close_diff_overlay(window, cx);
            return None;
        }
        let snapshot = self.current_vcs_snapshot();
        if let Some(current) = snapshot.as_ref() {
            let changed = self
                .vcs_list
                .diff_overlay
                .seen_snapshot
                .as_ref()
                .map(|seen| !Arc::ptr_eq(seen, current))
                .unwrap_or(true);
            if changed {
                let mut target = self.vcs_list.diff_overlay.target.clone().unwrap();
                match listed_entry(current, &target.path, target.semantics) {
                    None => {
                        self.close_diff_overlay(window, cx);
                        return None;
                    },
                    // 快照更新可能改变文件状态（M→D 等）：头部状态字母随重取
                    // 同步刷新，不再冻结在点击时刻；查找与关闭判定共用同一
                    // 列表语义（listed_entry）。
                    Some((status, _)) => {
                        target.status = *status;
                        self.vcs_list.diff_overlay.target = Some(target.clone());
                    },
                }
                self.vcs_list.diff_overlay.seen_snapshot = snapshot.clone();
                self.vcs_list.diff_overlay.generation =
                    self.vcs_list.diff_overlay.generation.wrapping_add(1);
                self.vcs_list.diff_overlay.phase = Phase::Loading;
                fetch::spawn(self.vcs_list.diff_overlay.generation, &target, cx);
            }
        } else if self.vcs_list.diff_overlay.seen_snapshot.is_some() {
            // 列表来源整体消失（仓库退出/刷新失败）：按「文件已不在列表」的
            // 保守延伸直接关闭，不留指向已消失快照的浮层。
            self.close_diff_overlay(window, cx);
            return None;
        }
        Some(self.render_overlay_body(cx))
    }

    fn current_vcs_snapshot(&self) -> Option<Arc<GitInfo>> {
        match self.vcs_list.diff_overlay.target.as_ref().map(|t| t.remote) {
            Some(true) => self.remote_git.snapshot_for_overlay(),
            _ => self.side_panel.git_snapshot(),
        }
    }

    fn render_overlay_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let language = crate::gpui_shell::config::ui_language(cx);
        let status = self.vcs_list.diff_overlay.target.as_ref().map(|t| t.status);
        let path: SharedString = self
            .vcs_list
            .diff_overlay
            .target
            .as_ref()
            .map(|t| t.path.clone())
            .unwrap_or_default()
            .into();
        let semantics_label =
            self.vcs_list.diff_overlay.target.as_ref().map(|t| match t.semantics {
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
        let handle =
            self.vcs_list.diff_overlay.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        div()
            .id("diff-overlay")
            .debug_selector(|| "diff-overlay".to_string())
            // 键盘焦点锚在浮层根（open_diff_overlay 打开时接管）：Esc 处理
            // 器挂根上，焦点落在内部控件时也沿元素树冒泡到达。
            .track_focus(&handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape") {
                    cx.stop_propagation();
                    this.close_diff_overlay(window, cx);
                }
            }))
            .absolute()
            .inset_0()
            // 阻断浮层之下的鼠标命中（生产路径：终端的 mousedown 会无条件
            // 抢走键盘焦点，Esc 的派发路径随之离开浮层）。不 occlude 时
            // hit_test 不含遮挡语义，底层 hitbox 依然 hovered。
            .occlude()
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
                    // 布局切换（split/unified）：SegmentedRow 滑块复用 vcs_panel
                    // 视图切换的构造模式。无图标——label 常显（label-when-active
                    // 会让未选侧成空钮）。
                    .child(crate::gpui_shell::widgets::segmented::SegmentedRow::new(
                        "diff-mode",
                        match self.vcs_list.diff_overlay.mode {
                            DiffMode::Split => 0,
                            DiffMode::Unified => 1,
                        },
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("diff-mode-split")
                                    .label(language.text(Message::VcsDiffModeSplit))
                                    .debug_selector(|| "diff-mode-split".to_string())
                                    .ghost()
                                    .small()
                                    .toggled(self.vcs_list.diff_overlay.mode == DiffMode::Split)
                                    .tooltip(language.text(Message::VcsDiffModeSplit))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_diff_mode(DiffMode::Split, cx);
                                    })),
                            )
                            .child(
                                Button::new("diff-mode-unified")
                                    .label(language.text(Message::VcsDiffModeUnified))
                                    .debug_selector(|| "diff-mode-unified".to_string())
                                    .ghost()
                                    .small()
                                    .toggled(self.vcs_list.diff_overlay.mode == DiffMode::Unified)
                                    .tooltip(language.text(Message::VcsDiffModeUnified))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_diff_mode(DiffMode::Unified, cx);
                                    })),
                            ),
                        div().rounded(theme.radius).bg(theme.tokens.secondary_active),
                    ))
                    // 关闭按钮（§5.3 关闭路径之一；Esc/切 tab 见根节点与
                    // render_diff_overlay 的指纹判定）。
                    .child(
                        Button::new("diff-close")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .debug_selector(|| "diff-close".to_string())
                            .tooltip(language.text(Message::VcsDiffClose))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_diff_overlay(window, cx);
                            })),
                    ),
            )
            .child(match &self.vcs_list.diff_overlay.phase {
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
                            .debug_selector(|| "diff-retry".to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.retry_diff_fetch(cx);
                            })),
                    )
                    .into_any_element(),
                // Loaded/Raw 共用滚动体；行形在 render_overlay_row 按 mode 分派。
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
        let scroll = self.vcs_list.diff_overlay.scroll.clone();
        let owner = cx.entity().downgrade();
        let muted = cx.theme().muted_foreground;
        v_flex()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .id("diff-overlay-body")
                    .debug_selector(|| "diff-overlay-body".to_string())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        gpui::list(scroll.clone(), move |index, _window, cx| {
                            owner
                                .update(cx, |workspace, cx| workspace.render_overlay_row(index, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        })
                        .size_full(),
                    )
                    // 滑块住绝对定位宿主浮在列表右缘（file_tree 与组件库 Table
                    // 同构——直接当普通 child 会参与布局挤占行高）。ListState
                    // 实现了组件库 ScrollbarHandle，拖动直接驱动同一滚动模型。
                    // Always：diff 长度/位置需要常驻可读（组件在无溢出时自隐藏）。
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .debug_selector(|| "diff-overlay-scrollbar".to_string())
                            .w(px(16.0))
                            .child(
                                gpui_component::scroll::Scrollbar::vertical(&scroll)
                                    .scrollbar_show(
                                        gpui_component::scroll::ScrollbarShow::Always,
                                    ),
                            ),
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

    /// 虚拟列表行：Loaded 按 mode 分派（split → 配对行，unified → 行模型），
    /// 越界槽位（模式切换后计数差异的残留）画空行；Raw → 等宽原文。
    fn render_overlay_row(&mut self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let mode = self.vcs_list.diff_overlay.mode;
        let (row, pair, raw) = match &self.vcs_list.diff_overlay.phase {
            Phase::Loaded(loaded) => match mode {
                DiffMode::Unified => (loaded.rows.get(index), None, None),
                DiffMode::Split => (None, loaded.pairs.get(index), None),
            },
            Phase::Raw { lines, .. } => (None, None, lines.get(index)),
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
        if let Some(pair) = pair {
            return split_row(index, pair, theme);
        }
        let Some(row) = row else { return div().into_any_element() };
        match row {
            pair::DisplayRow::FullWidth { text } => band_row(
                SharedString::from(format!("diff-overlay-row-unified-{index}")),
                text,
                theme,
            ),
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

/// 整行横带（selector 作为元素 id + debug key 传入）：hunk 头高亮，note
/// （"\ No newline..."）按弱化说明处理；unified 的 FullWidth 与 split 的
/// band 共用同一构造。带 hunk 上下文的头可达数百字符：单行截断不撑高。
fn band_row(selector: SharedString, text: &str, theme: &gpui_component::Theme) -> gpui::AnyElement {
    let hunk = text.starts_with("@@");
    h_flex()
        .id(selector.clone())
        .debug_selector(move || selector.to_string())
        .w_full()
        .px_3()
        .h(px(22.0))
        .flex_shrink_0()
        .items_center()
        .when(hunk, |row| row.bg(theme.primary.opacity(0.08)))
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.font_size - px(1.0))
        .text_color(if hunk { theme.primary } else { theme.muted_foreground })
        .child(div().flex_1().min_w_0().truncate().child(text.to_string()))
        .into_any_element()
}

/// split 模式一行：band（hunk 头/注记）→ 整行横带（样式同 unified 的
/// FullWidth）；否则左右两半夹 1px 分隔线。外层 `items_stretch`：行高由
/// 较高一侧驱动，空占位与分隔线随之拉伸——两列恒对齐是 §5.2 单滚动
/// 驱动的前提（pair 已保证两侧行数相等，这里补齐视觉高度）。
fn split_row(
    index: usize,
    pair: &pair::SplitPair,
    theme: &gpui_component::Theme,
) -> gpui::AnyElement {
    if let Some(text) = pair.band.as_deref() {
        return band_row(
            SharedString::from(format!("diff-overlay-row-split-{index}")),
            text,
            theme,
        );
    }
    h_flex()
        .id(SharedString::from(format!("diff-overlay-row-split-{index}")))
        .debug_selector(move || format!("diff-overlay-row-split-{index}"))
        .w_full()
        .items_stretch()
        .child(split_half(index, "left", pair.left.as_ref(), theme))
        .child(div().w(px(1.0)).flex_shrink_0().bg(theme.border))
        .child(split_half(index, "right", pair.right.as_ref(), theme))
        .into_any_element()
}

/// split 单侧半行：行号槽（48px 右对齐弱化）+ 软换行文本；着色按单元格
/// kind（Deleted 红 / Added 绿，底色同透明度；Context 前景色）。`None` 侧
/// 渲染同构空占位（空文本、无底色）——高度零贡献，由外层 stretch 拉齐。
fn split_half(
    index: usize,
    side: &'static str,
    cell: Option<&pair::LineCell>,
    theme: &gpui_component::Theme,
) -> gpui::Stateful<gpui::Div> {
    let (fg, bg) = match cell.map(|cell| cell.kind) {
        Some(crate::native_result_model::diff::Kind::Deleted) => {
            (theme.danger, Some(theme.danger.opacity(0.13)))
        },
        Some(crate::native_result_model::diff::Kind::Added) => {
            (theme.success, Some(theme.success.opacity(0.13)))
        },
        _ => (theme.foreground, None),
    };
    h_flex()
        .id(SharedString::from(format!("diff-overlay-row-split-{index}-{side}")))
        .debug_selector(move || format!("diff-overlay-row-split-{index}-{side}"))
        .flex_1()
        .min_w_0()
        .items_start()
        .when_some(bg, |half, bg| half.bg(bg))
        .child(
            div()
                .w(px(48.0))
                .flex_shrink_0()
                .text_right()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.font_size - px(1.0))
                .text_color(theme.muted_foreground)
                .child(cell.and_then(|cell| cell.no).map(|n| n.to_string()).unwrap_or_default()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.font_size - px(1.0))
                .text_color(fg)
                .line_height(gpui::relative(1.35))
                .child(cell.map(|cell| cell.text.clone()).unwrap_or_default()),
        )
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

/// Loaded 态在指定模式下的列表计数（落地 reset 与模式切换共用同一选择，
/// 保证两条路径的行数判定不会漂移）。
fn loaded_item_count(loaded: &Loaded, mode: DiffMode) -> usize {
    match mode {
        DiffMode::Split => loaded.pairs.len(),
        DiffMode::Unified => loaded.rows.len(),
    }
}

/// 快照更新后目标文件在「与抽屉显示一致」语义下的列表条目（`GitInfo`
/// 在上游，判定放浮层侧）：None = 已不在列表（关闭判定），Some 携带
/// 条目（状态刷新用）。
fn listed_entry<'a>(
    info: &'a GitInfo,
    path: &str,
    semantics: fetch::Semantics,
) -> Option<&'a (char, String)> {
    let list = match semantics {
        fetch::Semantics::Staged => &info.staged,
        fetch::Semantics::ConflictVsHead => &info.conflicts,
        _ => &info.unstaged,
    };
    // 抽屉把冲突路径从 staged/changes 段排除、只在冲突段渲染（list_model
    // 的 exclude_conflicts 语义）；真实数据里冲突路径同时保留在原始列表
    // （GitInfo.conflicts 字段合同）。刚转入冲突的文件在抽屉原段已消失，
    // 若按原始列表判「仍在」会让浮层以旧语义继续重取——点击语义实际已
    // 变成冲突 vs HEAD——与用户所见脱节。这里同步排除，让它落入
    // 「不在列表 → 关闭」分支。
    if !matches!(semantics, fetch::Semantics::ConflictVsHead)
        && info.conflicts.iter().any(|(_, p)| p == path)
    {
        return None;
    }
    list.iter().find(|(_, p)| p == path)
}
