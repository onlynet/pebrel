# SSH 连接池 single-flight 与自愈设计（恢复共享通道 + 后台通道康复）

- 日期：2026-11-11
- 状态：已确认范围（A + S1 + S3；S2 仅诊断证实后可选）
- 分支：`feat/sidebar_tree_v232`
- 上游归属核实（`git cat-file -e upstream/main:<path>`，基线 e1117d7e）：
  - 上游文件（仅追加式最小插入）：`nebula_app/src/ssh_session.rs`、`nebula_app/src/ssh_session/lifecycle.rs`
  - fork 独占（可自由扩展）：新模块 `nebula_app/src/ssh_session/singleflight.rs`、`remote_git.rs`、`fast_path.rs`

## 1. 背景与问题

**现象一**：窗口关闭重开恢复任务标签时，同一远程主机的 N 个 SSH 标签要输 N 次密码（远端强制密码登录、本机不保存凭证）。已登录状态下新开标签则只共享既有通道、零弹窗。

**现象二**：SSH 任务连接运行一段时间后，终端 pane 正常，但右侧 git 面板状态异常；此时复制该任务标签需要重新输密码重连。

## 2. 根因分析

### 2.1 池机制现状

- `authenticated_route`（`ssh_session.rs:761`）建连前先查 `connection_pool()`，按 `route.pool_key()`（目标 + 认证方式 + 跳板/代理链哈希，`route.rs:28`）命中且 `!is_closed()` 即复用——这是「已登录后新开标签共享通道」的机制。
- 池条目只在**完整认证成功后**才插入（`ssh_session.rs:819-833`），建连期间没有任何去重。
- 恢复路径 `session_recovery.rs:81` → `append_session_tab`（`session_snapshot.rs:35`）→ `spawn_tab_ssh` 逐标签同步 spawn 异步 `lifecycle::run`，N 个同主机标签**并发**查池——全部 miss → 各自 `open_transport` + `authenticate` → N 次密码弹窗。

### 2.2 两个缺口

- **缺口一（现象一）**：无 single-flight。并发建连彼此不可见，密码认证场景下并发即多弹窗。
- **缺口二（现象二）**：池自愈缺失，两个子路径：
  - git 面板走的 `open_exec_channel`（`ssh_session.rs:1091`）拿池连接开 `channel_open_session` 失败后**原样报错**，坏/满（如服务端 `MaxSessions` 打满）的池条目永留池中，每次刷新撞同一堵墙；pane 路径有 reused-失败驱逐重连回退（`lifecycle.rs:197-211`），后台路径没有对齐。
  - 任一 pane 出错退出即驱逐共享池条目（`lifecycle.rs:224-226` 只看本 pane `result.is_err()`，不问会话是否仍被其他 pane 使用）→ 池空 → 后台 exec 池 miss → 新连接认证（再次要密码）。

### 2.3 机制交叉说明

A 落地后同主机流量更多汇聚单连接，`MaxSessions` 压力**提前**；S1 的失败驱逐是天然分流（满连接被摘出池，下一次建连换新连接）。因此 A 与 S1/S3 必须捆绑，不可只做 A。

## 3. 目标与非目标

**目标**
1. 恢复（及一切并发首连）时同 pool_key 只有一次密码输入，其余共享同一 SSH 通道。
2. 第一个认证失败/取消时，等待者顺序接棒，绝不同时弹多个密码框。
3. 池条目失效（死/满）后能自愈，git 面板不再被摁死在坏连接上。
4. 共享会话的驱逐决策感知其他使用者。

**非目标**
- 不保存/缓存密码到磁盘（远程强制密码、本机不存凭证的现状不变）。
- 不做 UI 文案改动（等待者复用现有 `SshStage` 展示；零 i18n 新键）。
- 不默认做客户端通道数分流（S2，见 §8）。
- 不改 unattended（设置页连接测试）路径语义。

## 4. 设计总览

新增 fork 独占模块 `ssh_session/singleflight.rs` 承载三件事：

```
singleflight.rs（fork 独占，~150–250 行含测试）
├─ A  建连互斥：per-key tokio::sync::Mutex 注册表 + authenticated_route 串行化包装
├─ S1 exec 驱逐自愈：open_exec_channel_recovered（解析→池取→开通道→失败驱逐→Err）
└─ S3 引用计数守卫：pool_key → 活跃 pane 计数；evict_if_last
```

上游文件插入点（合计 7 行级，全部为行级替换或追加）：

| 文件 | 位置 | 改动 |
|---|---|---|
| `ssh_session.rs` | mods 区 | +1 行 `mod singleflight;` |
| `ssh_session.rs` | `:723` `authenticated_session_at` | 1 行：`authenticated_route(...)` → `singleflight::authenticated_route_serialized(...)` |
| `ssh_session.rs` | `:876` `open_transport` 跳板递归 | 1 行：同上包装 |
| `ssh_session.rs` | `:1073` `exec_capture` | 1 行：`open_exec_channel(...)` → `singleflight::open_exec_channel_recovered(...)` |
| `lifecycle.rs` | `:186` 取得 acquired 后 | +1 行 `singleflight::pane_joined(&acquired.key)` |
| `lifecycle.rs` | `:224-226` 清理分支 | +1 行 `pane_left`；1 行驱逐调用换 `evict_if_last` |

## 5. A：single-flight 建连互斥

```rust
pub(super) async fn authenticated_route_serialized<H: SshEventHost>(
    route: &ResolvedRoute, progress: Option<&H>, unattended: bool, allow_host_key_prompt: bool,
) -> Result<AcquiredSession, SessionError> {
    if unattended { return super::authenticated_route(route, progress, unattended, allow_host_key_prompt).await; }
    let _guard = establishment_lock(&route.pool_key()).await;   // per-key 公平锁
    super::authenticated_route(route, progress, unattended, allow_host_key_prompt).await
}
```

- `establishment_lock`：`OnceLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>`，键存在取 Arc，不存在建后返回；释放时若 `Arc::strong_count == 1`（仅注册表持有）则摘除，防注册表无限增长。
- 等待者语义：上游 `authenticated_route` 函数体第一件事就是查池（`ssh_session.rs:768-784`），拿到锁后自动命中胜者刚插入的条目——**上游函数体零改动**即获得共享。
- 失败接棒：第一个认证失败（密码错/取消）→ 锁释放 → 下一个等待者进入 → 池 miss → 自己成为建立者。逐个串行，绝不多框并发。
- 覆盖面：`authenticated_session_at` 是 `lifecycle::run`（pane/恢复/复制/重连）、completion、`open_exec_channel`、`open_sftp` 的共同入口，全部自然串行化；跳板递归（`:876`）单独包装，避免同跳板多目标重复要跳板密码。

**死锁分析**：锁粒度 = pool_key（含完整路由哈希）。跳板链在 `build_route` 内有 ancestors 环检测 + 深度上限，锁依赖图呈 target→jump 单向无环；跳板锁在建连内层短暂持有即释放，target 锁不嵌套等待反向键。无死锁。

**取消语义**：等待者在锁上等待时 pane 被关闭 → `drive` 的 `tokio::select!` 丢弃 future → 锁等待自动撤销，无残留。持有者被取消 → guard drop → 锁释放，未完成的连接按上游现状丢弃。

**锁等待 UX**：等待者 pane 停留在既有 `SshStage::Resolve/Connect` 展示（`ssh_connect_overlay`），不改文案。第一个弹窗是模态可见的，等待是用户可理解的。

## 6. S1：exec 通道失败驱逐自愈

```rust
pub(super) async fn open_exec_channel_recovered(
    raw_destination: &str,
) -> Result<russh::Channel<russh::client::Msg>, SessionError> {
    // 解析逻辑与上游 open_exec_channel 一致（增量原则：允许重复，不抽上游公共层）
    let (destination, profile) = resolve_destination(raw_destination)?;
    // 序列化已由 :723 插入点在 authenticated_session_at 层覆盖，此处直调上游函数
    let acquired = super::authenticated_session(&destination, &profile, None::<&NoopSshEventHost>).await?;
    match lifecycle::network("exec channel", acquired.session.channel_open_session()).await {
        Ok(channel) => Ok(channel),
        Err(error) => {
            super::evict_pooled_session(&acquired.key, &acquired.session).await;
            Err(error)
        },
    }
}
```

- **不主动重连**：后台路径重建连 = 无凭据新连接 = 随机弹密码框，用户体验更差。驱逐后返回 Err，git 面板维持既有错误渲染；池在下一次用户驱动建连（新开/复制标签，经 A 串行化只弹一次）时自愈。
- 获取连接同样走 A 的串行化路径（与 pane 同一把锁），避免「pane 正在建连+弹窗、git exec 并发再开一条连、再弹一个框」。
- `exec_private`（`ssh_session.rs:1078`，agent 中转路径）保持上游不动。

## 7. S3：池条目引用计数守卫

- fork 侧维护 `pool_key → 活跃 pane 计数`（`std::sync::Mutex<HashMap<String, u64>>`）。
- `pane_joined(key)`：`lifecycle::run` 首次取得 acquired 后调用（`:186` 后；`:200` 重连同 key 不重复计）。
- `pane_left(key)` + `evict_if_last(key, session)`：`lifecycle.rs:224-226` 清理分支——计数减到 0（最后一个使用者退出）或会话 `is_closed()` 时才驱逐；否则保留条目（其他 pane 仍在用）。
- 死会话无需立即驱逐的兜底：`authenticated_route:774` 取池时本就检查 `!is_closed()`，死条目自然被跳过并驱逐。
- 保留既有立即驱逐路径：`lifecycle.rs:198`（reused 开 shell 失败 → 连接对新建通道不可用，正是 S1 哲学的 pane 版）与 `Opening::drop`（`lifecycle.rs:270`）。

## 8. S2：通道数分流（可选，诊断门槛）

仅当诊断证实服务端 `MaxSessions` 常态打满（同主机标签多、通道长期 ≥ 上限）才做：计数超阈值（默认 8，留 2 余量）时池不再把该连接发给新消费者。代价：分流 = 新 TCP 连接 = 多一次密码（强制密码登录下无法避免），故不默认启用，且需配设置项。**本设计不实施，仅留扩展点**（S1 的驱逐计数与 S2 的分流计数同源，届时复用）。

## 9. 诊断清单（实施前执行，~0.5 天）

1. 远端：`grep -E 'MaxSessions|MaxStartups' /etc/ssh/sshd_config`。
2. 复现现象二时本机日志按序核查：
   - 「复用已认证 SSH 连接」出现后紧跟 exec 失败、无 reconnecting → S1 缺口实锤；
   - 某 pane 退出后池 miss / 出现新认证 → S3 缺口实锤；
   - 「SSH pooled channel failed; reconnecting」高频出现 → MaxSessions 嫌疑上升，评估 S2。
3. 实机三步法（pane list/read/send）区分远端回退与消费层 bug。

## 10. 测试与验收

**单元测试（全部在 fork 独占文件，零上游测试文件冲突）**
- A：可控内部 future 验证串行化性质（第二调用在第一释放后才进入）、unattended 直通、失败后接棒、注册表条目回收。
- S1：驱逐决策（开通道失败 → 条目出池）与错误透传。
- S3：joined/left 配对、最后使用者才驱逐、非最后保留、死会话路径。

**集成/验收（实机）**
- 密码强制主机开 3 个标签 → 关窗重开恢复：恰好 1 次密码弹窗，3 标签全部 Ready 且共享通道（远端 `who`/sshd 日志单会话）。
- 恢复后 git 面板正常刷新；杀掉远端会话/模拟通道打满后 git 面板报错可自愈（新开标签一次密码后恢复）。
- 取消第一个密码框：第二个标签接棒弹框，其余不并发弹。

**回归**：`cargo test --bin pebrel -- <ssh/lifecycle 域过滤>`；`python3 scripts/check_architecture.py --base <PR-base>`；`scripts/check_platform_cfg.py`（纯 Rust 无平台分支，预期无预算变化）。

## 11. 架构治理

- 上游文件净增 ~7 行（`ssh_session.rs` +4、`lifecycle.rs` +3），均为行级替换/追加，rebase 冲突面极小；`ssh_session.rs` 现 1966 行，增量后仍 < 2000 防灾上限。
- fork 新模块预估 < 300 行（含测试），低于 800 行提示线。
- 按模块 AGENTS.md，本改动涉及跨层生命周期/所有权决策，写 note：`architecture/notes/nebula_app/ssh_session/2026-11-11-singleflight-pool-recovery.md`（记录被否定方案：恢复层串行等待、密码分发）。
- 零 UI 文案、零 i18n 新键、零持久化格式变更。

## 12. 成本

| 项 | 内容 | 成本 |
|---|---|---|
| A | singleflight 模块 + 2 处调用点包装 | 0.5–1 天 |
| S1 | exec 驱逐自愈 + 1 处调用点 | ~0.5 天 |
| S3 | 引用计数守卫 + lifecycle 3 行 | ~0.5 天 |
| 诊断 | §9 清单 | ~0.5 天（可穿插） |
| 合计 | | **1.5–2 人天** |

## 13. 被否定方案

- **恢复层按主机分组串行等待**（app 层）：`append_session_tab` 现为同步签名，需异步化改造恢复管线（~150–300 行），只覆盖恢复场景，标签出现时间被拉长，成本 2–3 天。
- **密码弹窗分发**（一次输入喂 N 个并发认证）：仍是 N 条连接，不满足「共享同一通道」；密码在多认证流间传播，zeroize 路径复杂化。
- **全局建连锁**：跳板递归（`open_transport` 内嵌 `authenticated_route`）会在持锁状态下再取锁，直接死锁；必须 per-key。
