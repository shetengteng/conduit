//! Control API server —— 兼容 UI 现有的 `/api/*` REST + `/api/events` SSE。
//!
//! 这是 W2 Sprint 2 的 S2.7 阶段产物：让 server-app/ui 现有的所有
//! `apiGet("/api/...")` / SSE 调用在 sidecar 删除之后**继续工作**，
//! 不强迫前端立刻切到 Tauri `invoke`。等 W3 / W4 有空闲时再做 IPC 平移。
//!
//! 仅在 `127.0.0.1:cfg.api_port` 监听（loopback only），UI / 内网监控脚本访问。
//!
//! 覆盖 endpoint（最小集合，与 `server-app/ui/src/api/server.ts` 完全对齐）：
//! - `GET /healthz` → [`HealthzResponse`]（5 项 named check 占位）
//! - `GET /api/status` → [`ServerStatusOut`]
//! - `GET /api/clients` → [`ClientsResponse`]
//! - `GET /api/traffic?window=N&peer=X` → [`TrafficResponse`]
//!   （历史窗口暂时为空，仅占位让 UI 不崩；流量数据走 SSE）
//! - `POST /api/admin/stop` → 200 + 触发 ProxyCore 取消 + 进程退出
//! - `GET /api/events` → SSE，从 [`EventBus<ServerEvent>`] 转发

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{debug, info, warn};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use super::core::ProxyCore;

/// 运行 control API server，监听 `127.0.0.1:cfg.api_port`，直到 cancel-token 触发。
pub async fn run(core: ProxyCore, cancel: CancellationToken) -> std::io::Result<()> {
    let cfg = core.config();
    let bind = format!("127.0.0.1:{}", cfg.api_port);
    let listener = TcpListener::bind(&bind).await?;
    info!("[ctl] control api listening on {bind}");
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("[ctl] cancel requested, accept loop exiting");
                break;
            }
            res = listener.accept() => {
                let (sock, _peer) = match res {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("[ctl] accept error: {e}");
                        continue;
                    }
                };
                let core = core.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_request(sock, core, cancel).await {
                        debug!("[ctl] connection error: {e}");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle_request(
    stream: TcpStream,
    core: ProxyCore,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::with_capacity(8 * 1024, read_half);

    let mut request_line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut request_line))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "ctl request line"))??;
    if n == 0 {
        return Ok(());
    }
    let trimmed = request_line.trim_end_matches(['\r', '\n']);
    let mut it = trimmed.splitn(3, ' ');
    let (method, target) = match (it.next(), it.next()) {
        (Some(m), Some(t)) => (m.to_uppercase(), t.to_string()),
        _ => {
            send_json_status(&mut write_half, 400, "Bad Request", b"{}", false).await?;
            return Ok(());
        }
    };
    drain_headers(&mut reader).await.ok();

    let path_only = target.split('?').next().unwrap_or("").to_string();

    // CORS preflight：webview 从 vite dev origin (http://localhost:1420) fetch
    // 跨源到 127.0.0.1:api_port 时,某些 fetch (带自定义 headers / 非 simple method)
    // 会先发 OPTIONS。直接 204 返回放行所有 method/header,loopback only 不存在
    // 安全风险。simple GET 也需要响应里带 Allow-Origin,所以 send_json_status 总是带。
    if method == "OPTIONS" {
        send_cors_preflight(&mut write_half).await?;
        return Ok(());
    }

    match (method.as_str(), path_only.as_str()) {
        ("GET", "/healthz") => serve_healthz(&mut write_half, &core).await?,
        ("GET", "/api/status") => serve_status(&mut write_half, &core).await?,
        ("GET", "/api/clients") => serve_clients(&mut write_half, &core).await?,
        ("GET", "/api/traffic") => serve_traffic(&mut write_half).await?,
        ("POST", "/api/admin/stop") => serve_admin_stop(&mut write_half, &core, &cancel).await?,
        ("GET", "/api/events") => serve_events(write_half, &core, &cancel).await?,
        _ => {
            let body = b"{\"error\":{\"code\":\"NOT_FOUND\",\"message\":\"unknown route\"}}";
            send_json_status(&mut write_half, 404, "Not Found", body, false).await?;
        }
    }
    Ok(())
}

/// 响应 CORS preflight (OPTIONS)。loopback only API,放行所有 origin/method/header。
async fn send_cors_preflight<W: AsyncWriteExt + Unpin>(out: &mut W) -> std::io::Result<()> {
    let head = "HTTP/1.1 204 No Content\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Access-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\n\
        Access-Control-Allow-Headers: Content-Type, Accept\r\n\
        Access-Control-Max-Age: 3600\r\n\
        Connection: close\r\n\r\n";
    out.write_all(head.as_bytes()).await?;
    out.flush().await
}

// ─────────────────────────────────────────────────────────────────────────────
// 端点处理器
// ─────────────────────────────────────────────────────────────────────────────

async fn serve_healthz<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    core: &ProxyCore,
) -> std::io::Result<()> {
    let cfg = core.config();
    let status = core.status().await;
    // detail 反映**真实 bind**(http/socks5 默认 0.0.0.0,api 固定 127.0.0.1),
    // 而不是之前硬编码的 127.0.0.1 —— 之前展示给用户的"端口监听 listening on 127.0.0.1"
    // 与实际"任何网卡都接受"严重不符。
    let proxy_bind = if cfg.bind.is_empty() {
        "0.0.0.0"
    } else {
        cfg.bind.as_str()
    };
    let lan_hint = if proxy_bind == "0.0.0.0" {
        // 通配监听 → 顺手把 LAN IP 也告诉用户,直观说明"同事实际拨过来用什么 IP"
        let host = super::effective_advertised_host(&cfg);
        if host != "127.0.0.1" {
            format!(" (LAN: {host})")
        } else {
            " (loopback fallback, no LAN iface detected)".into()
        }
    } else if proxy_bind == "127.0.0.1" {
        " (loopback only)".into()
    } else {
        String::new()
    };
    let resp = HealthzResponse {
        ready: status.running,
        running: status.running,
        uptime_sec: status.uptime_sec,
        checks: vec![
            HealthCheckEntry {
                name: "http_port".into(),
                ok: status.running,
                detail: format!("listening on {proxy_bind}:{}{lan_hint}", cfg.http_port),
            },
            HealthCheckEntry {
                name: "socks5_port".into(),
                ok: status.running,
                detail: format!("listening on {proxy_bind}:{}{lan_hint}", cfg.socks_port),
            },
            HealthCheckEntry {
                name: "api_port".into(),
                ok: true,
                detail: format!("listening on 127.0.0.1:{} (loopback only)", cfg.api_port),
            },
            HealthCheckEntry {
                name: "lan_ip".into(),
                ok: true,
                detail: "control_api always reports ok (advisory)".into(),
            },
            HealthCheckEntry {
                name: "vpn_tunnel".into(),
                ok: status.vpn_on,
                detail: if status.vpn_on {
                    "vpn iface detected".into()
                } else {
                    "no utun/ppp/tun detected".into()
                },
            },
        ],
    };
    let body = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
    send_json_status(out, 200, "OK", &body, false).await
}

async fn serve_status<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    core: &ProxyCore,
) -> std::io::Result<()> {
    let cfg = core.config();
    let inner = core.status().await;
    let host = super::effective_advertised_host(&cfg);
    let pac_url = format!("http://{}:{}/proxy.pac", host, cfg.http_port);
    let mdns_name = if cfg.mdns_service_name.is_empty() {
        super::mdns::detect_hostname()
    } else {
        cfg.mdns_service_name.clone()
    };
    let resp = ServerStatusOut {
        running: inner.running,
        version: env!("CARGO_PKG_VERSION").to_string(),
        http_port: inner.http_port,
        socks5_port: inner.socks_port,
        api_port: inner.api_port,
        pac_url: Some(pac_url),
        mdns: MdnsStatus {
            enabled: inner.mdns_enabled,
            name: mdns_name,
            service_type: conduit_core::mdns::SERVICE_TYPE.to_string(),
        },
        vpn: VpnStatus {
            available: inner.vpn_on,
            iface: inner.vpn_iface.clone(),
            // 是否真正走默认路由需要 `route -n get default` 解析。当前阶段
            // 用 vpn_on 近似——有 VPN 接口就视为可用，UI 仅用 available 字段
            // 切徽标，不依赖 default_route_via_vpn 做硬决策。
            default_route_via_vpn: inner.vpn_on,
        },
        lan: LanStatus {
            available: true,
            detail: None,
        },
        clients_count: inner.active_sessions,
        passive_clients_count: core.sessions().passive_count().await,
        uptime_sec: inner.uptime_sec,
        ready: inner.running,
    };
    let body = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
    send_json_status(out, 200, "OK", &body, false).await
}

async fn serve_clients<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    core: &ProxyCore,
) -> std::io::Result<()> {
    let sessions = core.sessions();
    let active = sessions.snapshot().await;
    let passives = sessions.passive_clients().await;
    let now = epoch_secs();
    let resp = ClientsResponse {
        count: active.len(),
        clients: active
            .iter()
            .map(|c| ClientSession {
                session_id: c.session_id.clone(),
                peer_ip: c.peer_ip.clone(),
                proto: c.proto.to_string(),
                target: c.target.clone(),
                since: c.since,
                last_seen: c.last_seen,
                sent_bytes: c.sent_bytes,
                recv_bytes: c.recv_bytes,
            })
            .collect(),
        passive_count: passives.len(),
        passive_clients: passives
            .iter()
            .map(|p| PassiveClientOut {
                peer_ip: p.peer_ip.clone(),
                client_name: p.client_name.clone(),
                version: p.version.clone(),
                first_seen: p.first_seen,
                last_seen: p.last_seen,
                idle_sec: (now - p.last_seen).max(0.0) as u64,
            })
            .collect(),
    };
    let body = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
    send_json_status(out, 200, "OK", &body, false).await
}

/// 历史 traffic 时间窗口聚合当前未实装（设计上排在 DIRECT-first 路由特性之后）。
/// 先返回 200 + 空 series 让 UI 不崩；实时数据通过 `traffic_tick`
/// SSE event 流推。
async fn serve_traffic<W: AsyncWriteExt + Unpin>(out: &mut W) -> std::io::Result<()> {
    let resp = TrafficResponse {
        window_sec: 60,
        now: epoch_secs(),
        series: serde_json::json!({}),
    };
    let body = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
    send_json_status(out, 200, "OK", &body, false).await
}

async fn serve_admin_stop<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    core: &ProxyCore,
    cancel: &CancellationToken,
) -> std::io::Result<()> {
    info!("[ctl] /api/admin/stop received, cancelling ProxyCore");
    cancel.cancel();
    core.stop().await;
    let body = b"{\"ok\":true}";
    send_json_status(out, 200, "OK", body, false).await?;
    // 告诉外层 lib.rs 退出（cancel-token 已触发 mDNS / HTTP / SOCKS5 退出）
    Ok(())
}

/// SSE forwarder：把 [`EventBus<ServerEvent>`] 的事件翻译成 UI 期望的格式。
///
/// 输出格式（named SSE event，与 client-app `stream_events` 一致）：
/// ```text
/// event: vpn_state_changed
/// data: {"available":true,"iface":"utun5"}
///
/// ```
///
/// 前端 `useEvents` 用 `addEventListener("vpn_state_changed", ...)` 监听
/// named event，data 直接是 payload JSON（不再包 `{type, payload}` 外层）。
async fn serve_events(
    mut out: tokio::net::tcp::OwnedWriteHalf,
    core: &ProxyCore,
    cancel: &CancellationToken,
) -> std::io::Result<()> {
    let head = b"HTTP/1.1 200 OK\r\n\
        Content-Type: text/event-stream\r\n\
        Cache-Control: no-cache\r\n\
        Connection: keep-alive\r\n\
        Access-Control-Allow-Origin: *\r\n\r\n";
    out.write_all(head).await?;
    out.write_all(b": connected\n\n").await?;
    out.flush().await?;

    let mut rx = core.event_bus().subscribe();
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    tick.tick().await; // 首次立即触发，跳过

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            res = rx.recv() => match res {
                Ok(evt) => {
                    // serde_json::Value Display 是 compact JSON，可直接放进 SSE 帧
                    let line = format!("event: {}\ndata: {}\n\n", evt.kind, evt.payload);
                    if out.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    if out.flush().await.is_err() { break; }
                }
                Err(RecvError::Lagged(n)) => {
                    debug!("[ctl] sse subscriber lagged {n} events");
                }
                Err(RecvError::Closed) => break,
            },
            _ = tick.tick() => {
                // SSE keepalive comment（不会被 EventSource 当作 message）
                if out.write_all(b": keepalive\n\n").await.is_err() { break; }
                if out.flush().await.is_err() { break; }
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Wire types（与 server-app/ui/src/types/proxy.ts 严格对齐）
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthzResponse {
    ready: bool,
    checks: Vec<HealthCheckEntry>,
    running: bool,
    uptime_sec: f64,
}

#[derive(Serialize)]
struct HealthCheckEntry {
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Serialize)]
struct ServerStatusOut {
    running: bool,
    version: String,
    http_port: u16,
    socks5_port: u16,
    api_port: u16,
    pac_url: Option<String>,
    mdns: MdnsStatus,
    vpn: VpnStatus,
    lan: LanStatus,
    clients_count: usize,
    passive_clients_count: usize,
    uptime_sec: f64,
    ready: bool,
}

#[derive(Serialize)]
struct MdnsStatus {
    enabled: bool,
    name: String,
    service_type: String,
}

#[derive(Serialize)]
struct VpnStatus {
    available: bool,
    iface: Option<String>,
    default_route_via_vpn: bool,
}

#[derive(Serialize)]
struct LanStatus {
    available: bool,
    detail: Option<String>,
}

#[derive(Serialize)]
struct ClientsResponse {
    count: usize,
    clients: Vec<ClientSession>,
    passive_count: usize,
    passive_clients: Vec<PassiveClientOut>,
}

#[derive(Serialize)]
struct ClientSession {
    session_id: String,
    peer_ip: String,
    proto: String,
    target: String,
    since: f64,
    last_seen: f64,
    sent_bytes: u64,
    recv_bytes: u64,
}

#[derive(Serialize)]
struct PassiveClientOut {
    peer_ip: String,
    client_name: String,
    version: String,
    first_seen: f64,
    last_seen: f64,
    idle_sec: u64,
}

#[derive(Serialize)]
struct TrafficResponse {
    window_sec: u64,
    now: f64,
    series: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP 字节级 helper —— 不引 hyper/axum，手写报文够用
// ─────────────────────────────────────────────────────────────────────────────

async fn drain_headers(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> std::io::Result<()> {
    let mut line = String::new();
    let mut total = 0usize;
    loop {
        line.clear();
        let n = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "ctl drain headers"))??;
        if n == 0 {
            return Ok(());
        }
        total += n;
        if total > 64 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "ctl headers too large",
            ));
        }
        if line == "\r\n" || line == "\n" {
            return Ok(());
        }
    }
}

async fn send_json_status<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    status: u16,
    reason: &str,
    body: &[u8],
    keep_alive: bool,
) -> std::io::Result<()> {
    let conn = if keep_alive { "keep-alive" } else { "close" };
    // 始终带 CORS Allow-Origin: *,因为 loopback only,任何 origin 都安全。
    // 没有它 webview (vite dev: http://localhost:1420) 跨源 fetch 会被同源策略拦。
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: {conn}\r\n\
         Cache-Control: no-store\r\n\
         Access-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    out.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        out.write_all(body).await?;
    }
    out.flush().await
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
    use crate::proxy::{ProxyConfig, ProxyCore};
    use std::time::Duration as StdDuration;
    use tokio::io::AsyncReadExt as _;
    use tokio::net::TcpStream as TS;

    async fn start_test_core_with_ctl() -> (ProxyCore, u16) {
        let http = portpicker::pick_unused_port().unwrap();
        let socks = portpicker::pick_unused_port().unwrap();
        let api = portpicker::pick_unused_port().unwrap();
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        cfg.mdns_enabled = false; // 测试时不广播
        let core = ProxyCore::new(cfg);
        let cancel = core.cancel_token();
        let core_clone = core.clone();
        tokio::spawn(async move {
            let _ = super::run(core_clone, cancel).await;
        });
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", api)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        (core, api)
    }

    async fn http_get(api_port: u16, path: &str) -> (String, String) {
        let mut s = TS::connect(("127.0.0.1", api_port)).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let raw = String::from_utf8_lossy(&buf).into_owned();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        (head.to_string(), body.to_string())
    }

    async fn http_post(api_port: u16, path: &str) -> String {
        let mut s = TS::connect(("127.0.0.1", api_port)).await.unwrap();
        let req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[tokio::test]
    async fn healthz_returns_named_checks() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/healthz").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json["checks"].as_array().unwrap().len() >= 5);
        let names: Vec<&str> = json["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        for expected in ["http_port", "socks5_port", "api_port", "lan_ip", "vpn_tunnel"] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }
        core.stop().await;
    }

    /// http/socks5 detail 必须如实反映 cfg.bind(默认 0.0.0.0,而不是历史硬编码
    /// 的 127.0.0.1),api 始终 loopback only。
    /// 这里另起一个 core 用真实 default bind(0.0.0.0)。
    #[tokio::test]
    async fn healthz_detail_reflects_actual_bind() {
        let http = portpicker::pick_unused_port().unwrap();
        let socks = portpicker::pick_unused_port().unwrap();
        let api = portpicker::pick_unused_port().unwrap();
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.mdns_enabled = false;
        let core = ProxyCore::new(cfg);
        let cancel = core.cancel_token();
        let core_clone = core.clone();
        tokio::spawn(async move {
            let _ = super::run(core_clone, cancel).await;
        });
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", api)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }

        let (_, body) = http_get(api, "/healthz").await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let by_name: std::collections::HashMap<&str, &str> = json["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["name"].as_str().unwrap(), c["detail"].as_str().unwrap()))
            .collect();
        let http_detail = by_name["http_port"];
        let socks5_detail = by_name["socks5_port"];
        let api_detail = by_name["api_port"];
        assert!(
            http_detail.starts_with("listening on 0.0.0.0:"),
            "http_port detail should reflect 0.0.0.0 bind, got: {http_detail}"
        );
        assert!(
            socks5_detail.starts_with("listening on 0.0.0.0:"),
            "socks5_port detail should reflect 0.0.0.0 bind, got: {socks5_detail}"
        );
        assert!(
            api_detail.starts_with("listening on 127.0.0.1:")
                && api_detail.contains("loopback only"),
            "api_port detail should be loopback only, got: {api_detail}"
        );
        core.stop().await;
    }

    #[tokio::test]
    async fn status_returns_ui_compatible_shape() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/api/status").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        for k in [
            "running", "version", "http_port", "socks5_port", "api_port",
            "pac_url", "mdns", "vpn", "lan", "clients_count",
            "passive_clients_count", "uptime_sec", "ready",
        ] {
            assert!(!json[k].is_null(), "field {k} missing in {json}");
        }
        assert_eq!(json["mdns"]["service_type"], "_conduit._tcp.local.");
        core.stop().await;
    }

    #[tokio::test]
    async fn clients_returns_empty_lists_when_no_traffic() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/api/clients").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["count"], 0);
        assert_eq!(json["passive_count"], 0);
        assert_eq!(json["clients"].as_array().unwrap().len(), 0);
        core.stop().await;
    }

    #[tokio::test]
    async fn admin_stop_returns_ok() {
        let (core, api) = start_test_core_with_ctl().await;
        let resp = http_post(api, "/api/admin/stop").await;
        assert!(resp.contains("HTTP/1.1 200"), "{resp}");
        assert!(resp.contains("\"ok\":true"));
        core.stop().await;
    }

    #[tokio::test]
    async fn unknown_route_returns_404_envelope() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/api/nope").await;
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
        assert!(body.contains("NOT_FOUND"));
        core.stop().await;
    }
}
