# SSH 组头 chip 的组级探测兜底（fish 里切分支）

## Status

Accepted

## Context

组头 chip 的 LWW（2026-10-31 note）只覆盖「集成 shell 的上报沿」：组内
正常 bash/zsh pane 切分支后 `NEBULA|` 上报，chip 跟随（实机 130562f2
已验证）。但**切分支动作本身发生在无集成 shell 里**时（用户在嵌套 fish
里 `git checkout`），该 pane 的上报静默、其它 pane 也没有任何信号——
LWW 无输入，chip 停在全组冻结值上（2026-11-01 实机报障：现场两个 pane
的 `view.branch` 都冻结在 checkout 前的分支，仓库真实 HEAD 已变，唯一
能发现真相的是主动 `rev-parse`）。右侧 Git 面板不受影响：它走点击沿的
强制 `git status` 通道。

## Evidence

实机现场（隔离实例 130562f2）：`pane list` 两个同组 SSH pane 的
branch 同为冻结值 `feature/op_recall_cache`，fish 里已切走；
`task_groups_ssh.rs` 的 `record_ssh_group_branch` 采信条件（branch 非空
且 seen 未命中）在静默 pane 上永远不成立；`remote_git.rs` 的
`refresh_remote_project` 只在「上报根失守」的变化沿探测，cwd 未动的
切分支不触发。

## Decision

1. 组级探测作为 LWW 的**第二写入源**：`probe_ssh_group_head`（remote_git
   域，与 `refresh_remote_project` 同命令构造/预算/解析）沿 TitleChanged
   （fish 的 OSC 7 照发，事件沿不缺）节流发起——每
   `HEAD_PROBE_THROTTLE`（2s）窗口每**组键**至多一条
   `rev-parse --abbrev-ref HEAD`，在途去重按组键。
2. 只在仓库组（root 覆盖 cwd，判据同 `ssh_group_info` 的 `group_root`）
   发起：cwd 组/兜底组没有 chip 可言。
3. 落地仲裁在 [`SshGroupHeads::apply_probe_head`]：探测结果带**发出
   时刻**参与比较，晚于现存记录才可覆盖——并发场景里刚到的上报更新鲜，
   迟到的旧探测不得回退；不可信值（空/路径形态）不落地；失败静默（chip
   保持最后已知值）。
4. 测试环境（`cfg(test)`）不发起远端 exec（gpui 调度器要求确定性）：
   节流/在途置位仍生效，落地链路由单测与集成测试直接驱动。

## Rejected alternatives

- 每 TitleChanged 立即探测：每提示符一条网络 exec，远端负担不可接受。
- 周期轮询所有组：常驻流量随主机×仓库数放大，终端工具不合适。
- 复用 Git 面板快照：面板状态机绑定「抽屉开 + Git 视图 + 聚焦 root」，
  chip 常驻且覆盖所有组，两种生命周期合不来。
- `branch_reported_at` 时间戳短路（上报新鲜的组跳过探测）：可再省
  bash 在线组的探测流量，但多一份 view 协议状态；第一版先按组键节流
  观察，流量确成问题再加。

## Consequences

fish 里切分支后 chip 的更新延迟 = 节流窗 + 网络往返（≤2s + RTT）。
集成在线的组也会周期探测（值与上报一致，浪费一条 rev-parse/2s，
组键级而非 pane 级）。probe 的组键残留与 LWW 残留同一语义（无害）。

## Validation

`task_groups_ssh.rs` 单测 `probe_heads_arbitrate_by_issue_time`（覆盖/
迟到回退/不可信值五断言）；`workspace/tests.rs` 集成
`the_group_chip_follows_a_probe_head_when_the_pane_shell_is_silent`
（静默 pane 的 seen 冻结下探测落地穿透 + 迟到不回退，red 验证过短路
apply 即红）；既有 `ssh_sidebar_group` 8 项、`task_groups` 23 项全绿。
组合跑偶发的 `git_drawer_click_edges…`/spinner 系列为父提交同样存在的
本机存量 flaky（stash 复核），与本改动无关。

## Supersedes

None（与 2026-10-31 LWW note 互补：上报沿与探测沿是同一仲裁链的两条
写入源）。

## Revisit when

若后续加 `branch_reported_at` 新鲜度短路，或集成标题协议带上报序号
（可精确判定静默），节流窗可放宽/退役；远端 exec 若常驻复用通道成
本趋零，可评估对所有展开组做周期探测。
