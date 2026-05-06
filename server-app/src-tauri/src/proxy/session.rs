//! 进程内会话注册表 —— 平移自 Python `server-app/core/active_connections.py`。
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
use std::time::{SystemTime, UNIX_EPOCH};

use conduit_core::ProgressSink;
use tokio::sync::Mutex;

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
}

struct SessionEntry {
    info: Mutex<ConnectionInfo>,
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
                sent: AtomicU64::new(0),
                recv: AtomicU64::new(0),
            }),
        );
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

    pub async fn passive_clients(&self) -> Vec<PassiveClient> {
        self.inner.lock().await.passive.values().cloned().collect()
    }

    pub async fn passive_count(&self) -> usize {
        self.inner.lock().await.passive.len()
    }
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
        if let Ok(inner) = self.registry.inner.try_lock() {
            if let Some(entry) = inner.sessions.get(&self.session_id) {
                if sent_delta > 0 {
                    entry.sent.fetch_add(sent_delta, Ordering::Relaxed);
                }
                if recv_delta > 0 {
                    entry.recv.fetch_add(recv_delta, Ordering::Relaxed);
                }
            }
        }
    }
}

fn epoch_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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
