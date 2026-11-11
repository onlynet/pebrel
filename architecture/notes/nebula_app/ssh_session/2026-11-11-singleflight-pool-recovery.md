# 2026-11-11 连接池 single-flight 建连互斥与驱逐自愈

## Status

已实施（分支 feat/sidebar_tree_v232；spec 见 `docs/superpowers/specs/2026-11-11-ssh-restore-single-flight-design.md`，plan 见 `docs/superpowers/plans/2026-11-11-ssh-singleflight-pool-recovery.md`）。

## Context

恢复任务标签时同主机 N 个 SSH 标签并发建连，连接池只在认证完成后插入条目，建连期间无去重，强制密码登录下弹 N 次密码。长期运行后还观察到：pane 正常、git 面板（exec 走池）异常、复制标签要密码重连——池条目失效/被误驱逐后没有自愈路径。

## Evidence

- 池前检查与认证后插入：`nebula_app/src/ssh_session.rs` 的 `authenticated_route`（复用分支与 insert 之间无并发屏障）。
- 恢复入口：`session_snapshot.rs` `append_session_tab` → `spawn_tab_ssh` 逐标签立即 spawn 异步建连。
- 后台 exec 无失败回退：`open_exec_channel` 对池条目开通道失败原样报错，坏/满条目永留池中。
- pane 错误即驱逐共享条目：`lifecycle.rs` 清理分支只看本 pane 的 `result`，不问会话是否仍被其他 pane 使用。

## Decision

fork 独占模块 `ssh_session/singleflight.rs` 承载三件事，上游文件仅 6 处行级插入：

1. A：per-pool-key 建连互斥（`tokio::sync::Mutex` 注册表，公平 FIFO；`unattended` 直通保持设置页连接测试语义）；等待者靠上游 `authenticated_route` 开头的池前检查自然复用，上游函数体零改动。
2. S1：`open_exec_channel_recovered`——后台 exec 开通道失败即摘池条目，不主动重连（无凭据新连接只会随机弹密码框），池在下一次用户建连时自愈。
3. S3：`PaneLease` 计数（Drop 兜底提前返回路径的计数回收）+ `evict_if_last`——最后一个使用者离开才驱逐，共享会话不再被单个 pane 的通道错误误摘。

## Rejected alternatives

- 恢复层按主机分组串行等待：需把同步 `append_session_tab` 异步化，只覆盖恢复场景。
- 密码弹窗分发给并发认证：仍是 N 条连接，不满足共享通道；密码跨认证流传播。
- 全局建连锁：跳板递归（`open_transport` 内再建连）持锁再取锁，死锁；必须 per-key。
- 客户端通道数分流（S2）：分流即新 TCP 连接即多一次密码，仅诊断证实 MaxSessions 常态打满才考虑，本设计留扩展点未实施。

## Consequences

- 恢复/复制/后台首连同源串行化：同主机并发场景恰好一次密码输入。
- 取消等待中的建连（future drop）不经过 `release_establishment`，注册表条目留待下次 acquire/release 收敛；注册表上限为不同 pool_key 数，有界。
- A 汇聚更多流量到单连接，MaxSessions 压力提前，由 S1 的失败驱逐自然分流兜底。

## Validation

- 单测：`singleflight` 模块内 4 个测试（同键互斥、异键并发、取消清理、lease 计数）。
- SSH 域回归：`cargo test --bin pebrel ssh` 301 通过。
- 实机验收记录见 spec §10（恢复恰一次密码、共享通道、自愈路径）。

## Supersedes

None

## Revisit when

- 诊断证实服务端 MaxSessions 常态打满（远端 `grep MaxSessions /etc/ssh/sshd_config`，日志高频出现 "SSH pooled channel failed; reconnecting"）时实施 S2 通道分流。
- 上游若原生引入建连去重，按 #863 先例跟随上游、删除 fork 包装。
