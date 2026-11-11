# SSH single-flight 与池自愈实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 恢复（及一切并发首连）时同 pool_key 只弹一次密码并共享 SSH 通道；后台 exec 通道失败驱逐自愈；池条目驱逐感知存活 pane 计数。

**Architecture:** 全部逻辑放 fork 独占新模块 `nebula_app/src/ssh_session/singleflight.rs`；上游文件 `ssh_session.rs`/`lifecycle.rs` 只做 6 处行级插入/替换（防 rebase 冲突）。规格见 `docs/superpowers/specs/2026-11-11-ssh-restore-single-flight-design.md`。

**Tech Stack:** Rust + tokio（`tokio::sync::Mutex` 建连互斥、`Notify` 测试门闩）、russh。无 UI/无 i18n/无持久化改动。

---

## 构建性能提示（本机实测）

- 编译验证用 check（增量、不链接，分钟级）：`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo check --bin pebrel --tests`
- 跑测试要链接，**首次约 25 分钟**，之后增量数分钟：`DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel <过滤>`
- 因此：单测红/绿循环尽量合并批次；Task 2/4 插入步骤只做 check + 既有测试，新测试集中在 Task 1/3 各跑一次、Task 6 统一回归。

## 已核实的锚点（写代码前不再重查）

| 锚点 | 位置 | 现内容 |
|---|---|---|
| mod 声明区 | `ssh_session.rs:38` | `mod sftp;`（其后插 `mod singleflight;`，字母序 sftp < singleflight） |
| 入口调用点 | `ssh_session.rs:723` | `    authenticated_route(&route, progress, false, true).await` |
| 跳板递归点 | `ssh_session.rs:876-883` | `Box::pin(authenticated_route(jump, None::<&NoopSshEventHost>, unattended, allow_host_key_prompt)).await` |
| exec 调用点 | `ssh_session.rs:1073` | `    let channel = open_exec_channel(raw_destination).await?;` |
| 取得连接后 | `lifecycle.rs:185-186` | `let mut acquired = super::authenticated_session_at(&resolved, &profile, Some(&event_proxy), 0).await?;` |
| 清理驱逐 | `lifecycle.rs:224-226` | `if result.is_err() \|\| acquired.session.is_closed() { super::evict_pooled_session(&acquired.key, &acquired.session).await; }` |
| 可见性先例 | — | lifecycle.rs 以 `super::authenticated_session_at`/`super::evict_pooled_session` 调父模块私有项——singleflight（ssh_session 的子模块）同权访问 `super::*` 私有项 |
| 类型 | — | `AcquiredSession{key,session,reused,jump_sessions}`（ssh_session.rs:58）、`type SharedSession = Arc<ClientSession>`（:56）、`struct NoopSshEventHost`（:101） |

---

### Task 1: singleflight 模块骨架 + 建连互斥核心（A 的锁原语）

**Files:**
- Create: `nebula_app/src/ssh_session/singleflight.rs`
- Modify: `nebula_app/src/ssh_session.rs:38`（mod 声明）

- [ ] **Step 1: 创建带 `todo!()` 骨架的模块文件**

创建 `nebula_app/src/ssh_session/singleflight.rs`（本阶段不引入任何 `use super::` 项，避免阶段性未使用告警；super 导入在 Task 2/3/5 按需添加）：

```rust
//! 连接池的 single-flight 与自愈（fork 增量；上游建连逻辑零改动）。
//!
//! A：per-pool-key 建连互斥——并发首连在锁上排队，胜者认证入池后，等待
//!    者经上游 `authenticated_route` 开头的池前检查自然复用；
//! S1：后台 exec 通道打开失败即摘除池条目，池在下一次用户建连时自愈；
//! S3：池条目驱逐感知存活 pane 计数，最后一个使用者离开才驱逐。

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

fn establishment_registry() -> &'static StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn acquire_establishment(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    todo!("Task 1 Step 5 实现")
}

fn release_establishment(key: &str) {
    todo!("Task 1 Step 5 实现")
}

async fn serialized<T>(key: &str, inner: impl Future<Output = T>) -> T {
    todo!("Task 1 Step 5 实现")
}
```

- [ ] **Step 2: 在 ssh_session.rs 挂载模块**

`nebula_app/src/ssh_session.rs:38`，`mod sftp;` 之后插一行：

```rust
mod singleflight;
```

- [ ] **Step 3: 写失败的单测（锁互斥核心性质）**

在 `singleflight.rs` 文件末尾追加（直接内联 `mod tests`，与上游 `#[cfg(test)] mod tests;` 分文件风格不同属有意为之——fork 文件零上游冲突）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// 与 lifecycle/tests.rs 同款：单线程 runtime + 10s 超时兜底（死锁转失败）。
    fn check(future: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(10), future)
                    .await
                    .expect("singleflight 测试超时（疑似死锁）");
            });
    }

    #[cfg(test)]
    fn establishment_registry_has(key: &str) -> bool {
        // （cfg(test) 冗余于外层 mod tests，保留无碍；pane_users 同。）
        establishment_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(key)
    }

    #[test]
    fn same_key_establishment_runs_exclusively() {
        let first_done = Arc::new(AtomicBool::new(false));
        let observed = Arc::new(AtomicBool::new(false));
        check(async {
            let a = serialized("exclusive", async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                first_done.store(true, Ordering::SeqCst);
            });
            let observed = observed.clone();
            let first_done = first_done.clone();
            let b = serialized("exclusive", async move {
                observed.store(first_done.load(Ordering::SeqCst), Ordering::SeqCst);
            });
            let ((), ()) = tokio::join!(a, b);
        });
        // b 进入即证明 a 已完成（join 语义）；若锁失效，b 会先进入且观察到 false。
        assert!(observed.load(Ordering::SeqCst), "第二个建连必须在第一个完成后才进入");
    }

    #[test]
    fn different_keys_establish_concurrently() {
        let inside = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let gate = (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
        check(async {
            let a = {
                let (flag, gate) = (inside.0.clone(), gate.0.clone());
                serialized("concurrent-a", async move {
                    flag.store(true, Ordering::SeqCst);
                    gate.notified().await;
                })
            };
            let b = {
                let (flag, gate) = (inside.1.clone(), gate.1.clone());
                serialized("concurrent-b", async move {
                    flag.store(true, Ordering::SeqCst);
                    gate.notified().await;
                })
            };
            let coordinator = async {
                while !(inside.0.load(Ordering::SeqCst) && inside.1.load(Ordering::SeqCst)) {
                    tokio::task::yield_now().await;
                }
                gate.0.notify_one();
                gate.1.notify_one();
            };
            tokio::join!(a, b, coordinator);
        });
    }

    #[test]
    fn dropped_waiter_leaves_registry_clean() {
        check(async {
            let holder = serialized("cancel", async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            });
            let waiter = serialized("cancel", async {});
            tokio::select! {
                _ = holder => {},
                _ = waiter => panic!("等待者不应先于持有者完成"),
            }
            // select 结束即丢弃 waiter（锁上被取消）；新调用必须仍可进入。
            serialized("cancel", async {}).await;
        });
        assert!(!establishment_registry_has("cancel"));
    }
}
```

- [ ] **Step 4: 跑测试确认红**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel singleflight`
Expected: 3 个测试全 FAIL（`todo!()` panic「not yet implemented」）。首次链接约 25 分钟。

- [ ] **Step 5: 实现三个函数体**

用以下实现替换三个 `todo!()`（`establishment_registry` 已在骨架中就位）：

```rust
fn acquire_establishment(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let registry =
        establishment_registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.entry(key.to_owned()).or_default().clone()
}

/// 调用方持锁期间注册表只挂自身一份 Arc；strong_count 回到 1 即摘键。
/// 被取消的等待者不经过这里（其 permit Arc 随 future drop），条目留待下
/// 一次 acquire/release 收敛——注册表上限为不同 pool_key 数，有界。
fn release_establishment(key: &str) {
    let mut registry =
        establishment_registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if registry.get(key).is_some_and(|lock| Arc::strong_count(lock) == 1) {
        registry.remove(key);
    }
}

async fn serialized<T>(key: &str, inner: impl Future<Output = T>) -> T {
    let permit = acquire_establishment(key);
    let guard = permit.lock().await;
    let result = inner.await;
    drop(guard);
    drop(permit);
    release_establishment(key);
    result
}
```

- [ ] **Step 6: 跑测试确认绿**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel singleflight`
Expected: PASS（3 个测试；增量链接数分钟）。若锁粒度写错，`same_key_establishment_runs_exclusively` FAIL。

- [ ] **Step 7: Commit**

```bash
git add nebula_app/src/ssh_session/singleflight.rs nebula_app/src/ssh_session.rs
git commit -m "feat(ssh): add per-key establishment lock primitive for pooled connections"
```

---

### Task 2: authenticated_route 串行化包装 + 两处上游调用点（A 接线）

**Files:**
- Modify: `nebula_app/src/ssh_session/singleflight.rs`
- Modify: `nebula_app/src/ssh_session.rs:723`、`:876-883`

- [ ] **Step 1: 在 singleflight.rs 追加包装函数**（`serialized` 定义之后；同时在文件头 `use std::sync::...` 之后补 `use super::{AcquiredSession, ResolvedRoute, SshEventHost, SessionError};`）

```rust
/// 并发首连的 single-flight 门卫：同 pool_key 的建连互斥排队。上游
/// `authenticated_route` 开头即查池（ssh_session.rs:768-784），等待者拿到
/// 锁后自动命中胜者刚插入的池条目，因此上游函数体保持零改动。
/// `unattended`（设置页连接测试）维持上游语义直通，不参与排队。
pub(super) async fn authenticated_route_serialized<H: SshEventHost>(
    route: &ResolvedRoute,
    progress: Option<&H>,
    unattended: bool,
    allow_host_key_prompt: bool,
) -> Result<AcquiredSession, SessionError> {
    if unattended {
        return super::authenticated_route(route, progress, unattended, allow_host_key_prompt)
            .await;
    }
    let key = route.pool_key();
    serialized(
        &key,
        super::authenticated_route(route, progress, unattended, allow_host_key_prompt),
    )
    .await
}
```

- [ ] **Step 2: 替换 ssh_session.rs:723**

```rust
    singleflight::authenticated_route_serialized(&route, progress, false, true).await
```

- [ ] **Step 3: 替换 ssh_session.rs:876-883 跳板递归调用名**

原：

```rust
            let acquired = Box::pin(authenticated_route(
                jump,
                None::<&NoopSshEventHost>,
                unattended,
                allow_host_key_prompt,
            ))
            .await
```

改为：

```rust
            let acquired = Box::pin(singleflight::authenticated_route_serialized(
                jump,
                None::<&NoopSshEventHost>,
                unattended,
                allow_host_key_prompt,
            ))
            .await
```

（`Box::pin` 保留：serialized → authenticated_route → open_transport → serialized 的递归类型尺寸仍由它截断。）

- [ ] **Step 4: 编译 + 既有 SSH 域回归**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo check --bin pebrel --tests`
Expected: 无 error。

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel ssh`
Expected: 全 PASS（lifecycle/agent/route 既有测试；fixture 直插 pool 的测试不受影响——它们不走建连入口）。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/ssh_session/singleflight.rs nebula_app/src/ssh_session.rs
git commit -m "feat(ssh): serialize concurrent first connections per pool key"
```

---

### Task 3: PaneLease 引用计数（S3 原语）

**Files:**
- Modify: `nebula_app/src/ssh_session/singleflight.rs`

- [ ] **Step 1: 写失败的单测**（追加进 `mod tests`）

```rust
    #[cfg(test)]
    fn pane_users(key: &str) -> u64 {
        pane_counts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    #[test]
    fn pane_lease_counts_users_and_releases_on_drop() {
        assert_eq!(pane_users("lease-key"), 0);
        let first = PaneLease::new("lease-key");
        assert_eq!(pane_users("lease-key"), 1);
        let second = PaneLease::new("lease-key");
        assert_eq!(pane_users("lease-key"), 2);
        assert!(!first.is_last_pane());
        assert!(!second.is_last_pane());
        drop(second);
        assert!(first.is_last_pane());
        drop(first);
        assert_eq!(pane_users("lease-key"), 0);
    }
```

- [ ] **Step 2: 验证失败**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo check --bin pebrel --tests`
Expected: FAIL（`PaneLease` 未定义，编译错误即红）。

- [ ] **Step 3: 实现 PaneLease**（`serialized` 与 `authenticated_route_serialized` 之后、`mod tests` 之前；`use super::{...}` 追加 `SharedSession`）

```rust
fn pane_counts() -> &'static StdMutex<HashMap<String, u64>> {
    static COUNTS: OnceLock<StdMutex<HashMap<String, u64>>> = OnceLock::new();
    COUNTS.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// 一个 pane 对某 pool_key 连接的使用权。Drop 释放计数——`lifecycle::run`
/// 的任何提前返回（如首次开 shell 失败的 `return Err`）也不会泄漏。
pub(super) struct PaneLease {
    key: String,
}

impl PaneLease {
    pub(super) fn new(key: &str) -> Self {
        let mut counts =
            pane_counts().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *counts.entry(key.to_owned()).or_insert(0) += 1;
        Self { key: key.to_owned() }
    }

    fn users(&self) -> u64 {
        pane_counts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&self.key)
            .copied()
            .unwrap_or(0)
    }

    /// 计数含自身；`users() <= 1` 即本 pane 是该连接的最后一个使用者。
    pub(super) fn is_last_pane(&self) -> bool {
        self.users() <= 1
    }

    /// 上游清理分支（lifecycle.rs:224）的守卫版驱逐：仅当本 pane 是最后
    /// 使用者才驱逐。共享会话被其他 pane 正常使用时，单个 pane 的通道错
    /// 误不再把池条目误摘。
    pub(super) async fn evict_if_last(&self, session: &SharedSession) {
        if self.is_last_pane() {
            super::evict_pooled_session(&self.key, session).await;
        }
    }
}

impl Drop for PaneLease {
    fn drop(&mut self) {
        let mut counts =
            pane_counts().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match counts.get_mut(&self.key) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                counts.remove(&self.key);
            },
            None => {},
        }
    }
}
```

- [ ] **Step 4: 验证通过**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel singleflight`
Expected: PASS（4 个测试）。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/ssh_session/singleflight.rs
git commit -m "feat(ssh): add pane lease counting for pool eviction guard"
```

---

### Task 4: lifecycle.rs 接线（S3 生效）

**Files:**
- Modify: `nebula_app/src/ssh_session/lifecycle.rs:186`、`:224-226`

- [ ] **Step 1: 取得连接后挂 lease**（`:186` 的 `.await?;` 行之后插一行）

```rust
        let _pane_lease = super::singleflight::PaneLease::new(&acquired.key);
```

（`:200` 重连重赋值 `acquired` 同 key，不重复挂；`:212` 提前 `return Err` 由 lease 的 Drop 兜底计数。）

- [ ] **Step 2: 清理分支换守卫驱逐**

原（`lifecycle.rs:224-226`）：

```rust
        if result.is_err() || acquired.session.is_closed() {
            super::evict_pooled_session(&acquired.key, &acquired.session).await;
        }
```

改为：

```rust
        if result.is_err() || acquired.session.is_closed() {
            _pane_lease.evict_if_last(&acquired.session).await;
        }
```

- [ ] **Step 3: 编译 + 回归**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo check --bin pebrel --tests`
Expected: 无 error（`_pane_lease` 在 `:225` 被使用，无未使用告警）。

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel ssh`
Expected: 全 PASS。

- [ ] **Step 4: Commit**

```bash
git add nebula_app/src/ssh_session/lifecycle.rs
git commit -m "feat(ssh): guard pool eviction with live pane count"
```

---

### Task 5: exec 通道驱逐自愈（S1）

**Files:**
- Modify: `nebula_app/src/ssh_session/singleflight.rs`
- Modify: `nebula_app/src/ssh_session.rs:1073`

- [ ] **Step 1: 在 singleflight.rs 追加恢复版通道获取**（`PaneLease` 之后；`use super::{...}` 追加 `NoopSshEventHost, SshDestination`）

```rust
/// 后台 exec 的通道获取：解析地址 → 取（或按需建立）已认证连接 → 开独立
/// exec 通道。与上游 `open_exec_channel` 的唯一差别在最后一步：开通道失
/// 败立即摘除这条池连接——坏/满（如服务端 MaxSessions 打满）的连接不再
/// 发给后续消费者，池在下一次用户驱动的建连时自愈。不主动重连：后台路
/// 径重建连等于无凭据新连接，只会把密码框随机弹给用户。
pub(super) async fn open_exec_channel_recovered(
    raw_destination: &str,
) -> Result<russh::Channel<russh::client::Msg>, SessionError> {
    let profiles_path = crate::display::nebula_data_dir().join("ssh_profiles.json");
    let raw = raw_destination.to_owned();
    let (destination, profile) = tokio::task::spawn_blocking(move || {
        let destination = SshDestination::resolve(&raw)?;
        let profiles = crate::ssh_profiles::SshProfiles::load(&profiles_path)
            .unwrap_or_else(|_| crate::ssh_profiles::SshProfiles::default());
        Ok::<_, std::io::Error>((destination, profiles.for_destination(&raw)))
    })
    .await
    .map_err(|err| format!("SSH 地址解析任务失败: {err}"))??;

    let acquired =
        super::authenticated_session_at(&destination, &profile, None::<&NoopSshEventHost>, 0)
            .await?;
    match super::lifecycle::network("exec channel", acquired.session.channel_open_session()).await
    {
        Ok(channel) => Ok(channel),
        Err(error) => {
            super::evict_pooled_session(&acquired.key, &acquired.session).await;
            Err(error)
        },
    }
}
```

- [ ] **Step 2: 替换 ssh_session.rs:1073 调用点**

原：

```rust
    let channel = open_exec_channel(raw_destination).await?;
```

改为：

```rust
    let channel = singleflight::open_exec_channel_recovered(raw_destination).await?;
```

（`exec_capture` 的调用方 remote_git.rs 零改动；`open_exec_channel` 本体与 `exec_private`（:1078）保持上游原样。）

- [ ] **Step 3: 编译 + 回归**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo check --bin pebrel --tests`
Expected: 无 error（`SshDestination`/`NoopSshEventHost` 已按 Step 1 补进 use 列表）。

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel ssh`
Expected: 全 PASS（completion 的 exec 相关测试走 fixture 直插 pool，不经过此入口）。

- [ ] **Step 4: Commit**

```bash
git add nebula_app/src/ssh_session/singleflight.rs nebula_app/src/ssh_session.rs
git commit -m "feat(ssh): evict pooled session when background exec channel fails"
```

---

### Task 6: 门禁验证 + 因果 note

**Files:**
- Create: `architecture/notes/nebula_app/ssh_session/2026-11-11-singleflight-pool-recovery.md`

- [ ] **Step 1: 格式与架构门禁**

```bash
cargo fmt
python3 scripts/check_architecture.py --base ed527b2c
```

Expected: arch PASS（`ssh_session.rs` 净 +4 行 → 1970 < 2000；`lifecycle.rs` 净 +1 行；`singleflight.rs` 新文件 ~400 行 < 800 提示线）。若 fmt 改动了文件，重新 `git add` 并并入本任务提交。

- [ ] **Step 2: 新增测试统一回归**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel singleflight`
Expected: PASS（4 个测试）。

- [ ] **Step 3: 撰写 note**（9 节、≤200 行，依据 `architecture/notes/AGENTS.md`）

```markdown
# 2026-11-11 连接池 single-flight 建连互斥与驱逐自愈

## Status

已实施（分支 feat/sidebar_tree_v232，spec 见 docs/superpowers/specs/2026-11-11-ssh-restore-single-flight-design.md）。

## Context

恢复任务标签时同主机 N 个 SSH 标签并发建连，连接池只在认证完成后插入条目，建连期间无去重，
强制密码登录下弹 N 次密码。长期运行后还观察到：pane 正常、git 面板（exec 走池）异常、复制
标签要密码重连——池条目失效/被误驱逐后没有自愈路径。

## Evidence

- 池前检查与认证后插入：nebula_app/src/ssh_session.rs 的 authenticated_route（复用分支与
  insert 之间无并发屏障）。
- 恢复入口：session_snapshot.rs append_session_tab → spawn_tab_ssh 逐标签立即 spawn 异步建连。
- 后台 exec 无失败回退：open_exec_channel 对池条目开通道失败原样报错。
- pane 错误即驱逐共享条目：lifecycle.rs 清理分支只看本 pane 的 result。

## Decision

fork 独占模块 ssh_session/singleflight.rs 承载三件事，上游文件仅 6 处行级插入：
1. A：per-pool-key 建连互斥（tokio::sync::Mutex 注册表，公平 FIFO；unattended 直通）；
   等待者靠上游 authenticated_route 开头的池前检查自然复用，上游函数体零改动。
2. S1：open_exec_channel_recovered——后台 exec 开通道失败即摘池条目，不主动重连
   （无凭据新连接只会随机弹密码框），池在下一次用户建连时自愈。
3. S3：PaneLease 计数（Drop 兜底提前返回路径）+ evict_if_last——最后一个使用者
   离开才驱逐，共享会话不再被单个 pane 的通道错误误摘。

## Rejected alternatives

- 恢复层按主机分组串行等待：需把同步 append_session_tab 异步化，只覆盖恢复场景。
- 密码弹窗分发给并发认证：仍是 N 条连接，不满足共享通道；密码跨认证流传播。
- 全局建连锁：跳板递归（open_transport 内再建连）持锁再取锁，死锁。
- 客户端通道数分流（S2）：分流即新 TCP 连接即多一次密码，仅诊断证实 MaxSessions
  常态打满才考虑，本设计留扩展点未实施。

## Consequences

- 恢复/复制/后台首连同源串行化：同主机并发场景恰好一次密码输入。
- 取消等待中的建连（future drop）不经过 release_establishment，注册表条目留待下
  次 acquire/release 收敛；注册表上限为不同 pool_key 数，有界。
- A 汇聚更多流量到单连接，MaxSessions 压力提前，由 S1 的失败驱逐自然分流兜底。

## Validation

- 单测：singleflight 模块内 4 个测试（同键互斥、异键并发、取消清理、lease 计数）。
- 实机验收记录见 spec §10。

## Supersedes

None

## Revisit when

- 诊断证实服务端 MaxSessions 常态打满（远端 grep MaxSessions /etc/ssh/sshd_config，
  日志高频出现 "SSH pooled channel failed; reconnecting"）时实施 S2 通道分流。
- 上游若原生引入建连去重，按 #863 先例跟随上游、删除 fork 包装。
```

- [ ] **Step 4: Commit**

```bash
git add architecture/notes/nebula_app/ssh_session/2026-11-11-singleflight-pool-recovery.md
git commit -m "docs: record the singleflight pool recovery causal note"
```

---

### Task 7: 实机验收（需用户配合，含 spec §9 诊断）

- [ ] **Step 1: 诊断基线（远端执行一次）**

```bash
grep -E 'MaxSessions|MaxStartups' /etc/ssh/sshd_config
```

记录结果进验收记录（决定 S2 是否提上日程）。

- [ ] **Step 2: 验收清单（密码强制主机，开 3 个同主机标签 → 关窗 → 重开）**

1. 恢复时**恰好 1 次**密码弹窗，3 标签全部到达 Ready。
2. 取消第一个密码框 → 第二个标签接棒弹框（一次一个，绝不并发多框）。
3. 恢复完成后远端单会话：`who` 或 sshd 日志确认 3 个标签共享一条连接。
4. git 面板正常刷新（exec 走共享通道）。
5. 复制标签：共享连接健康时**不再要密码**（池命中直接开通道）。

- [ ] **Step 3: 自愈路径验收**

杀掉远端会话（或 `~.` 断开）后：git 面板短暂报错；新开一个同主机标签（一次密码）后 git 面板恢复刷新。

- [ ] **Step 4: 结果记录**

验收结论追加到 spec 文件末尾（一行式：日期 + 通过项 + MaxSessions 实测值），

```bash
git add docs/superpowers/specs/2026-11-11-ssh-restore-single-flight-design.md
git commit -m "docs: record singleflight acceptance results"
```

---

## 完成判据

- [ ] 4 个新单测 PASS；既有 `--bin pebrel ssh` 域回归 PASS
- [ ] `check_architecture.py --base ed527b2c` PASS；`cargo fmt` 无差异
- [ ] 上游文件改动合计 6 处行级（ssh_session.rs 4、lifecycle.rs 2）
- [ ] 实机验收 7 项全过，结果已记录
