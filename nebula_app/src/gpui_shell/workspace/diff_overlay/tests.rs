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
    let root = tempfile::tempdir().unwrap().path().to_path_buf();
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

#[gpui::test]
fn single_click_opens_overlay_in_loading_state(cx: &mut TestAppContext) {
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
            cx.debug_bounds("diff-overlay-loading").expect("loading shell visible");
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
            cx.update(|_, cx| {
                owner.update(cx, |ws, cx| {
                    ws.open_diff_overlay(
                        fetch::Site::Local { root: PathBuf::from(root) },
                        "f".into(),
                        fetch::Semantics::Unstaged,
                        'M',
                        false,
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
        cx.update(|_, cx| {
            owner.update(cx, |ws, cx| {
                ws.open_diff_overlay(
                    fetch::Site::Local { root: PathBuf::from("/b") },
                    "f".into(),
                    fetch::Semantics::Unstaged,
                    'M',
                    true,
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
            cx.update(|_, cx| {
                owner.update(cx, |ws, cx| {
                    ws.open_diff_overlay(
                        fetch::Site::Local { root: PathBuf::from("/repo") },
                        "src/a.rs".into(),
                        fetch::Semantics::Unstaged,
                        'M',
                        false,
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
