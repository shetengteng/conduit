//! Control API server —— 兼容 UI 现有的 `/api/*` REST + `/api/events` SSE。
//!
//! 这是 W2 Sprint 2 的 S2.7 阶段产物:让 server-app/ui 现有的所有
//! `apiGet("/api/...")` / SSE 调用在 sidecar 删除之后**继续工作**,
//! 不强迫前端立刻切到 Tauri `invoke`。等 W3 / W4 有空闲时再做 IPC 平移。
//!
//! 仅在 `127.0.0.1:cfg.api_port` 监听 (loopback only),UI / 内网监控脚本访问。
//!
//! ## v0.2.3 起改用 axum 0.8 + tower-http
//!
//! 之前手写 HTTP/1.1 解析 + 路由分发 + CORS preflight,~400 行。
//! 现在 axum + CorsLayer + Sse 把基础设施全部下沉,本文件只剩 wire schema +
//! handler 业务逻辑。详见 `design/2026-05-07-1-Conduit-第三方库替换计划.md` 阶段 3。
//!
//! 覆盖 endpoint (最小集合,与 `server-app/ui/src/api/server.ts` 完全对齐):
//! - `GET /healthz` → [`HealthzResponse`] (5 项 named check)
//! - `GET /api/status` → [`ServerStatusOut`]
//! - `GET /api/clients` → [`ClientsResponse`]
//! - `GET /api/traffic?window=N&peer=X` → [`TrafficResponse`]
//!   (历史窗口暂时为空,仅占位让 UI 不崩;流量数据走 SSE)
//! - `POST /api/admin/stop` → 200 + 触发 ProxyCore 取消 + 进程退出
//! - `GET /api/events` → SSE,从 [`EventBus<ServerEvent>`] 转发
//!
//! `OPTIONS *` 由 `tower_http::cors::CorsLayer::very_permissive()` 自动响应。

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use conduit_core::time::epoch_secs;
use log::{debug, info};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;

use super::core::{ProxyCore, ServerEvent};

#[derive(Clone)]
struct AppState {
    core: ProxyCore,
    cancel: CancellationToken,
}

/// 运行 control API server,监听 `127.0.0.1:cfg.api_port`,直到 cancel-token 触发。
pub async fn run(core: ProxyCore, cancel: CancellationToken) -> std::io::Result<()> {
    let cfg = core.config();
    let bind = format!("127.0.0.1:{}", cfg.api_port);
    let listener = TcpListener::bind(&bind).await?;
    info!("[ctl] control api listening on {bind}");

    let state = AppState {
        core,
        cancel: cancel.clone(),
    };
    let app = Router::new()
        .route("/healthz", get(serve_healthz))
        .route("/api/status", get(serve_status))
        .route("/api/clients", get(serve_clients))
        .route("/api/traffic", get(serve_traffic))
        .route("/api/admin/stop", post(serve_admin_stop))
        .route("/api/events", get(serve_events))
        // loopback only,所有 origin/method/header 全放行;tower-http 0.6
        // 提供 very_permissive() 与 axum 0.8 完全配套。
        .layer(CorsLayer::very_permissive())
        .fallback(not_found)
        .with_state(state);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await
}

// ─────────────────────────────────────────────────────────────────────────────
// 端点 handler
// ─────────────────────────────────────────────────────────────────────────────

async fn serve_healthz(State(s): State<AppState>) -> Json<HealthzResponse> {
    let cfg = s.core.config();
    let status = s.core.status().await;
    // detail 反映**真实 bind**(http/socks5 默认 0.0.0.0,api 固定 127.0.0.1),
    // 而不是之前硬编码的 127.0.0.1 —— 之前展示给用户的"端口监听 listening on 127.0.0.1"
    // 与实际"任何网卡都接受"严重不符。
    let proxy_bind = if cfg.bind.is_empty() {
        "0.0.0.0"
    } else {
        cfg.bind.as_str()
    };
    let lan_hint = if proxy_bind == "0.0.0.0" {
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
    Json(resp)
}

async fn serve_status(State(s): State<AppState>) -> Json<ServerStatusOut> {
    let cfg = s.core.config();
    let inner = s.core.status().await;
    let host = super::effective_advertised_host(&cfg);
    let pac_url = format!("http://{}:{}/proxy.pac", host, cfg.http_port);
    let mdns_name = if cfg.mdns_service_name.is_empty() {
        super::mdns::detect_hostname()
    } else {
        cfg.mdns_service_name.clone()
    };
    Json(ServerStatusOut {
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
            // v0.2.2 起由 vpn_detect 走 `netdev::get_default_interface()` 真实
            // 判定: default route 出接口若属 Tunnel/Ppp/is_tun 则为 true。
            default_route_via_vpn: inner.default_route_via_vpn,
        },
        // LAN 出口: host=="127.0.0.1" 表示没探测到 LAN 网卡,这种情况把
        // available 翻 false,让前端"已检测/未检测"徽标如实切换。
        lan: LanStatus {
            available: host != "127.0.0.1",
            detail: Some(host.clone()),
        },
        clients_count: inner.active_sessions,
        passive_clients_count: s.core.sessions().passive_count().await,
        uptime_sec: inner.uptime_sec,
        ready: inner.running,
    })
}

async fn serve_clients(State(s): State<AppState>) -> Json<ClientsResponse> {
    let sessions = s.core.sessions();
    let active = sessions.snapshot().await;
    let passives = sessions.passive_clients().await;
    let now = epoch_secs();
    Json(ClientsResponse {
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
    })
}

/// 历史 traffic 时间窗口聚合当前未实装(设计上排在 DIRECT-first 路由特性之后)。
/// 先返回 200 + 空 series 让 UI 不崩;实时数据通过 `traffic_tick` SSE event 流推。
async fn serve_traffic() -> Json<TrafficResponse> {
    Json(TrafficResponse {
        window_sec: 60,
        now: epoch_secs(),
        series: serde_json::json!({}),
    })
}

async fn serve_admin_stop(State(s): State<AppState>) -> Json<serde_json::Value> {
    info!("[ctl] /api/admin/stop received, cancelling ProxyCore");
    s.cancel.cancel();
    s.core.stop().await;
    Json(serde_json::json!({ "ok": true }))
}

/// SSE forwarder:把 [`EventBus<ServerEvent>`] 的事件翻译成 UI 期望的格式。
///
/// 输出 named event,`data` 字段直接是 payload JSON(不再包 `{type, payload}` 外层),
/// 与 client-app `stream_events` 一致。`KeepAlive::default()` 周期发 `:` 注释心跳。
async fn serve_events(
    State(s): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = s.core.event_bus().subscribe();
    let cancel = s.cancel.clone();
    let stream = BroadcastStream::new(rx)
        .take_while(move |_| !cancel.is_cancelled())
        .filter_map(|res| match res {
            Ok(evt) => Some(broadcast_event_to_sse(evt)),
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                debug!("[ctl] sse subscriber lagged {n} events");
                None
            }
        });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// `BroadcastStream` 的别名,避免上面 import 拉满。
type BroadcastStreamRecvError = tokio_stream::wrappers::errors::BroadcastStreamRecvError;

fn broadcast_event_to_sse(evt: ServerEvent) -> Result<Event, Infallible> {
    // serde_json::Value Display 是 compact JSON,适合直接做 SSE data 字段
    let payload = evt.payload.to_string();
    Ok(Event::default().event(evt.kind).data(payload))
}

async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": { "code": "NOT_FOUND", "message": "unknown route" } })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Wire types (与 server-app/ui/src/types/proxy.ts 严格对齐)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{ProxyConfig, ProxyCore};
    use std::time::Duration as StdDuration;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
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
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let raw = String::from_utf8_lossy(&buf).into_owned();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        (head.to_string(), body.to_string())
    }

    async fn http_post(api_port: u16, path: &str) -> String {
        let mut s = TS::connect(("127.0.0.1", api_port)).await.unwrap();
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// chunked body 在 axum 输出中很常见(默认 Transfer-Encoding: chunked),
    /// 不能简单按"\r\n\r\n 后即 body"提取,需要走 `\r\n\r\n` 后再按 chunked 反解。
    /// 这里用更宽松的 contains 比对,而不强校验 Content-Length。
    fn body_includes(body_or_raw: &str, needle: &str) -> bool {
        body_or_raw.contains(needle)
    }

    #[tokio::test]
    async fn healthz_returns_named_checks() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/healthz").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        for expected in [
            "\"http_port\"",
            "\"socks5_port\"",
            "\"api_port\"",
            "\"lan_ip\"",
            "\"vpn_tunnel\"",
        ] {
            assert!(
                body_includes(&body, expected),
                "missing {expected} in body: {body}"
            );
        }
        core.stop().await;
    }

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
        assert!(
            body.contains(&format!("listening on 0.0.0.0:{http}")),
            "http_port detail should reflect 0.0.0.0 bind: {body}"
        );
        assert!(
            body.contains(&format!("listening on 0.0.0.0:{socks}")),
            "socks5_port detail should reflect 0.0.0.0 bind: {body}"
        );
        assert!(
            body.contains(&format!(
                "listening on 127.0.0.1:{api} (loopback only)"
            )),
            "api_port detail should be loopback only: {body}"
        );
        core.stop().await;
    }

    #[tokio::test]
    async fn status_returns_ui_compatible_shape() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/api/status").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        for k in [
            "\"running\"",
            "\"version\"",
            "\"http_port\"",
            "\"socks5_port\"",
            "\"api_port\"",
            "\"pac_url\"",
            "\"mdns\"",
            "\"vpn\"",
            "\"lan\"",
            "\"clients_count\"",
            "\"passive_clients_count\"",
            "\"uptime_sec\"",
            "\"ready\"",
        ] {
            assert!(body_includes(&body, k), "field {k} missing in {body}");
        }
        assert!(body.contains("_conduit._tcp.local."));
        core.stop().await;
    }

    #[tokio::test]
    async fn clients_returns_empty_lists_when_no_traffic() {
        let (core, api) = start_test_core_with_ctl().await;
        let (head, body) = http_get(api, "/api/clients").await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(body.contains("\"count\":0"));
        assert!(body.contains("\"passive_count\":0"));
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
