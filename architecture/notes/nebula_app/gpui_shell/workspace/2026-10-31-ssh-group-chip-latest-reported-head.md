# SSH 组头分支 chip 改随「组内最近一次上报」（LWW）

## Status

Accepted

## Context

SSH 任务组的分支 chip 此前取**首见 tab** 的 `TabGroupInfo.branch`，且聚合
只在组主 branch 为 `None` 时才补位。fish / AI agent pane 里 `NEBULA|`
标题协议静默（OSC 7 照发），`view.branch` 冻结在进入嵌套 shell / 启动
agent 前的旧支——冻结值**非空**，补位通道失效，组头被首见 tab 钉死在
旧支上。同组另一个集成在线的 tab 切分支并正常上报后，树上的 chip 不变；
右侧 Git 面板却能跟——它走 `refresh_remote_git` 的主动
`git status --porcelain=v2 --branch` 通道，不受集成静默影响（用户 10-31
报障的分辨点）。`refresh_remote_project` 的 rev-parse 探测只在「上报根
失守（cwd 移出根）」的变化沿上补位：cwd 未动的静默场景不触发，冻结值
没有任何新鲜数据源可以顶替。

对照：本地组无此问题——`project_group_cache` 每个提示符重读一次
`.git/HEAD`（文件系统权威、天然新鲜），SSH 侧的等价物只能是网络往返，
不能照抄。

## Evidence

`nebula_app/src/gpui_shell/workspace/task_groups.rs` 的 `grouped_rows`
聚合（branch 仅 `None` 补位）、`remote_git.rs` 的
`refresh_remote_project`（`git_root_covers` 为真即早退）、
`task_groups_ssh.rs` 的 `group_branch` 回落序（集成在线 → 根失守探测 →
裸仓库）、`remote_git.rs` 的 `wake_remote_git_refresh`（面板点击沿的
强制刷新，独立通道）。

## Decision

1. 组头 chip 的语义从「首见 tab 的派生值」改为「**该组最近一次上报的
   HEAD**」：`SshGroupHeads`（`task_groups_ssh.rs`）持有
   `branches: 组键 → 最近上报` 与 `seen: pane → 上次采信值`。
2. 写入沿：TitleChanged（`terminal_activity.rs`）→
   `record_ssh_group_branch`。只在「上报根仍覆盖 cwd」的组里采信——
   根失守的组键由 cwd 派生，chip 交给探测补位，**LWW 不与 rev-parse
   竞争**（主动探测的新事实胜过失守键下的旧上报）。
3. 读取沿：render 的 `tab_group_info` → `ssh_group_info` 内按组键就地
   查表（键在函数内派生，消费方无须重算，事件/render 共用同一函数防
   漂移）。`latest` 不可信（空/绝对路径形态）时回落 pane 自身值。
4. `seen` 去重防闪回：OSC 7 只报 cwd 也发 TitleChanged，没有这份去重，
   冻结 pane 的每一条 cwd 上报都会把组头拉回旧支。

## Rejected alternatives

- 聚合层「后见覆盖」：组头跟随组内最后一个 tab——组创建顺序决定对错
  （静默 tab 后进组时引入反向 bug），无新鲜度语义。
- 探测常态化（每 TitleChanged 发 rev-parse）：每提示符两条网络 exec，
  流量与远端负担不可接受；现有变化沿设计正是为此。
- `branch_reported_at` 时间戳 + 跨 pane 取最新：跨 pane 比较时刻需要
  侵入 `TabGroupInfo` 纯数据结构，且与 per-pane 去重等价复杂度，多一份
   时钟状态。
- 本地 `.git/HEAD` 直读的 SSH 等价物：SSH 侧只能走网络，同探测常态化。

## Consequences

组内只剩静默 pane 时 chip 显示最后上报值（与旧行为一致，不更差）。
pane 全部离开后条目残留：组重建首帧显示上次上报值，同仓库大概率仍对，
不值得为清理性挂 pane 生命周期（pane id 单调不复用，条目量级无害）。
本地组、SSH 键派生与排序均不受影响（2026-10-12 / 10-19 的决策继续
有效）。

## Validation

`task_groups_ssh.rs` 纯函数测试
（`the_latest_reported_branch_wins_while_the_reported_root_still_covers`
含 latest 不可信回落与「失守键不消费 latest」）；`workspace/tests.rs`
集成测试 `the_group_chip_follows_the_latest_reported_branch_not_the_
frozen_first_tab`（真实 TitleChanged 事件路由：基线 → B 上报跟随 →
A 的 cwd 沿不闪回；构造期 SSH 失败沿的竞态以清表隔离）。既有
`task_groups` 22 项、`ssh_sidebar_group` 7 项、`tab_duplication` 6 项
全绿；`sidebar_group_branch_chip_hovers…` 与 spinner 系列在本机为存量
时序失败（父提交同样失败，与本改动无关）。

## Supersedes

None

## Revisit when

若集成标题协议加上报序号 / 单调时间戳，或 rev-parse 探测因连接复用
变得足够便宜（exec 常态化），LWW 表可退役为探测直读。
