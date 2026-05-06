//! `connectivity` —— 一次性 probe + 周期心跳。
//!
//! 两个公开能力：
//! - [`probe`] —— 一次性可达性验证（HTTP / SOCKS 端口 TCP 三次握手）
//! - [`Heartbeat`] —— 进入 connected 后周期 probe，连续失败时通过 EventBus 推
//!   `heartbeat_changed`（tone = green/yellow/red）

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{debug, info, warn};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use conduit_core::{EventBus, HeartbeatState, HeartbeatTone, ProbeResult};

use super::core::ClientEvent;

pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
pub const HEARTBEAT_YELLOW_AT: u32 = 1;
pub const HEARTBEAT_RED_AT: u32 = 3;

/// 一次性 probe：HTTP + SOCKS 都做 TCP 三次握手，任一失败即整体失败。
///
/// `server_vpn` 通过 `vpn_hint` 回填（来源：server 在 mDNS TXT 上自报的 `vpn=on/off`）。
pub async fn probe(
    host: &str,
    http_port: u16,
    socks_port: u16,
    vpn_hint: bool,
    timeout_dur: Duration,
) -> ProbeResult {
    let start = std::time::Instant::now();
    let http_ok = tcp_probe(host, http_port, timeout_dur).await;
    let socks_ok = tcp_probe(host, socks_port, timeout_dur).await;
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    let ok = http_ok && socks_ok;
    let error = if !ok {
        let mut bits = vec![];
        if !http_ok {
            bits.push(format!("http :{http_port} unreachable"));
        }
        if !socks_ok {
            bits.push(format!("socks :{socks_port} unreachable"));
        }
        Some(bits.join("; "))
    } else {
        None
    };
    ProbeResult {
        ok,
        healthz_ok: ok, // 简化：当作 healthz_ok = http_reachable
        socks_reachable: socks_ok,
        http_reachable: http_ok,
        error,
        latency_ms,
        server_vpn: vpn_hint,
    }
}

async fn tcp_probe(host: &str, port: u16, timeout_dur: Duration) -> bool {
    let target = format!("{host}:{port}");
    matches!(
        timeout(timeout_dur, TcpStream::connect(&target)).await,
        Ok(Ok(_))
    )
}

/// 心跳协程：每 `interval` 跑一次 probe，更新 EventBus 心跳状态。
///
/// 副作用：每次 TCP probe 通过后，会向 server 发送一次
/// `GET /api/clients/heartbeat?name=&version=`，让 server 端把本机注册成
/// "passive client"（已连接但暂无流量），server UI 才能显示客户端已接入。
pub struct Heartbeat {
    bus: EventBus<ClientEvent>,
    target: HeartbeatTarget,
    cancel: CancellationToken,
    state: Arc<Mutex<HeartbeatState>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    http: reqwest::Client,
}

#[derive(Clone)]
struct HeartbeatTarget {
    host: String,
    http_port: u16,
    socks_port: u16,
    client_name: String,
    client_version: String,
}

impl Heartbeat {
    pub fn new(
        bus: EventBus<ClientEvent>,
        host: String,
        http_port: u16,
        socks_port: u16,
        client_name: String,
        client_version: String,
    ) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Arc::new(Self {
            bus,
            target: HeartbeatTarget {
                host,
                http_port,
                socks_port,
                client_name,
                client_version,
            },
            cancel: CancellationToken::new(),
            state: Arc::new(Mutex::new(HeartbeatState {
                tone: HeartbeatTone::Green,
                consecutive_failures: 0,
                last_check_at: 0.0,
                last_error: None,
            })),
            handle: Mutex::new(None),
            http,
        })
    }

    pub async fn start(self: Arc<Self>) {
        let me = self.clone();
        let task = tokio::spawn(async move {
            me.run_loop().await;
        });
        *self.handle.lock().await = Some(task);
        info!(
            "[heartbeat] started target={}:{} every {}s",
            self.target.host,
            self.target.http_port,
            DEFAULT_HEARTBEAT_INTERVAL.as_secs()
        );
    }

    pub async fn stop(&self) {
        self.cancel.cancel();
        if let Some(h) = self.handle.lock().await.take() {
            let _ = h.await;
        }
        info!("[heartbeat] stopped");
    }

    pub async fn snapshot(&self) -> HeartbeatState {
        self.state.lock().await.clone()
    }

    async fn run_loop(self: Arc<Self>) {
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            let result = probe(
                &self.target.host,
                self.target.http_port,
                self.target.socks_port,
                false,
                Duration::from_secs(5),
            )
            .await;
            self.update_state(&result).await;

            // TCP 双端口都通才发 HTTP 心跳通知 server。HTTP 失败不计入心跳健康
            // (server 可能临时高负载或 reqwest 重试中),只 warn 一行。
            if result.ok {
                self.notify_server().await;
            }

            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(DEFAULT_HEARTBEAT_INTERVAL) => {}
            }
        }
    }

    /// 给 server 发 `GET /api/clients/heartbeat?name=&version=`,让 server 把
    /// 本机登记成 passive client。server 端实现见
    /// `server-app/src-tauri/src/proxy/http.rs::serve_heartbeat`。
    async fn notify_server(&self) {
        let url = format!(
            "http://{}:{}/api/clients/heartbeat?name={}&version={}",
            self.target.host,
            self.target.http_port,
            urlencoding(&self.target.client_name),
            urlencoding(&self.target.client_version),
        );
        match self.http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                debug!("[heartbeat] notified server passive registry: {url}");
            }
            Ok(resp) => {
                warn!("[heartbeat] server returned {} for {url}", resp.status());
            }
            Err(e) => {
                warn!("[heartbeat] failed to notify server passive registry: {e}");
            }
        }
    }

    async fn update_state(&self, probe: &ProbeResult) {
        let mut state = self.state.lock().await;
        state.last_check_at = epoch_now();
        if probe.ok {
            if state.consecutive_failures > 0 {
                debug!("[heartbeat] recovered from {} failures", state.consecutive_failures);
            }
            state.consecutive_failures = 0;
            state.last_error = None;
            state.tone = HeartbeatTone::Green;
        } else {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
            state.last_error = probe.error.clone();
            let prev = state.tone;
            state.tone = if state.consecutive_failures >= HEARTBEAT_RED_AT {
                HeartbeatTone::Red
            } else if state.consecutive_failures >= HEARTBEAT_YELLOW_AT {
                HeartbeatTone::Yellow
            } else {
                HeartbeatTone::Green
            };
            if prev != state.tone {
                warn!(
                    "[heartbeat] {prev:?} → {:?} after {} failures: {}",
                    state.tone,
                    state.consecutive_failures,
                    probe.error.as_deref().unwrap_or("?")
                );
            }
        }
        let snapshot = state.clone();
        drop(state);
        self.bus.publish(ClientEvent {
            kind: "heartbeat_changed".into(),
            payload: serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
            ts: epoch_now(),
        });
    }
}

fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// 极简 URL 编码：保留 ASCII 字母 / 数字 / `-_.~`,其余字节按 `%XX` 转义。
/// 仅服务于 query string 拼装,不追求 RFC3986 完整覆盖。
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_unreachable_target_returns_error() {
        // 192.0.2.1 是 TEST-NET-1，保证不可路由
        let r = probe("192.0.2.1", 12345, 12346, false, Duration::from_millis(200)).await;
        assert!(!r.ok);
        assert!(!r.http_reachable);
        assert!(!r.socks_reachable);
        assert!(r.error.is_some());
    }

    #[tokio::test]
    async fn probe_reports_latency() {
        let r = probe("192.0.2.1", 12345, 12346, true, Duration::from_millis(100)).await;
        assert!(r.latency_ms > 0.0);
        assert!(r.server_vpn);
    }

    #[test]
    fn epoch_now_is_positive() {
        let n = epoch_now();
        assert!(n > 1_700_000_000.0);
    }

    #[test]
    fn urlencoding_escapes_special_chars() {
        assert_eq!(urlencoding("alice-mac"), "alice-mac");
        assert_eq!(urlencoding("v0.2.0"), "v0.2.0");
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencoding("中"), "%E4%B8%AD");
    }

    /// 起一个 mock TCP listener 模拟 server 的 HTTP listener,
    /// 接受一次 GET 请求后回 200 OK,验证 Heartbeat 真的会发 GET /api/clients/heartbeat。
    #[tokio::test]
    async fn heartbeat_notifies_server_after_successful_probe() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // mock server 在两个端口上监听:http_port 处理 heartbeat 请求,
        // socks_port 只接受 TCP 连接(让 probe 通过)。
        let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_port = http_listener.local_addr().unwrap().port();
        let socks_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socks_port = socks_listener.local_addr().unwrap().port();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4);
        let tx_socks = tx.clone();

        // socks 端口只处理 probe 的 TCP 三次握手,accept 后立即关闭。
        tokio::spawn(async move {
            while let Ok((mut s, _)) = socks_listener.accept().await {
                let _ = tx_socks.send("socks-accept".into()).await;
                let _ = s.shutdown().await;
            }
        });

        // http 端口接受 GET 后 capture request line 推到 channel 再回 200。
        tokio::spawn(async move {
            while let Ok((mut s, _)) = http_listener.accept().await {
                let mut buf = [0u8; 1024];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx.send(req).await;
                let body = b"{\"ok\":true,\"created\":true,\"ttl_sec\":60}";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(body).await;
                let _ = s.shutdown().await;
            }
        });

        let bus: EventBus<ClientEvent> = EventBus::new(8);
        let hb = Heartbeat::new(
            bus,
            "127.0.0.1".to_string(),
            http_port,
            socks_port,
            "alice-mac".to_string(),
            "0.2.0".to_string(),
        );
        hb.clone().start().await;

        // 等待 mock server 收到 heartbeat HTTP 请求;最多等 5s。
        let mut got_heartbeat = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Some(req)) if req.starts_with("GET /api/clients/heartbeat?") => {
                    assert!(req.contains("name=alice-mac"), "{req}");
                    assert!(req.contains("version=0.2.0"), "{req}");
                    got_heartbeat = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }
        hb.stop().await;
        assert!(got_heartbeat, "Heartbeat did not notify server within 5s");
    }
}
