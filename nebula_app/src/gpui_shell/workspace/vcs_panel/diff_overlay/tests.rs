//! diff 浮层交互测试：真实控件路径（单击弹层、关闭、切换、布局）。
use super::*;
use crate::display::side_panel::{GitInfo, GitPanelView, VcsKind};
use gpui::{Modifiers, TestAppContext};
use std::path::PathBuf;
use std::sync::Arc;

/// 探针同时画抽屉行列表与浮层：行列表提供可点击的真实控件，浮层挂载
/// 几何与 render.rs 一致——盖「中间卡」（自己的 relative 容器内
/// absolute inset_0），不遮抽屉行；否则「浮层开着点其他文件」的真实
/// 点击路径在测试里不可达。
struct OverlayProbe(Entity<NebulaWorkspace>);
impl Render for OverlayProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .w(px(640.0))
            .h(px(400.0))
            .flex()
            .child(self.0.update(cx, |ws, cx| ws.render_vcs_list(cx)))
            .child(
                div()
                    .relative()
                    .w(px(320.0))
                    .children(self.0.update(cx, |ws, cx| ws.render_diff_overlay(window, cx))),
            )
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
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
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
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
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
        let first = owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation());
        // 完全相同的目标：无操作（generation 不动）。
        open_local(cx, "/a");
        assert_eq!(
            owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation()),
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
            owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation()),
            first + 1,
            "different site/remote must not be a no-op"
        );
        assert_eq!(
            owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_target_site()),
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
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "open must take effect before the vanish edge"
            );
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| {
                    // 侧栏从未 sync：current_vcs_snapshot() 为 None。伪装
                    // 「打开时基于过一份快照」，制造 Some→None 的消失沿。
                    ws.vcs_list
                        .diff_overlay
                        .force_seen_snapshot(Some(Arc::new(GitInfo::default())));
                });
            });
            cx.update(|window, cx| {
                probe.update(cx, |_, cx| cx.notify());
                let _ = window.draw(cx);
            });
            assert!(
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
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
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
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
                    owner.read(cx).vcs_list.diff_overlay.overlay_focused(window, cx),
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
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
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
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
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
                    owner.read(cx).vcs_list.diff_overlay.overlay_focused(window, cx),
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
                    !owner.read(cx).vcs_list.diff_overlay.overlay_focused(window, cx),
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
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()),
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
                owner.update(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_scroll_to_end());
            });
            redraw(cx);
            cx.debug_bounds("diff-overlay-row-unified-3")
                .expect("last unified row must be reachable after the toggle");
        },
    );
}

/// 远端取数不可用必须落 Error（含重试钮）而不是停在 Loading：远端 Git 视图
/// 的文件行走 Site::Remote，测试环境不发起远端 exec（网络 runtime 侧按
/// 「连接不可用」收 Err），Loading 必须被 Error 取代。Task 10 前远端取数
/// 分支缺失，点击后永远停在 Loading——本测试是该回归的钉子。
#[gpui::test]
fn remote_fetch_unavailable_lands_in_error_phase(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "src/a.rs".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            // 列表切到远端形态（生产由 render_remote_git sync）+ 绑定远端目标
            // （生产由 route_remote_git 在聚焦远端 pane 时设置）：行点击才会
            // 解析出 Site::Remote。
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| {
                    ws.remote_git.test_bind("dest", "/repo");
                    ws.vcs_list.sync(
                        Some(Arc::new(GitInfo {
                            unstaged: vec![('M', "src/a.rs".into())],
                            ..GitInfo::default()
                        })),
                        GitPanelView::Changes,
                        Some(PathBuf::from("/repo")),
                        true,
                    );
                });
            });
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
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "a remote file row must open the overlay"
            );
            assert!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_phase_is_error()),
                "an unavailable remote fetch must land in the error phase, not stay loading"
            );
            redraw(cx);
            cx.debug_bounds("diff-overlay-error")
                .expect("the error body must render for a failed remote fetch");
            cx.debug_bounds("diff-retry").expect("the retry button must be offered");
        },
    );
}

/// generation 作废合同（设计 §6 生命周期）：晚到的成功结果不得覆盖新状态。
/// 打开 A（generation N）再打开 B（N+1），对 N 落一份 Ok(Untracked)：必须
/// 整份丢弃；排空后浮层形态由当前请求（远端不可用 → Error）决定，而不是
/// 被旧结果拉成 Loaded。
#[gpui::test]
fn stale_generation_result_is_dropped(cx: &mut TestAppContext) {
    workspace_with_changes(cx, GitInfo::default(), |owner, _probe, _root, cx| {
        let open = |cx: &mut gpui::VisualTestContext, path: &'static str| {
            cx.update(|window, cx| {
                owner.update(cx, |ws, cx| {
                    ws.open_diff_overlay(
                        fetch::Site::Remote { destination: "dest".into(), root: "/repo".into() },
                        path.into(),
                        fetch::Semantics::Untracked,
                        '?',
                        true,
                        window,
                        cx,
                    );
                });
            });
        };
        open(cx, "a.txt");
        let stale = owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation());
        open(cx, "b.txt");
        assert_eq!(
            owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation()),
            stale + 1,
            "opening another file must advance the generation"
        );
        cx.update(|_, cx| {
            owner.update(cx, |ws, cx| {
                ws.apply_fetch_result(
                    stale,
                    Ok(fetch::Fetched::Untracked {
                        lines: vec!["stale".to_string()],
                        truncated: false,
                    }),
                    cx,
                );
            });
        });
        assert!(
            !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()),
            "a stale-generation success must be dropped wholesale"
        );
        cx.run_until_parked();
        assert!(
            owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_phase_is_error()),
            "after parking, the current generation's outcome must own the phase"
        );
    });
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
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()),
                "a.txt must reach the Loaded phase"
            );
            redraw(cx);
            cx.debug_bounds("diff-overlay-row-split-0").expect("a.txt opens in split by default");
            click_toggle(cx, "diff-mode-unified");
            cx.update(|window, cx| {
                owner.update(cx, |ws, cx| ws.close_diff_overlay(window, cx));
            });
            assert!(
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "close_diff_overlay must close before the second file"
            );
            // long.txt：模式必须仍是 unified。
            redraw(cx);
            click(cx, "git-tree-row-changes-1-long.txt");
            assert!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()),
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

/// 重画探针：render_diff_overlay 的快照/tab 判定在 draw 时同步执行。
fn redraw_overlay(probe: &Entity<OverlayProbe>, cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        probe.update(cx, |_, cx| cx.notify());
        let _ = window.draw(cx);
    });
}

/// 远端快照用例共用编排：绑定远端目标、预置初始快照、列表切远端形态
/// （行点击解析出 Site::Remote），点击文件行打开浮层，再按当前 generation
/// 注入一份成功结果推进 Loaded——远端 exec 在测试里不可用（remote_exec
/// 返回 None 落 Error），stale 测试同款模型级注入。返回打开时的
/// generation。快照驱动的用例必须沿远端源编排：本地快照源是 side_panel
/// 内部状态，测试无法注入。
fn open_remote_overlay_loaded(
    owner: &Entity<NebulaWorkspace>,
    probe: &Entity<OverlayProbe>,
    cx: &mut gpui::VisualTestContext,
    snapshot: Arc<GitInfo>,
    row_selector: &'static str,
) -> u64 {
    cx.update(|_, cx| {
        owner.update(cx, |ws, _| {
            ws.remote_git.test_bind("dest", "/repo");
            ws.remote_git.test_snapshot(Some(snapshot.clone()));
            ws.vcs_list.sync(
                Some(snapshot),
                GitPanelView::Changes,
                Some(PathBuf::from("/repo")),
                true,
            );
        });
    });
    redraw_overlay(probe, cx);
    let bounds =
        cx.debug_bounds(row_selector).unwrap_or_else(|| panic!("{row_selector} must render"));
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    assert!(
        owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
        "the remote row click must open the overlay"
    );
    let generation = owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation());
    cx.update(|_, cx| {
        owner.update(cx, |ws, cx| {
            ws.apply_fetch_result(
                generation,
                Ok(fetch::Fetched::Untracked { lines: vec!["one".to_string()], truncated: false }),
                cx,
            );
        });
    });
    assert!(
        owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()),
        "the injected result must land the overlay in Loaded"
    );
    generation
}

/// 面板开着点其他文件 → 原地换内容（设计 §5.1）：浮层全程不关、
/// generation +1、phase 先回 Loading 再 Loaded、头部路径换成新文件。
#[gpui::test]
fn clicking_another_file_replaces_content_in_place(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo {
            unstaged: vec![('M', "a.txt".into()), ('M', "b.txt".into())],
            ..GitInfo::default()
        },
        |owner, probe, root, cx| {
            init_repo(&root);
            for name in ["a.txt", "b.txt"] {
                std::fs::write(root.join(name), "one\ntwo\n").unwrap();
            }
            run_git(&root, &["add", "a.txt", "b.txt"]);
            for name in ["a.txt", "b.txt"] {
                std::fs::write(root.join(name), "one\nTWO\n").unwrap();
            }
            let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
                let bounds = cx.debug_bounds(selector).expect("row rendered");
                cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
                cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            };
            redraw_overlay(&probe, cx);
            click(cx, "git-tree-row-changes-0-a.txt");
            cx.run_until_parked();
            assert!(owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()));
            let first = owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation());
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_target_path()),
                Some("a.txt".to_string()),
                "sanity: the first click owns the header path"
            );
            // 点 b 行：浮层保持打开，generation 递增；phase 经 Loading 再回
            // Loaded——本地取数在确定性调度器上随事件派发同步落地（见
            // single_click 测试同注），Loading 窗口不可观测，以排空后的
            // Loaded + 新 generation 断言换内容成立。
            click(cx, "git-tree-row-changes-1-b.txt");
            assert!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "switching files must keep the overlay open"
            );
            cx.run_until_parked();
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation()),
                first + 1,
                "switching files advances the generation once"
            );
            assert!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_loaded()),
                "b.txt must reach the Loaded phase"
            );
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_target_path()),
                Some("b.txt".to_string()),
                "the header path must follow the newly clicked file"
            );
        },
    );
}

/// 快照驱动的自动刷新（设计 §6）：快照更新且文件仍在 → 自动重取
/// （generation +1、离开 Loaded）；文件已不在列表 → 下一次 render 自动
/// 关闭。快照源沿远端路径编排（见 open_remote_overlay_loaded）。
#[gpui::test]
fn snapshot_refresh_refetches_and_disappearing_file_closes(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "src/a.rs".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            let opened = Arc::new(GitInfo {
                unstaged: vec![('M', "src/a.rs".into())],
                ..GitInfo::default()
            });
            let loaded_gen = open_remote_overlay_loaded(
                &owner,
                &probe,
                cx,
                opened,
                "git-tree-row-changes-0-src/a.rs",
            );
            // (i) 快照更新（新 Arc）且文件仍在：必须自动重取——generation +1。
            let still_listed = Arc::new(GitInfo {
                unstaged: vec![('M', "src/a.rs".into())],
                ..GitInfo::default()
            });
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| ws.remote_git.test_snapshot(Some(still_listed)));
            });
            redraw_overlay(&probe, cx);
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation()),
                loaded_gen + 1,
                "a changed snapshot with the file still listed must refetch"
            );
            assert!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "a still-listed file must keep the overlay open"
            );
            cx.run_until_parked();
            assert!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_phase_is_error()),
                "the refetch must own the phase (remote exec is unavailable in tests)"
            );
            // (ii) 文件从列表消失：下一次 render 必须自动关闭。
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| {
                    ws.remote_git.test_snapshot(Some(Arc::new(GitInfo::default())));
                });
            });
            redraw_overlay(&probe, cx);
            assert!(
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "a snapshot without the file must close the overlay"
            );
        },
    );
}

/// 快照重取必须同步头部状态字母（缺陷 B）：点击时 'M'、下一份快照变
/// 'D'（M→D），头部不能冻结在点击时刻；generation 语义不变（重取 +1）。
#[gpui::test]
fn snapshot_refresh_updates_header_status_letter(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "a.txt".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            let opened =
                Arc::new(GitInfo { unstaged: vec![('M', "a.txt".into())], ..GitInfo::default() });
            let loaded_gen = open_remote_overlay_loaded(
                &owner,
                &probe,
                cx,
                opened,
                "git-tree-row-changes-0-a.txt",
            );
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_target_status()),
                Some('M'),
                "sanity: the status letter starts at the click-time value"
            );
            let deleted =
                Arc::new(GitInfo { unstaged: vec![('D', "a.txt".into())], ..GitInfo::default() });
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| ws.remote_git.test_snapshot(Some(deleted)));
            });
            redraw_overlay(&probe, cx);
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_generation()),
                loaded_gen + 1,
                "the status refresh rides the refetch; generation still advances once"
            );
            assert_eq!(
                owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.overlay_target_status()),
                Some('D'),
                "the header status letter must follow the new snapshot, not the click"
            );
        },
    );
}

/// 文件转入冲突 → 关闭浮层（缺陷 A）：抽屉把冲突路径从 staged/changes 段
/// 排除、只在冲突段渲染（list_model 的 exclude_conflicts）；真实数据里
/// 冲突路径同时保留在 unstaged 原始列表（GitInfo.conflicts 字段合同）。
/// 按原始列表判「仍在」会让浮层以旧语义继续重取，与抽屉所见脱节——判定
/// 必须与抽屉显示一致：转入冲突即视为不在列表，关闭。
#[gpui::test]
fn file_turning_conflicted_closes_overlay(cx: &mut TestAppContext) {
    workspace_with_changes(
        cx,
        GitInfo { unstaged: vec![('M', "a.txt".into())], ..GitInfo::default() },
        |owner, probe, _root, cx| {
            let opened =
                Arc::new(GitInfo { unstaged: vec![('M', "a.txt".into())], ..GitInfo::default() });
            open_remote_overlay_loaded(&owner, &probe, cx, opened, "git-tree-row-changes-0-a.txt");
            // a.txt 同时出现在 unstaged 原始列表与 conflicts：抽屉的 changes
            // 段已不再显示它（转入冲突段）——浮层必须按「不在列表」关闭。
            let conflicted = Arc::new(GitInfo {
                unstaged: vec![('M', "a.txt".into())],
                conflicts: vec![('U', "a.txt".into())],
                ..GitInfo::default()
            });
            cx.update(|_, cx| {
                owner.update(cx, |ws, _| ws.remote_git.test_snapshot(Some(conflicted)));
            });
            redraw_overlay(&probe, cx);
            assert!(
                !owner.read_with(cx, |ws, _| ws.vcs_list.diff_overlay.is_open()),
                "a file that moved into the conflicts section must close the overlay"
            );
        },
    );
}
