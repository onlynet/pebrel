//! SSH 主机分组的键派生与主机头渲染。
//!
//! SSH pane（`TerminalLaunch::Ssh`）在侧栏呈两级：主机头（`ssh:{dest}`）之下
//! 按远端项目/目录分子组——与本地分组同一语义（仓库根优先，否则 cwd，兜底
//! 「工作区」）。远端仓库根由远端集成经 `SetUserVar=pebrel_git_root` 直接上报
//! （见 `res/shell/bashrc`/`zshrc` 的 precmd），本地**不**做远端路径探测，所以
//! 远端组键不进 [`NebulaWorkspace::project_group_cache`]——远端 `/home/x` 与
//! 本地 `/home/x` 是两个键空间，混进同一张缓存会串组。
//!
//! 键只在构造侧拼接（`ssh:{dest}|{path}`），消费侧整体比较、永不反解；`|` 出现在
//! 远端路径里会先破坏 `NEBULA|` 标题协议，属既有约束。
//!
//! 上报只在外层集成 shell 的提示符发生：用户切进没有集成的嵌套 shell（fish、
//! MSYS bash…）后，`pebrel_git_root`/`NEBULA|` 标题静默，而这类 shell 的 OSC 7
//! 照发（见 view.rs 的 10-10 兜底注释），cwd 仍在走。已上报的根必须过
//! [`git_root_covers`] 一致性校验：cwd 移出该根即视为陈旧，回落到 cwd 语义。

use super::task_groups::TabGroupInfo;
use super::*;
use std::time::Instant;

/// 主机键前缀，与 `recent:` 前缀同一约定：与侧栏折叠表
/// （`sidebar_groups`）共用键空间，前缀隔离后互不干扰。
pub(super) const HOST_KEY_PREFIX: &str = "ssh:";
/// 主机键与子组路径段的分隔符（只拼不解）。
const KEY_PATH_SEPARATOR: char = '|';

/// 主机分组键：`ssh:{destination}`。
pub(super) fn host_key(destination: &str) -> SharedString {
    format!("{HOST_KEY_PREFIX}{destination}").into()
}

/// 远端兜底子组键：cwd 未知（无集成/未上报）时的唯一落点，渲染层把
/// [`super::task_groups::WORKSPACE_GROUP_KEY`] 哨兵名翻译成「工作区」。
pub(super) fn fallback_child_key(destination: &str) -> SharedString {
    format!(
        "{HOST_KEY_PREFIX}{destination}{KEY_PATH_SEPARATOR}{}",
        super::task_groups::WORKSPACE_GROUP_KEY
    )
    .into()
}

/// 上报的仓库根是否仍覆盖当前 cwd。
///
/// `pebrel_git_root` 只在进出仓库的变化沿发送，`NEBULA|` 标题与它同源于外层
/// bash/zsh 的 precmd。用户切进没有集成的嵌套 shell 后这两条信道静默，最后
/// 一次上报会一直留在 pane 上；此时 cwd（OSC 7）若已移出该根，根就是陈旧
/// 信号——分组与远端 Git 面板都不得再信它。**不做本地清除**：变化沿协议靠
/// shell 与 pane 各存一份上次值，清了 view 侧，退出嵌套 shell 回到同一仓库
/// 时外层不会重发，反而把正确的根也丢了。
pub(super) fn git_root_covers(root: &str, cwd: &str) -> bool {
    let root = root.trim();
    if !root.starts_with('/') {
        return false;
    }
    let cwd = cwd.trim();
    cwd.is_empty() || cwd == root || cwd.starts_with(&format!("{root}/"))
}

/// 远端项目子组键：git 根优先（远端集成上报，且必须仍覆盖 cwd），否则可组
/// 的远端 cwd，兜底 [`fallback_child_key`]——与本地 `tab_group_info` 的回落序
/// 完全一致。
pub(super) fn child_key(destination: &str, git_root: &str, cwd: &str) -> SharedString {
    if git_root_covers(git_root, cwd) {
        return format!("{HOST_KEY_PREFIX}{destination}{KEY_PATH_SEPARATOR}{}", git_root.trim())
            .into();
    }
    if super::task_groups::groupable_cwd(cwd) {
        return format!("{HOST_KEY_PREFIX}{destination}{KEY_PATH_SEPARATOR}{}", cwd.trim()).into();
    }
    fallback_child_key(destination)
}

/// 子组显示名：路径末级目录名（`/` 这类无末级的回落全串）；兜底子组保持
/// 哨兵名，由渲染层翻译。
fn child_name(destination: &str, key: &str) -> SharedString {
    let prefix = format!("{HOST_KEY_PREFIX}{destination}{KEY_PATH_SEPARATOR}");
    let path = key.strip_prefix(&prefix).unwrap_or(key);
    if path == super::task_groups::WORKSPACE_GROUP_KEY {
        return super::task_groups::WORKSPACE_GROUP_KEY.into();
    }
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
        .into()
}

/// SSH 组头分支 chip 的「最近上报」状态。
///
/// fish/AI agent pane 里 `NEBULA|` 标题静默后 `view.branch` 冻结在旧支且
/// 非空，首见 tab 的旧聚合（仅 `None` 补位）会把组头钉死在冻结值上——
/// chip 改随「该组最后说话的人」：TitleChanged 沿把新上报写进
/// [`Self::branches`]（见 [`NebulaWorkspace::record_ssh_group_branch`]），
/// render 路径按组键就地读取。pane 全部离开后条目残留：组重建首帧显示
/// 上次上报值，同仓库大概率仍对，不值得为清理性挂 pane 生命周期。
#[derive(Default)]
pub(super) struct SshGroupHeads {
    /// 组键 →（最近采信的分支，采信时刻）。两条写入源——`NEBULA|` 上报
    /// 沿（[`NebulaWorkspace::record_ssh_group_branch`]）与组级探测落地
    /// （[`Self::apply_probe_head`]，见 `probe_ssh_group_head`）——按时刻
    /// 仲裁：迟到的旧探测不得覆盖更晚的上报。
    pub(super) branches: HashMap<SharedString, (SharedString, Instant)>,
    /// 组键 → 上次探测的发出时刻（`probe_ssh_group_head` 的节流窗判据）。
    pub(super) probed_at: HashMap<SharedString, Instant>,
    /// 探测在途的组键（慢网络下防同组重入；完成回调移除）。
    pub(super) probing: std::collections::HashSet<SharedString>,
    /// pane → 上一次被采信的上报值。OSC 7 只报 cwd 也发 TitleChanged，
    /// 靠它把「冻结 pane 的旧支」挡在 LWW 外，防组头闪回。pane id 单调
    /// 不复用，条目随 pane 数线性增长且量级无害。
    pub(super) seen: HashMap<u64, String>,
}

impl SshGroupHeads {
    /// 组级探测的落地（带仲裁）：`issued_at` 是探测**发出**时刻，晚于现存
    /// 记录才可覆盖——并发场景里刚到的上报更新鲜，迟到的探测不得回退它；
    /// 不可信值（空/路径形态，同 [`plausible_branch`]）一律不落地。
    pub(super) fn apply_probe_head(
        &mut self,
        key: &SharedString,
        branch: &str,
        issued_at: Instant,
    ) {
        let Some(branch) = plausible_branch(branch) else {
            return;
        };
        if self.branches.get(key).is_some_and(|(_, at)| *at >= issued_at) {
            return;
        }
        self.branches.insert(key.clone(), (branch.to_owned().into(), issued_at));
    }
}

/// 分支值的可信形态：非空且不是绝对路径（塞错槽的根宁可不信）。
fn plausible_branch(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty() && !trimmed.starts_with('/')).then_some(trimmed)
}

/// 组头的分支 chip：集成在线时随「该组最近一次上报」（`latest_reported`，
/// 见 [`NebulaWorkspace::record_ssh_group_branch`] 的 LWW 表）走，缺席或
/// 不可信时回落本 pane 的 `NEBULA|` 标题值；根陈旧（fish 等无集成嵌套
/// shell，标题静默且 cwd 已移出根）时以探测结果为准——latest 不与探测
/// 竞争，失守组键下的旧上报不得顶掉主动 rev-parse 的新事实；空根 + 有
/// 分支 = 裸仓库形态（集成单行输出），保持展示。**分支槽里出现绝对路径
/// 形态一律不信**——上游（标题协议/远端 git wrapper）把根塞进分支槽时，
/// 宁可空着也不错挂。
fn group_branch(
    branch: &str,
    git_root: &str,
    cwd: &str,
    project: Option<&super::remote_git::RemoteProject>,
    latest_reported: Option<&str>,
) -> Option<SharedString> {
    if git_root_covers(git_root, cwd) {
        return latest_reported
            .and_then(plausible_branch)
            .or_else(|| plausible_branch(branch))
            .map(SharedString::from);
    }
    if let Some(project) = project {
        return plausible_branch(&project.branch).map(SharedString::from);
    }
    (git_root.trim().is_empty() && plausible_branch(branch).is_some())
        .then(|| SharedString::from(branch.trim()))
}

/// 组键的根来源：集成上报的根（须仍覆盖 cwd）→ 探测根（同 pane、同主机、
/// 且仍覆盖 cwd）→ `None`（交回 cwd/兜底回落）。
fn group_root<'a>(
    destination: &str,
    reported: &'a str,
    cwd: &str,
    project: Option<&'a super::remote_git::RemoteProject>,
) -> Option<&'a str> {
    if git_root_covers(reported, cwd) {
        return Some(reported);
    }
    project
        .filter(|project| project.destination == destination && git_root_covers(&project.root, cwd))
        .map(|project| project.root.as_str())
}

/// 单个 SSH pane 的分组归属（render 纯内存；事件路径打点共用同一函数防漂移）。
/// `project` 是按 pane 缓存的远端项目身份（`rev-parse` 探测，见
/// `remote_git` 模块的 `refresh_remote_project`）：集成上报的根陈旧或缺失时
/// 的补位源，与分支 chip、Git 面板根三处同源。`group_branches` 是组键 →
/// 最近上报分支的 LWW 表：组键在本函数内派生，chip 就地按键读取，消费方
/// 无须重算键。
pub(super) fn ssh_group_info(
    destination: &str,
    view: &TerminalView,
    project: Option<&super::remote_git::RemoteProject>,
    group_branches: &HashMap<SharedString, (SharedString, Instant)>,
) -> TabGroupInfo {
    let cwd = view.cwd.as_str();
    let reported = view.git_root.as_str();
    let root = group_root(destination, reported, cwd, project).unwrap_or("");
    let key = child_key(destination, root, cwd);
    let name = child_name(destination, key.as_ref());
    let branch = group_branch(
        view.branch.as_str(),
        reported,
        cwd,
        project.filter(|project| project.destination == destination),
        group_branches.get(&key).map(|(branch, _)| branch.as_str()),
    );
    TabGroupInfo { host: Some(destination.into()), key, name, branch }
}

impl NebulaWorkspace {
    /// TitleChanged 沿上的 SSH 组分支 LWW 记录：`NEBULA|` 标题带来分支时，
    /// 把「组键 → 该分支」写进 [`Self::ssh_group_heads`]，组头 chip 由
    /// render 路径按组键就地读取。只在「上报根仍覆盖 cwd」的组里采信——
    /// 根失守的组键由 cwd 派生，chip 交给探测补位（[`group_branch`] 的
    /// 回落序），LWW 不与探测竞争。per-pane 去重防「OSC 7 只报 cwd 也发
    /// TitleChanged」的回流：冻结 pane 的旧支不得借此把组头拉回去。
    pub(super) fn record_ssh_group_branch(&mut self, view: &gpui::Entity<TerminalView>, cx: &App) {
        let view = view.read(cx);
        let Some(destination) = view.ssh_destination.as_deref() else {
            return;
        };
        let branch = view.branch.trim();
        if branch.is_empty()
            || branch.starts_with('/')
            || !git_root_covers(view.git_root.as_str(), view.cwd.as_str())
            || self.ssh_group_heads.seen.get(&view.pane_id).is_some_and(|seen| seen == branch)
        {
            return;
        }
        let info = ssh_group_info(
            destination,
            view,
            self.remote_git.project(view.pane_id),
            &self.ssh_group_heads.branches,
        );
        self.ssh_group_heads.seen.insert(view.pane_id, branch.to_owned());
        self.ssh_group_heads.branches.insert(info.key, (branch.into(), Instant::now()));
    }

    /// SSH 主机头行：chevron + 服务器图标 + 「SSH {目的地}」名 + 计数。与
    /// 项目组头同一解剖（`render_group_header`），差异只有三处表达层级：服务
    /// 器图标、名字走 `ssh_host` 词典词条（不加 `[ ]` 括号）、无分支 chip。
    /// 折叠键为主机键，与子组键共用 [`NebulaWorkspace::sidebar_groups`]。
    pub(super) fn render_ssh_host_header(
        &self,
        row_w: f32,
        row_ix: usize,
        key: &SharedString,
        name: &SharedString,
        tabs: usize,
        collapsed: bool,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let hover_bg = theme.list_hover;
        let faint = crate::gpui_shell::theme::faint_ink(cx);
        let settings = cx.try_global::<crate::gpui_shell::config::Settings>();
        let label_px = settings.map(|settings| settings.ui_font_size_px).unwrap_or(15.0);
        let chrome_family = theme.mono_font_family.clone();
        let language = crate::gpui_shell::config::ui_language(cx);
        let tooltip = if collapsed {
            language.text(crate::i18n::Message::SidebarExpandGroup)
        } else {
            language.text(crate::i18n::Message::SidebarCollapseGroup)
        };
        let toggle_key = key.clone();
        let count: SharedString = tabs.to_string().into();
        // 主机头命名「SSH {目的地}」（用户 10-13 裁定）：typed 占位符词条，
        // 与 `settings.network.route.ssh_jump`（"SSH {target}"）同一先例——
        // SSH 是技术缩写，各语言词条同形。
        let name_text: SharedString =
            language.format(crate::i18n::Message::SidebarSshHost, &[("host", name)]).into();
        let name_selector: SharedString = format!("sidebar-host-name-{key}").into();
        h_flex()
            .id(("sidebar-host", row_ix))
            .debug_selector(move || format!("sidebar-host-{key}"))
            .w(px(row_w))
            .h(px(super::task_groups::GROUP_HEADER_H))
            .overflow_hidden()
            .pl(px(8.0))
            .pr(px(8.0))
            .gap_1()
            .items_center()
            .rounded(px(crate::display::UI_CORNER_RADIUS_LOGICAL))
            .font_weight(FontWeight::NORMAL)
            .cursor_pointer()
            .text_color(muted)
            .hover(|style| style.bg(hover_bg))
            .tooltip(move |window, cx| {
                tab_presentation::tooltip(tooltip.into(), window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_sidebar_group(toggle_key.clone(), cx);
            }))
            // 同组头/条目行：click 合成链路依赖视图节点的 mouse_down listener
            // 参与 bubble 派发，去掉它主机头的 on_click 不会触发。
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _: &MouseDownEvent, _, _| {}),
            )
            .child(
                h_flex()
                    .debug_selector(move || name_selector.to_string())
                    .w(px(super::task_groups::GROUP_CHEVRON_SLOT_W))
                    .h_full()
                    .flex_shrink_0()
                    .items_center()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .xsmall()
                        .text_color(muted),
                    ),
            )
            .child(
                // SSH 身份用 Network 矢量图标（用户 10-19 裁定替换 Nerd Font
                // 字位 U+F0716——缺字体的环境里渲染近似「H+」）；与 ssh_dialog
                // 的 SSH 图标同一语义。宽度槽不变，行几何不动。
                h_flex()
                    .w(px(TAB_LABEL_ICON_W))
                    .flex_shrink_0()
                    .items_center()
                    .child(
                        Icon::new(IconName::Network)
                            .xsmall()
                            .text_color(muted),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w(px(row_w * 0.6))
                    .truncate()
                    .font_family(chrome_family)
                    .text_size(px(label_px * SIDEBAR_TITLE_SCALE))
                    .text_color(theme.success)
                    .child(name_text),
            )
            // 名称之后补弹性空位，计数仍靠右（同项目组头）。
            .child(div().flex_1())
            .when(tabs > 1, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(label_px * SIDEBAR_TAG_SCALE))
                        .text_color(faint)
                        .child(count),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_info(destination: &str, git_root: &str, cwd: &str) -> TabGroupInfo {
        TabGroupInfo {
            host: Some(destination.into()),
            key: child_key(destination, git_root, cwd),
            name: child_name(destination, child_key(destination, git_root, cwd).as_ref()),
            branch: None,
        }
    }

    /// 键派生的三级回落：git 根优先 → 可组的 cwd → 兜底子组；名字取路径
    /// 末级，兜底子组保持哨兵名走翻译。
    #[test]
    fn child_key_falls_back_git_root_then_cwd_then_workspace() {
        let by_root = ssh_info("build@10.0.0.7", "/srv/work/pebrel", "/srv/work/pebrel/src");
        assert_eq!(by_root.key.as_ref(), "ssh:build@10.0.0.7|/srv/work/pebrel");
        assert_eq!(by_root.name.as_ref(), "pebrel");
        // 无根但有远端 cwd：子目录各自成组（旧集成/无 git 仓库时的形态）。
        let by_cwd = ssh_info("build@10.0.0.7", "", "/home/user/other");
        assert_ne!(by_root.key, by_cwd.key);
        assert_eq!(by_cwd.key.as_ref(), "ssh:build@10.0.0.7|/home/user/other");
        assert_eq!(by_cwd.name.as_ref(), "other");
        // cwd 未知（无集成）：兜底子组。
        let fallback = ssh_info("build@10.0.0.7", "", "");
        assert_eq!(fallback.key, fallback_child_key("build@10.0.0.7"));
        assert_eq!(fallback.name.as_ref(), super::task_groups::WORKSPACE_GROUP_KEY);
        // 相对路径不算远端 cwd。
        assert_eq!(
            child_key("d", "", "relative/junk"),
            fallback_child_key("d"),
            "non-absolute remote paths are not groupable"
        );
    }

    /// 两个主机上的同一路径是两个组：主机键前缀隔离，不与本地路径键、
    /// `recent:` 键撞车。
    #[test]
    fn keys_are_namespaced_per_host() {
        let a = child_key("a@host", "", "/home/user/proj");
        let b = child_key("b@host", "", "/home/user/proj");
        assert_ne!(a, b);
        assert!(!a.as_ref().starts_with("recent:"));
        assert_ne!(a.as_ref(), "/home/user/proj");
    }

    /// 远端分支随组信息上抛（分支 chip 的输入；非仓库为空）。
    #[test]
    fn branch_rides_the_group_info() {
        let info = ssh_info("build@10.0.0.7", "/srv/work/pebrel", "/srv/work/pebrel");
        assert_eq!(info.branch, None);
        assert_eq!(info.host.as_deref(), Some("build@10.0.0.7"));
    }

    /// 嵌套无集成 shell（fish 等）里 UserVar/标题静默、OSC 7 照发：cwd 移出
    /// 上报根后，根不得再把分组钉在旧项目上；路径组件边界（`/srv/work` 不
    /// 覆盖 `/srv/workflow`）与根下子目录（仍覆盖）各验一遍。
    #[test]
    fn stale_root_no_longer_pins_the_group_once_cwd_leaves_the_repo() {
        let moved_on = child_key("d", "/srv/work/pebrel", "/home/user/other");
        assert_eq!(moved_on.as_ref(), "ssh:d|/home/user/other", "cwd 移出根后按 cwd 成组");
        let still_inside = child_key("d", "/srv/work/pebrel", "/srv/work/pebrel/src");
        assert_eq!(still_inside.as_ref(), "ssh:d|/srv/work/pebrel", "根下子目录仍按根成组");
        let boundary = child_key("d", "/srv/work", "/srv/workflow");
        assert_eq!(boundary.as_ref(), "ssh:d|/srv/workflow", "前缀相同不是同一组件");
        // cwd 从未上报（无 OSC 7）时无从判陈旧，根保持可信（与旧行为一致）。
        assert_eq!(child_key("d", "/srv/work/pebrel", "").as_ref(), "ssh:d|/srv/work/pebrel");
    }

    /// 分支 chip 与根同源：根陈旧则分支同罪；空根（裸仓库形态）下分支仍展示。
    #[test]
    fn branch_chip_follows_the_root_staleness_verdict() {
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/srv/work/pebrel/src", None, None).as_deref(),
            Some("main")
        );
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/home/user/other", None, None),
            None,
            "旧仓库的分支不上新组"
        );
        assert_eq!(
            group_branch("main", "", "/srv/bare.git", None, None).as_deref(),
            Some("main"),
            "裸仓库分支保留"
        );
        assert_eq!(group_branch("", "/srv/work/pebrel", "/srv/work/pebrel", None, None), None);
        // 分支槽出现绝对路径形态（上游塞错槽）一律不信。
        assert_eq!(
            group_branch(
                "/srv/work/pebrel",
                "/srv/work/pebrel",
                "/srv/work/pebrel/src",
                None,
                None
            ),
            None
        );
        let confused = project("d", "/srv/work/other", "/srv/work/other", "/srv/work/other");
        assert_eq!(
            group_branch("main", "/srv/old", "/srv/work/other", Some(&confused), None),
            None,
            "探测的分支槽同样防御"
        );
    }

    /// 探测落地的时刻仲裁：晚于现存记录的探测才可覆盖；发出更早的迟到
    /// 探测不得回退；不可信值（空/路径形态）一律不落地。
    #[test]
    fn probe_heads_arbitrate_by_issue_time() {
        use std::time::Duration;
        let mut heads = SshGroupHeads::default();
        let key: SharedString = "ssh:d|/srv/work".into();
        let t0 = std::time::Instant::now();
        heads.apply_probe_head(&key, "main", t0);
        assert_eq!(heads.branches.get(&key).unwrap().0.as_ref(), "main");
        // 更晚发出的探测覆盖。
        heads.apply_probe_head(&key, "dev", t0 + Duration::from_millis(10));
        assert_eq!(heads.branches.get(&key).unwrap().0.as_ref(), "dev");
        // 迟到的旧探测（发出更早）不回退刚采信的值。
        heads.apply_probe_head(&key, "main", t0 + Duration::from_millis(5));
        assert_eq!(heads.branches.get(&key).unwrap().0.as_ref(), "dev");
        // 不可信值不落地（chip 保持最后已知值）。
        heads.apply_probe_head(&key, "/srv/work", t0 + Duration::from_secs(1));
        assert_eq!(heads.branches.get(&key).unwrap().0.as_ref(), "dev");
        heads.apply_probe_head(&key, "", t0 + Duration::from_secs(2));
        assert_eq!(heads.branches.get(&key).unwrap().0.as_ref(), "dev");
    }

    /// 组头 chip 随「该组最近一次上报」（LWW）：集成在线（根仍覆盖 cwd）
    /// 时 latest 顶掉冻结的 `view.branch`——fish/AI agent pane 的标题静默
    /// 值不再是组头的终点；latest 缺席或不可信（空/路径形态）时回落旧
    /// 语义；根失守的组键不消费 latest，探测补位继续管。
    #[test]
    fn the_latest_reported_branch_wins_while_the_reported_root_still_covers() {
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/srv/work/pebrel", None, Some("dev"))
                .as_deref(),
            Some("dev"),
            "the latest report outranks the pane's frozen title value"
        );
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/srv/work/pebrel", None, Some("/srv"))
                .as_deref(),
            Some("main"),
            "an implausible latest falls back to the pane value"
        );
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/srv/work/pebrel", None, Some("")).as_deref(),
            Some("main")
        );
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/srv/work/pebrel", None, None).as_deref(),
            Some("main"),
            "without a latest the pane value keeps the chip"
        );
        // 根失守：latest 不与探测竞争，rev-parse 的新事实胜出。
        let probe = project("d", "/home/user/other", "trunk", "/home/user/other");
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/home/user/other", Some(&probe), Some("dev"))
                .as_deref(),
            Some("trunk")
        );
    }

    fn project(
        destination: &str,
        root: &str,
        branch: &str,
        cwd: &str,
    ) -> super::super::remote_git::RemoteProject {
        super::super::remote_git::RemoteProject {
            destination: destination.to_owned(),
            root: root.to_owned(),
            branch: branch.to_owned(),
            cwd: cwd.to_owned(),
        }
    }

    /// fish 等无集成嵌套 shell：上报根陈旧后，探测根接管组键与分支 chip；
    /// 探测说不在仓库时回落 cwd 成组、无分支。
    #[test]
    fn probed_identity_takes_over_when_the_reported_root_is_stale() {
        let probe = project("d", "/home/user/other", "trunk", "/home/user/other");
        // 组键：探测根（覆盖 cwd）。
        assert_eq!(
            group_root("d", "/srv/work/pebrel", "/home/user/other", Some(&probe)),
            Some("/home/user/other")
        );
        // 分支：探测结果顶替静默的标题协议。
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/home/user/other", Some(&probe), None)
                .as_deref(),
            Some("trunk")
        );
        // 探测过但不在仓库：根回落空串（child_key 自行走 cwd），分支为空。
        let bare = project("d", "", "", "/home/user/plain");
        assert_eq!(group_root("d", "/srv/work/pebrel", "/home/user/plain", Some(&bare)), None);
        assert_eq!(
            group_branch("main", "/srv/work/pebrel", "/home/user/plain", Some(&bare), None),
            None
        );
        // 探测身份属于另一台主机：不得接管。
        let elsewhere = project("other-host", "/srv/work/other", "trunk", "/home/user/other");
        assert_eq!(group_root("d", "/srv/work/pebrel", "/home/user/other", Some(&elsewhere)), None);
        // 探测根不再覆盖 cwd（又 cd 走了）：同样不得接管。
        let moved = project("d", "/srv/work/other", "trunk", "/home/user/other");
        assert_eq!(group_root("d", "/srv/work/pebrel", "/srv/third", Some(&moved)), None);
    }
}
