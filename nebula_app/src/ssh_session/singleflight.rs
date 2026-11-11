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
    let mut registry =
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

    fn establishment_registry_has(key: &str) -> bool {
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
        let gate =
            (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
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
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
            // 持有者：拿到锁后发信号、挂起，直至收到释放信号——期间一直持锁。
            let mut holder = Box::pin(serialized("cancel", async move {
                entered_tx.send(()).expect("进入信号通道存活");
                release_rx.await.expect("释放信号通道存活");
            }));
            tokio::select! {
                _ = holder.as_mut() => panic!("持有者不应在释放信号前完成"),
                entered = entered_rx => entered.expect("进入信号通道存活"),
            }
            // 等待者：持有者持锁期间构造并轮询，只能停在锁上；随后被取消。
            let mut waiter = Box::pin(serialized("cancel", async {}));
            tokio::select! {
                _ = waiter.as_mut() => panic!("等待者不应在持有者持锁期间完成"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
            }
            drop(waiter);
            release_tx.send(()).expect("释放信号通道存活");
            holder.await;
            // 取消过等待者之后，新调用必须仍可进入，且注册表正常收敛。
            serialized("cancel", async {}).await;
        });
        assert!(!establishment_registry_has("cancel"));
    }
}
