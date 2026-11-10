//! diff 浮层交互测试：真实控件路径（单击弹层、关闭、切换、布局）。
use super::*;
use crate::display::side_panel::{GitInfo, GitPanelView, VcsKind};
use gpui::{Modifiers, TestAppContext};
use std::path::PathBuf;
use std::sync::Arc;

/// 探针同时画抽屉行列表与浮层：行列表提供可点击的真实控件，浮层
/// 挂载顺序与 render.rs 一致（列表之上、后画者盖前）。
struct OverlayProbe(Entity<NebulaWorkspace>);
impl Render for OverlayProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .w(px(640.0))
            .h(px(400.0))
            .flex()
            .child(self.0.update(cx, |ws, cx| ws.render_vcs_list(cx)))
            .children(self.0.update(cx, |ws, cx| ws.render_diff_overlay(window, cx)))
    }
}

/// 逐行照抄 list_tests / tree_rows_tests 的 harness：同一初始化序列 +
/// 空启动 workspace，抽屉列表用传入快照驱动，建窗后进入 `body`。
/// `add_window_view` 借出的 `VisualTestContext` 不能逃逸出本函数，所以
/// 断言体以闭包传入；`root` 供选中路径断言比对。
fn workspace_with_changes(
    cx: &mut TestAppContext,
    info: GitInfo,
    body: impl FnOnce(
        Entity<NebulaWorkspace>,
        Entity<OverlayProbe>,
        PathBuf,
        &mut gpui::VisualTestContext,
    ),
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::gpui_shell::math_view::register(cx);
        crate::gpui_shell::file_editor::init(cx);
        crate::gpui_shell::init(cx, None);
        cx.set_reduce_motion(true);
        windowing::initialize(cx, crate::runtime_api::RuntimeHub::new());
    });
    let data = Arc::new(info);
    // TempDir 守卫活到 body 之后：真实仓库测试要在 root 里落一个 git 仓库。
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let mut owner = None;
    let mut probe = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| {
            NebulaWorkspace::new(
                window,
                None,
                None,
                1,
                crate::runtime_api::RuntimeHub::new(),
                windowing::WorkspaceStartup::Empty,
                windowing::WindowRole::Regular,
                cx,
            )
        });
        workspace.update(cx, |workspace, _| {
            workspace.vcs_list.sync(
                Some(data.clone()),
                GitPanelView::Changes,
                Some(root.clone()),
                false,
            );
        });
        let view = cx.new(|_| OverlayProbe(workspace.clone()));
        owner = Some(workspace);
        probe = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    body(owner.unwrap(), probe.unwrap(), root, cx);
}

/// 真实仓库 e2e 共用：跑真 git 子进程并断言退出码。
fn run_git(root: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git").args(args).current_dir(root).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// 真实仓库 e2e 共用：最小 init + 提交身份配置。
fn init_repo(root: &std::path::Path) {
    run_git(root, &["init", "-q"]);
    run_git(root, &["config", "user.email", "t@t"]);
    run_git(root, &["config", "user.name", "t"]);
}

/// 单击打开浮层。本地取数在确定性调度器上随事件派发同步落地（Task 7 起
/// fetch::spawn 真发命令），loading 窗口不再可观测；非仓库根必然落到
/// Error 态，据此断言「打开 + 取数必然离开放 loading」。成功落地的
/// 端到端路径由 overlay_renders_split_and_toggles_to_unified 覆盖。
#[gpui::test]
fn single_click_opens_overlay_and_local_fetch_resolves(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "src/a.rs".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            let redraw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| {
                    probe.update(cx, |_, cx| cx.notify());
                    let _ = window.draw(cx);
                });
            };
            redraw(cx);
            let bounds =
                cx.debug_bounds("git-tree-row-changes-0-src/a.rs").expect("file row rendered");
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            assert!(
                owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "single click on a file row must open the diff overlay"
            );
            redraw(cx);
            cx.debug_bounds("diff-overlay").expect("overlay mounted");
            assert!(
                cx.debug_bounds("diff-overlay-loading").is_none(),
                "local fetch resolves inline; the loading shell must be gone"
            );
            cx.debug_bounds("diff-overlay-error")
                .expect("non-repo root lands the overlay in the error body with retry");
        },
    );
}

/// SVN 行没有 git diff 语义：单击只选中、不弹浮层（回归：浮层门控在
/// `is_git` 上，SVN 平铺列表行为不变）。
#[gpui::test]
fn svn_click_keeps_selection_without_overlay(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { vcs: VcsKind::Svn, unstaged: vec![('M', "a.txt".into())], ..GitInfo::default() },
        |owner, probe, root, cx| {
            cx.update(|window, cx| {
                probe.update(cx, |_, cx| cx.notify());
                let _ = window.draw(cx);
            });
            let bounds = cx.debug_bounds("git-tree-row-svn-0-a.txt").expect("svn row rendered");
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            assert!(
                !owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "svn rows must not open the git diff overlay"
            );
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.side_panel.selected.clone()),
                Some(root.join("a.txt")),
                "svn click still selects the row"
            );
        },
    );
}

/// 同路径同语义但来源不同（site/remote）不得判 no-op——否则本地浮层开着、
/// 抽屉切到远端视图点同一路径时，会继续显示上一个仓库的 diff。
#[gpui::test]
fn same_path_different_site_is_not_a_noop(cx: &mut TestAppContext) {
    workspace_with_changes(cx, GitInfo::default(), |owner, _probe, _root, cx| {
        let open_local = |cx: &mut gpui::VisualTestContext, root: &str| {
            cx.update(|window, cx| {
                owner.update(cx, |ws, cx| {
                    ws.open_diff_overlay(
                        fetch::Site::Local { root: PathBuf::from(root) },
                        "f".into(),
                        fetch::Semantics::Unstaged,
                        'M',
                        false,
                        window,
                        cx,
                    );
                });
            });
        };
        open_local(cx, "/a");
        let first = owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_generation());
        // 完全相同的目标：无操作（generation 不动）。
        open_local(cx, "/a");
        assert_eq!(
            owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_generation()),
            first,
            "identical target is a no-op"
        );
        // 同路径换来源 + remote 标记：必须重开并指向新 site。
        cx.update(|window, cx| {
            owner.update(cx, |ws, cx| {
                ws.open_diff_overlay(
                    fetch::Site::Local { root: PathBuf::from("/b") },
                    "f".into(),
                    fetch::Semantics::Unstaged,
                    'M',
                    true,
                    window,
                    cx,
                );
            });
        });
        assert_eq!(
            owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_generation()),
            first + 1,
            "different site/remote must not be a no-op"
        );
        assert_eq!(
            owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_target_site()),
            Some(fetch::Site::Local { root: PathBuf::from("/b") })
        );
    });
}

/// 打开时基于过快照、现在列表来源整体消失（Some→None，仓库退出/刷新
/// 失败）：浮层必须自关，不指向已消失的快照。
#[gpui::test]
fn snapshot_vanishing_closes_the_overlay(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "src/a.rs".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            cx.update(|window, cx| {
                owner.update(cx, |ws, cx| {
                    ws.open_diff_overlay(
                        fetch::Site::Local { root: PathBuf::from("/repo") },
                        "src/a.rs".into(),
                        fetch::Semantics::Unstaged,
                        'M',
                        false,
                        window,
                        cx,
                    );
                });
            });
            assert!(
                owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "open must take effect before the vanish edge"
            );
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| {
                    // 侧栏从未 sync：current_vcs_snapshot() 为 None。伪装
                    // 「打开时基于过一份快照」，制造 Some→None 的消失沿。
                    ws.diff_overlay.force_seen_snapshot(Some(Arc::new(GitInfo::default())));
                });
            });
            cx.update(|window, cx| {
                probe.update(cx, |_, cx| cx.notify());
                let _ = window.draw(cx);
            });
            assert!(
                !owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "snapshot source vanished; overlay must close"
            );
        },
    );
}

/// 三条关闭路径逐一验证（设计 §5.3 决策表）：✕ 按钮、Esc、tab 指纹变化。
/// 每条路径独立从「已打开 + 已挂载」出发，避免前一条的关闭副作用喂给下一条。
#[gpui::test]
fn overlay_closes_via_button_escape_and_context_change(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "src/a.rs".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            let redraw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| {
                    probe.update(cx, |_, cx| cx.notify());
                    let _ = window.draw(cx);
                });
            };
            let open = |cx: &mut gpui::VisualTestContext| {
                let bounds =
                    cx.debug_bounds("git-tree-row-changes-0-src/a.rs").expect("file row rendered");
                cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.run_until_parked();
                redraw(cx);
                cx.debug_bounds("diff-overlay").expect("overlay mounted");
            };
            // (a) ✕ 按钮：占位符换成真 Button 后，点击必须关闭并卸载浮层。
            redraw(cx);
            open(cx);
            let close = cx.debug_bounds("diff-close").expect("close button rendered");
            cx.simulate_mouse_down(close.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(close.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            assert!(
                !owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "the close button must close the overlay"
            );
            redraw(cx);
            assert!(
                cx.debug_bounds("diff-overlay").is_none(),
                "overlay must unmount after the close button"
            );
            // (b) Esc：浮层持有焦点时按下 Escape 必须关闭（含 stop_propagation
            // 不外漏——若外漏到全局处理器也不会改变 is_open 断言的判定对象）。
            open(cx);
            cx.update(|window, cx| {
                assert!(
                    owner.read(cx).diff_overlay.overlay_focused(window, cx),
                    "an open overlay must hold keyboard focus"
                );
            });
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: gpui::Keystroke { key: "escape".into(), ..Default::default() },
                is_held: false,
                prefer_character_input: false,
            });
            cx.run_until_parked();
            assert!(
                !owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "escape must close the overlay"
            );
            // (c) tab 指纹：settings_open 翻转（等价于点任务栏切走的机制），
            // 下一次 render 必须自动关闭（Task 5 行为的回归钉）。
            open(cx);
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| ws.settings_open = true);
            });
            redraw(cx);
            assert!(
                !owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "a tab fingerprint change must close the overlay"
            );
        },
    );
}

/// 焦点合同（设计 §5.3）：打开即接管键盘焦点（Esc 可达），关闭即归还——
/// Empty 工作区没有可回落的视图，只断言浮层不再持有焦点。
#[gpui::test]
fn overlay_takes_and_releases_focus(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "src/a.rs".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            let redraw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| {
                    probe.update(cx, |_, cx| cx.notify());
                    let _ = window.draw(cx);
                });
            };
            redraw(cx);
            let bounds =
                cx.debug_bounds("git-tree-row-changes-0-src/a.rs").expect("file row rendered");
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            redraw(cx);
            cx.update(|window, cx| {
                assert!(
                    owner.read(cx).diff_overlay.overlay_focused(window, cx),
                    "opening the overlay must move keyboard focus into it"
                );
            });
            let close = cx.debug_bounds("diff-close").expect("close button rendered");
            cx.simulate_mouse_down(close.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(close.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            redraw(cx);
            cx.update(|window, cx| {
                assert!(
                    !owner.read(cx).diff_overlay.overlay_focused(window, cx),
                    "closing the overlay must release its focus"
                );
            });
        },
    );
}

/// 真实仓库端到端：点击文件行 → background_spawn 上跑真 git → Loaded。
/// 默认 split：split 行渲染、unified 行不渲染；点头部「统一」钮后反转。
/// 覆盖取数、generation 落地、默认模式与切换四层，而不是只测状态机。
#[gpui::test]
fn overlay_renders_split_and_toggles_to_unified(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "a.txt".into())], ..GitInfo::default() },
        |owner, probe, root, cx| {
            init_repo(&root);
            std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
            run_git(&root, &["add", "a.txt"]);
            std::fs::write(root.join("a.txt"), "one\nTWO\n").unwrap();

            let redraw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| {
                    probe.update(cx, |_, cx| cx.notify());
                    let _ = window.draw(cx);
                });
            };
            redraw(cx);
            let bounds =
                cx.debug_bounds("git-tree-row-changes-0-a.txt").expect("file row rendered");
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            // 排空后台取数（真实 git 子进程在 TestScheduler 上同步跑完）。
            cx.run_until_parked();
            assert!(
                owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_loaded()),
                "fetch result must land the overlay in the Loaded phase"
            );
            redraw(cx);
            cx.debug_bounds("diff-overlay-body").expect("scrolled body mounted");
            cx.debug_bounds("diff-overlay-row-split-0").expect("split is the default mode");
            assert!(
                cx.debug_bounds("diff-overlay-row-unified-0").is_none(),
                "unified rows must not render while the mode is split"
            );
            assert!(
                cx.debug_bounds("diff-overlay-loading").is_none(),
                "loading shell must be gone once loaded"
            );
            // 头部「统一」钮（真实 Button 点击路径）：行形必须反转。
            let toggle = cx.debug_bounds("diff-mode-unified").expect("mode toggle rendered");
            cx.simulate_mouse_down(toggle.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(toggle.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            redraw(cx);
            cx.debug_bounds("diff-overlay-row-unified-0").expect("unified rows after toggle");
            assert!(
                cx.debug_bounds("diff-overlay-row-split-0").is_none(),
                "split rows must not render once toggled to unified"
            );
            // 回归：split→unified 必须重置列表计数（本夹具 pairs=3 < rows=4：
            // hunk 带 + context 对 + 删/增对 vs 四条 unified 行），否则末行
            // （+TWO，index 3）落在计数之外永远不渲染。滚到末尾验证可达。
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| ws.diff_overlay.overlay_scroll_to_end());
            });
            redraw(cx);
            cx.debug_bounds("diff-overlay-row-unified-3")
                .expect("last unified row must be reachable after the toggle");
        },
    );
}

/// 模式是浮层持久偏好（设计 §5.1）：A 文件切到 unified 后关闭浮层，再点
/// B 文件仍以 unified 打开；再切回 split 也生效。untracked 单行超宽文件
/// 同时充当 split 单侧内容的等高验证：右列软换行成多行时，左列空占位
/// 随较高一侧拉伸（§5.2 单滚动驱动的前提是两列恒等高）。
#[gpui::test]
fn overlay_mode_persists_across_files(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo {
            unstaged: vec![('M', "a.txt".into()), ('?', "long.txt".into())],
            ..GitInfo::default()
        },
        |owner, probe, root, cx| {
            init_repo(&root);
            std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
            run_git(&root, &["add", "a.txt"]);
            std::fs::write(root.join("a.txt"), "one\nTWO\n").unwrap();
            let long_line = "x".repeat(180);
            std::fs::write(root.join("long.txt"), format!("{long_line}\n")).unwrap();

            let redraw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| {
                    probe.update(cx, |_, cx| cx.notify());
                    let _ = window.draw(cx);
                });
            };
            let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
                let bounds = cx.debug_bounds(selector).expect("row rendered");
                cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.run_until_parked();
            };
            let click_toggle = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
                let bounds = cx.debug_bounds(selector).expect("mode toggle rendered");
                cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.run_until_parked();
            };
            // a.txt：默认 split 打开，切到 unified，然后关闭浮层。
            redraw(cx);
            click(cx, "git-tree-row-changes-0-a.txt");
            assert!(
                owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_loaded()),
                "a.txt must reach the Loaded phase"
            );
            redraw(cx);
            cx.debug_bounds("diff-overlay-row-split-0").expect("a.txt opens in split by default");
            click_toggle(cx, "diff-mode-unified");
            cx.update(|window, cx| {
                owner.update(cx, |ws, cx| ws.close_diff_overlay(window, cx));
            });
            assert!(
                !owner.read_with(cx, |ws, _| ws.diff_overlay.is_open()),
                "close_diff_overlay must close before the second file"
            );
            // long.txt：模式必须仍是 unified。
            redraw(cx);
            click(cx, "git-tree-row-changes-1-long.txt");
            assert!(
                owner.read_with(cx, |ws, _| ws.diff_overlay.overlay_loaded()),
                "long.txt must reach the Loaded phase"
            );
            redraw(cx);
            cx.debug_bounds("diff-overlay-row-unified-0")
                .expect("mode persists across files: long.txt opens unified");
            // 切回 split：untracked 单侧行渲染，且左右两半等高。
            click_toggle(cx, "diff-mode-split");
            redraw(cx);
            let right =
                cx.debug_bounds("diff-overlay-row-split-0-right").expect("right half rendered");
            let left = cx
                .debug_bounds("diff-overlay-row-split-0-left")
                .expect("left placeholder rendered");
            assert!(
                right.size.height > px(22.0),
                "180-char line must soft-wrap in the ~270px half, otherwise the \
                 equal-height check below is vacuous"
            );
            assert_eq!(
                left.size.height, right.size.height,
                "one-sided row: the empty half must stretch to the wrapped sibling's height"
            );
        },
    );
}
