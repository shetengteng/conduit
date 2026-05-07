//! 进程内会话注册表 —— 跟踪当前所有活跃 HTTP/SOCKS5 代理会话（UI 活跃连接面板用）。
//!
//! 两类会话：
//! - **Active session**（[`ConnectionInfo`]）：HTTP CONNECT / SOCKS5 实际产生
//!   字节传输的连接，由 proxy handler 在 connect 之初 `add(...)` 拿到 `session_id`，
//!   每次 chunk 写完调 [`SessionProgressSink::on_progress`] 累加字节数，断开时
//!   `remove(...)`。
//! - **Passive client**（[`PassiveClient`]）：通过 `GET /api/clients/heartbeat`
//!   声明"已链接但暂无流量"的 client-app；UI 上要把这两类分开展示。
//!
//! 同时实现 [`conduit_core::ProgressSink`]（通过包装类 [`SessionProgressSink`]），
//! 直接交给 [`conduit_core::bidirectional_relay`] 做透传 + 字节计数。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use conduit_core::time::epoch_secs;
use conduit_core::ProgressSink;
use tokio::sync::Mutex;

/// passive client (心跳-only) 的 TTL: 30 秒.
///
/// client 端 [`crate::proxy::connectivity::Heartbeat`] 默认 10 秒发一次,
/// 容忍 3 次连续丢失即判定离线 (与传统 keep-alive 工业实践一致).
/// 必须与 [`http.rs`] 心跳响应里的 `ttl_sec` 字段保持同步,
/// 否则 client 端的预期 TTL 会与 server 实际清理时机不一致.
pub const PASSIVE_CLIENT_TTL_SEC: f64 = 30.0;

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub session_id: String,
    pub peer_ip: String,
    /// `"http"` 或 `"socks5"`。
    pub proto: &'static str,
    pub target: String,
    pub since: f64,
    pub last_seen: f64,
    pub sent_bytes: u64,
    pub recv_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct PassiveClient {
    pub peer_ip: String,
    pub client_name: String,
    pub version: String,
    pub first_seen: f64,
    pub last_seen: f64,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    sessions: HashMap<String, Arc<SessionEntry>>,
    passive: HashMap<String, PassiveClient>,
    /// per-peer 累计字节(monotonic, 不会因 session remove 而回退)。
    /// (sent_total, recv_total)。供 [`super::traffic_emitter`] 周期 tick
    /// 取差值算 bps;基于 peer 而非 session 是为了:
    /// 1) 短连接结束后字节不丢失,下一秒仍能算到 bps;
    /// 2) UI 端 ClientList 的 liveBps(peer) 与流量曲线 series[peer] 都按 peer 索引。
    peer_totals: HashMap<String, (u64, u64)>,
}

struct SessionEntry {
    info: Mutex<ConnectionInfo>,
    /// 与 `info.peer_ip` 同值,但提到顶层免锁同步读,
    /// [`SessionProgressSink::on_progress`] 是同步 trait 不能 await。
    peer_ip: String,
    sent: AtomicU64,
    recv: AtomicU64,
}

#[derive(Default)]
pub struct SessionRegistry {
    inner: Mutex<Inner>,
}

impl SessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 登记一条新代理会话。返回携带 `session_id` 的 [`ConnectionInfo`] 快照。
    pub async fn add(
        self: &Arc<Self>,
        peer_ip: String,
        proto: &'static str,
        target: String,
    ) -> ConnectionInfo {
        let mut inner = self.inner.lock().await;
        inner.next_id += 1;
        let sid = format!("s{}", inner.next_id);
        let now = epoch_secs();
        let info = ConnectionInfo {
            session_id: sid.clone(),
            peer_ip,
            proto,
            target,
            since: now,
            last_seen: now,
            sent_bytes: 0,
            recv_bytes: 0,
        };
        inner.sessions.insert(
            sid.clone(),
            Arc::new(SessionEntry {
                info: Mutex::new(info.clone()),
                peer_ip: info.peer_ip.clone(),
                sent: AtomicU64::new(0),
                recv: AtomicU64::new(0),
            }),
        );
        // 首次见到该 peer 时, 在 peer_totals 表里登记一个 0 起点,
        // 让 traffic_emitter 在 session 进/出过程中始终能拿到 baseline。
        inner.peer_totals.entry(info.peer_ip.clone()).or_insert((0, 0));
        info
    }

    /// 拿走一条会话；返回最终统计快照（若存在）。
    pub async fn remove(&self, session_id: &str) -> Option<ConnectionInfo> {
        let mut inner = self.inner.lock().await;
        let entry = inner.sessions.remove(session_id)?;
        // 把累计字节数 fold 进 ConnectionInfo 一起返回
        let mut info = entry.info.lock().await.clone();
        info.sent_bytes = entry.sent.load(Ordering::Relaxed);
        info.recv_bytes = entry.recv.load(Ordering::Relaxed);
        info.last_seen = epoch_secs();
        Some(info)
    }

    /// 给定 session_id 拿一个 [`ProgressSink`]，由 relay 累加字节。
    /// 即使 session 已被 `remove`，sink 仍是惰性查找（找不到就空操作），保证不 panic。
    pub fn sink_for(self: &Arc<Self>, session_id: String) -> Arc<dyn ProgressSink> {
        Arc::new(SessionProgressSink {
            registry: self.clone(),
            session_id,
        })
    }

    /// 全量快照——给 `/status` / IPC `list_sessions` 用。
    pub async fn snapshot(&self) -> Vec<ConnectionInfo> {
        let inner = self.inner.lock().await;
        let mut out = Vec::with_capacity(inner.sessions.len());
        for entry in inner.sessions.values() {
            let mut info = entry.info.lock().await.clone();
            info.sent_bytes = entry.sent.load(Ordering::Relaxed);
            info.recv_bytes = entry.recv.load(Ordering::Relaxed);
            out.push(info);
        }
        out
    }

    pub async fn active_count(&self) -> usize {
        self.inner.lock().await.sessions.len()
    }

    /// 当前所有"见过流量"的 peer 累计字节快照, 供 [`super::traffic_emitter`]
    /// 周期 tick 用差值算 per-peer bps。返回 `(peer_ip, sent_total, recv_total)`。
    /// 注意:peer_totals 表是单调递增的, session remove 不会回退, 因此即使
    /// 短连接在 tick 间隔内出生+死亡, 它的字节仍能被下一次 tick 计入。
    pub async fn peer_totals_snapshot(&self) -> Vec<(String, u64, u64)> {
        let inner = self.inner.lock().await;
        inner
            .peer_totals
            .iter()
            .map(|(p, (s, r))| (p.clone(), *s, *r))
            .collect()
    }

    /// 把一个 passive client 心跳更新到 registry。
    /// 返回 `true` 表示这是首次见到（用于 UI 展示"新加入"）。
    pub async fn touch_passive(&self, peer_ip: &str, name: &str, version: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let now = epoch_secs();
        match inner.passive.get_mut(peer_ip) {
            Some(c) => {
                c.client_name = name.to_string();
                c.version = version.to_string();
                c.last_seen = now;
                false
            }
            None => {
                inner.passive.insert(
                    peer_ip.to_string(),
                    PassiveClient {
                        peer_ip: peer_ip.to_string(),
                        client_name: name.to_string(),
                        version: version.to_string(),
                        first_seen: now,
                        last_seen: now,
                    },
                );
                true
            }
        }
    }

    /// 返回 passive client 列表 (会先惰性清理 [`PASSIVE_CLIENT_TTL_SEC`] 之外的过期条目).
    pub async fn passive_clients(&self) -> Vec<PassiveClient> {
        let mut inner = self.inner.lock().await;
        prune_expired_passive(&mut inner.passive, PASSIVE_CLIENT_TTL_SEC);
        inner.passive.values().cloned().collect()
    }

    /// 返回 passive client 数量 (会先惰性清理过期条目, 与 [`Self::passive_clients`] 行为一致).
    pub async fn passive_count(&self) -> usize {
        let mut inner = self.inner.lock().await;
        prune_expired_passive(&mut inner.passive, PASSIVE_CLIENT_TTL_SEC);
        inner.passive.len()
    }

    /// 强制清理所有过期 passive client, 返回被清掉的条目数.
    /// 用于后台 tick / 测试 / 显式触发场景.
    /// 当前生产路径完全靠 `passive_count` / `passive_clients` 的 lazy prune 兜底,
    /// 此方法暂时只在测试中调用; 保留 public 接口,供后续 GC tick / SSE 推送复用.
    #[allow(dead_code)]
    pub async fn prune_passive(&self) -> usize {
        let mut inner = self.inner.lock().await;
        let before = inner.passive.len();
        prune_expired_passive(&mut inner.passive, PASSIVE_CLIENT_TTL_SEC);
        before - inner.passive.len()
    }
}

/// 内部工具: 把 `passive` 表里 `last_seen + ttl < now` 的条目移除.
fn prune_expired_passive(passive: &mut HashMap<String, PassiveClient>, ttl_sec: f64) {
    let now = epoch_secs();
    passive.retain(|_, c| now - c.last_seen <= ttl_sec);
}

struct SessionProgressSink {
    registry: Arc<SessionRegistry>,
    session_id: String,
}

impl ProgressSink for SessionProgressSink {
    fn on_progress(&self, sent_delta: u64, recv_delta: u64) {
        // ProgressSink::on_progress 是同步的，但 SessionRegistry::inner 是 tokio Mutex。
        // 我们用 try_lock 避免阻塞；常态下没竞争（每个 session 只有一个 relay
        // 任务在累计），try_lock 即拿到。极端情况下错过一次累加（统计偏低 64KiB）
        // 是可接受的，下次 chunk 会跟上。
        if let Ok(mut inner) = self.registry.inner.try_lock() {
            if let Some(entry) = inner.sessions.get(&self.session_id) {
                if sent_delta > 0 {
                    entry.sent.fetch_add(sent_delta, Ordering::Relaxed);
                }
                if recv_delta > 0 {
                    entry.recv.fetch_add(recv_delta, Ordering::Relaxed);
                }
                let peer = entry.peer_ip.clone();
                let totals = inner.peer_totals.entry(peer).or_insert((0, 0));
                totals.0 = totals.0.saturating_add(sent_delta);
                totals.1 = totals.1.saturating_add(recv_delta);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_then_remove_returns_session() {
        let reg = SessionRegistry::new();
        let info = reg
            .add("192.168.1.10".into(), "http", "example.com:443".into())
            .await;
        assert_eq!(info.session_id, "s1");
        assert_eq!(reg.active_count().await, 1);
        let removed = reg.remove("s1").await.unwrap();
        assert_eq!(removed.target, "example.com:443");
        assert_eq!(reg.active_count().await, 0);
    }

    #[tokio::test]
    async fn sink_accumulates_bytes_into_session() {
        let reg = SessionRegistry::new();
        let info = reg
            .add("10.0.0.5".into(), "socks5", "host:443".into())
            .await;
        let sink = reg.sink_for(info.session_id.clone());
        sink.on_progress(1024, 0);
        sink.on_progress(0, 2048);
        sink.on_progress(512, 256);
        let final_info = reg.remove(&info.session_id).await.unwrap();
        assert_eq!(final_info.sent_bytes, 1024 + 512);
        assert_eq!(final_info.recv_bytes, 2048 + 256);
    }

    #[tokio::test]
    async fn passive_heartbeat_first_then_update() {
        let reg = SessionRegistry::new();
        assert!(reg.touch_passive("10.0.0.7", "alice-mac", "0.2.0").await);
        assert!(!reg.touch_passive("10.0.0.7", "alice-mac", "0.2.0").await);
        let list = reg.passive_clients().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].client_name, "alice-mac");
    }

    /// 关闭 client 后 server 端 passive 列表必须能在 TTL 过期后自动清掉,
    /// 否则 UI "待命客户端" 会永远保留已离线的条目 (Bug 回归保护).
    #[tokio::test]
    async fn passive_client_evicted_after_ttl() {
        let reg = SessionRegistry::new();
        assert!(reg.touch_passive("10.0.0.9", "ghost-mac", "0.2.0").await);
        assert_eq!(reg.passive_count().await, 1);

        // 把这条 passive 的 last_seen 手动回拨到 TTL+10s 之前 (模拟 client 静默 40s).
        {
            let mut inner = reg.inner.lock().await;
            let c = inner.passive.get_mut("10.0.0.9").unwrap();
            c.last_seen = epoch_secs() - PASSIVE_CLIENT_TTL_SEC - 10.0;
        }

        let pruned = reg.prune_passive().await;
        assert_eq!(pruned, 1, "should evict the stale passive client");
        assert_eq!(reg.passive_count().await, 0);
        assert!(reg.passive_clients().await.is_empty());
    }

    /// passive_clients() / passive_count() 必须在返回前自动清理过期条目,
    /// 这样 control_api `/api/clients` 不需要额外触发 prune.
    #[tokio::test]
    async fn passive_count_lazy_prunes_expired_entries() {
        let reg = SessionRegistry::new();
        assert!(reg.touch_passive("10.0.0.10", "fresh", "0.2.0").await);
        assert!(reg.touch_passive("10.0.0.11", "stale", "0.2.0").await);
        {
            let mut inner = reg.inner.lock().await;
            inner.passive.get_mut("10.0.0.11").unwrap().last_seen =
                epoch_secs() - PASSIVE_CLIENT_TTL_SEC - 5.0;
        }
        let count = reg.passive_count().await;
        assert_eq!(count, 1, "stale entry must be pruned by passive_count");
        let list = reg.passive_clients().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].client_name, "fresh");
    }

    #[tokio::test]
    async fn snapshot_reflects_live_byte_counters() {
        let reg = SessionRegistry::new();
        let a = reg.add("1.1.1.1".into(), "http", "a:1".into()).await;
        let b = reg.add("2.2.2.2".into(), "http", "b:2".into()).await;
        reg.sink_for(a.session_id.clone()).on_progress(100, 200);
        reg.sink_for(b.session_id.clone()).on_progress(300, 400);
        let snap = reg.snapshot().await;
        assert_eq!(snap.len(), 2);
        let by_id: HashMap<&str, &ConnectionInfo> =
            snap.iter().map(|c| (c.session_id.as_str(), c)).collect();
        assert_eq!(by_id["s1"].sent_bytes, 100);
        assert_eq!(by_id["s1"].recv_bytes, 200);
        assert_eq!(by_id["s2"].sent_bytes, 300);
        assert_eq!(by_id["s2"].recv_bytes, 400);
    }
}
