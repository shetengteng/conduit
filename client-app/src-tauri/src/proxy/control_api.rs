//! `control_api` —— client-app loopback HTTP 控制面。
//!
//! UI 通过 `http://127.0.0.1:{api_port}` 访问。所有响应字段与
//! `client-app/ui/src/types/client.ts` 严格对齐 (snake_case)。
//!
//! | Method | Path                              | 说明                             |
//! |--------|-----------------------------------|----------------------------------|
//! | GET    | `/healthz`                        | 进程探活 (HealthzResponse)       |
//! | GET    | `/api/connection`                 | 当前连接快照                     |
//! | GET    | `/api/servers`                    | mDNS + 历史 server 列表          |
//! | POST   | `/api/servers/forget`             | 从历史移除单条                   |
//! | POST   | `/api/servers/forget_all`         | 清空历史                         |
//! | POST   | `/api/connect/{server_id}`        | 触发 5 步连接                    |
//! | POST   | `/api/disconnect`                 | 断开                             |
//! | GET    | `/api/traffic`                    | TrafficSnapshot                  |
//! | GET    | `/api/cache?direction&source&limit` | RouteCacheResponse           |
//! | DELETE | `/api/cache`                      | 清空路由缓存                     |
//! | GET    | `/api/diagnose`                   | 诊断快照                         |
//! | GET    | `/api/events`                     | SSE: 所有 ClientEvent forward    |
//!
//! ## v0.2.3 起改用 axum 0.8 + tower-http
//!
//! 之前 ~700 行手写 HTTP/1.1 解析+ 路由分发 + CORS preflight + percent-decode +
//! SSE 帧拼装,现在 axum 替换后只剩 wire schema + handler 业务逻辑。
//! 详见 `design/2026-05-07-1-Conduit-第三方库替换计划.md` 阶段 3。

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use conduit_core::time::epoch_secs;
use conduit_core::{DiscoveredServer, RouteDirection};
use log::{debug, info};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::cors::CorsLayer;

use super::core::{ClientCore, ClientEvent};

#[derive(Clone)]
struct AppState {
    core: Arc<ClientCore>,
}

/// 启动 control_api,loopback 监听。返回实际监听端口。
pub async fn start(core: Arc<ClientCore>, bind_port: u16) -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", bind_port))
        .await
        .map_err(|e| format!("control_api bind failed: {e}"))?;
    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(bind_port);
    info!("[control_api] listening on http://127.0.0.1:{actual}");

    let cancel = core.cancel_token();
    let state = AppState { core: core.clone() };
    let app = Router::new()
        .route("/healthz", get(serve_healthz))
        .route("/api/connection", get(serve_connection))
        .route("/api/servers", get(serve_servers))
        .route("/api/servers/forget", post(serve_servers_forget))
        .route("/api/servers/forget_all", post(serve_servers_forget_all))
        .route("/api/connect/{server_id}", post(serve_connect))
        .route("/api/disconnect", post(serve_disconnect))
        .route("/api/traffic", get(serve_traffic))
        .route("/api/cache", get(serve_cache_get).delete(serve_cache_clear))
        .route("/api/diagnose", get(serve_diagnose))
        .route("/api/events", get(serve_events))
        .layer(CorsLayer::very_permissive())
        .fallback(not_found)
        .with_state(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await;
    });
    Ok(actual)
}

// ─────────────────────────────────────────────────────────────────────────────
// 端点 handler
// ─────────────────────────────────────────────────────────────────────────────

async fn serve_healthz(State(s): State<AppState>) -> Json<Value> {
    let uptime = epoch_secs() - s.core.started_at();
    Json(json!({
        "ready": true,
        "checks": [
            {"name": "process", "ok": true, "detail": "running"},
        ],
        "uptime_sec": uptime.max(0.0),
    }))
}

async fn serve_connection(State(s): State<AppState>) -> Json<Value> {
    let snap = s.core.connection_snapshot().await;
    Json(serde_json::to_value(&snap).unwrap_or_default())
}

async fn serve_servers(State(s): State<AppState>) -> Json<Value> {
    let list = s.core.list_servers().await;
    let wire: Vec<Value> = list.iter().map(discovered_to_wire).collect();
    Json(json!({
        "count": wire.len(),
        "available": true,
        "servers": wire,
    }))
}

#[derive(Deserialize)]
struct ForgetRequest {
    server_id: String,
}

async fn serve_servers_forget(
    State(s): State<AppState>,
    Json(req): Json<ForgetRequest>,
) -> Result<Json<Value>, ApiError> {
    let removed = s.core.discoverer().forget(&req.server_id).await;
    Ok(Json(json!({
        "ok": true,
        "removed": removed,
        "server_id": req.server_id,
    })))
}

async fn serve_servers_forget_all(State(s): State<AppState>) -> Json<Value> {
    let n = s.core.discoverer().forget_all().await;
    Json(json!({ "ok": true, "removed_count": n }))
}

async fn serve_connect(
    State(s): State<AppState>,
    Path(server_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // axum 0.8 Path<String> 自动 percent-decode,无需手 url_decode。
    let server = match s.core.discoverer().get_by_id(&server_id).await {
        Some(s) => s,
        None => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                "server not in registry",
            ));
        }
    };
    match s.core.connect_to(server).await {
        Ok(snap) => Ok(Json(serde_json::to_value(&snap).unwrap_or_default())),
        Err(e) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "CONNECT_FAILED",
            &e,
        )),
    }
}

async fn serve_disconnect(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    match s.core.disconnect().await {
        Ok(snap) => {
            let mut v = serde_json::to_value(&snap).unwrap_or_default();
            if let Value::Object(ref mut obj) = v {
                obj.insert("ok".into(), Value::Bool(true));
            }
            Ok(Json(v))
        }
        Err(e) => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DISCONNECT_FAILED",
            &e,
        )),
    }
}

async fn serve_traffic(State(s): State<AppState>) -> Json<Value> {
    let (sent, recv) = s.core.traffic_snapshot();
    Json(json!({
        "ts": epoch_secs(),
        "uplink_bytes": 0u64,
        "downlink_bytes": 0u64,
        "total_uplink": sent,
        "total_downlink": recv,
    }))
}

#[derive(Deserialize, Default)]
struct CacheQuery {
    direction: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
}

async fn serve_cache_get(
    State(s): State<AppState>,
    Query(q): Query<CacheQuery>,
) -> Json<Value> {
    let snap = s.core.cache().snapshot();
    let direction_filter: Option<RouteDirection> = q.direction.as_deref().and_then(|d| match d {
        "direct" => Some(RouteDirection::Direct),
        "proxy" => Some(RouteDirection::Proxy),
        _ => None,
    });
    let source_filter = q.source.clone();

    let mut entries: Vec<_> = snap
        .into_iter()
        .filter(|e| direction_filter.is_none_or(|d| d == e.direction))
        .filter(|e| source_filter.as_ref().is_none_or(|s| s == &e.source))
        .collect();
    if let Some(n) = q.limit {
        entries.truncate(n);
    }

    let total = entries.len();
    let mut direct_count = 0;
    let mut proxy_count = 0;
    let mut by_source: std::collections::HashMap<String, u32> = Default::default();
    for e in entries.iter() {
        match e.direction {
            RouteDirection::Direct => direct_count += 1,
            RouteDirection::Proxy => proxy_count += 1,
        }
        *by_source.entry(e.source.clone()).or_insert(0) += 1;
    }
    let now = epoch_secs();
    let entries_json: Vec<Value> = entries
        .iter()
        .map(|e| {
            let ttl = (e.expires_at - now).max(0.0);
            json!({
                "host": e.host,
                "direction": e.direction,
                "source": e.source,
                "hit_count": e.hit_count,
                "expires_at": iso8601(e.expires_at),
                "last_used": iso8601(now),
                "ttl_remaining_sec": ttl,
            })
        })
        .collect();
    Json(json!({
        "count": entries_json.len(),
        "total": total,
        "stats": {
            "total": total,
            "direct_count": direct_count,
            "proxy_count": proxy_count,
            "expired_count": 0,
            "by_source": by_source,
            "hits": 0,
            "misses": 0,
            "evictions": 0,
        },
        "entries": entries_json,
    }))
}

async fn serve_cache_clear(State(s): State<AppState>) -> Json<Value> {
    let n = s.core.cache().clear();
    Json(json!({ "ok": true, "removed": n }))
}

async fn serve_diagnose(State(s): State<AppState>) -> Json<Value> {
    let snap = s.core.connection_snapshot().await;
    let mdns_available = !s.core.list_servers().await.is_empty();
    let server_reach = matches!(snap.state, conduit_core::ConnectionState::Connected);
    let sys_proxy = snap.system_proxy_active;
    let checks = vec![
        json!({
            "key": "sidecar",
            "label": "进程",
            "ok": true,
            "detail": "in-process Rust runtime",
            "remediation": null,
        }),
        json!({
            "key": "mdns",
            "label": "mDNS 发现",
            "ok": true,
            "detail": if mdns_available { "found servers" } else { "no servers yet" },
            "remediation": null,
        }),
        json!({
            "key": "server_reach",
            "label": "Server 可达",
            "ok": server_reach,
            "detail": if server_reach { "connected" } else { "not connected" },
            "remediation": if server_reach { Value::Null } else { Value::String("先选择 server 并连接".into()) },
        }),
        json!({
            "key": "pac",
            "label": "PAC 拉取",
            "ok": server_reach,
            "detail": if server_reach { "fetched at connect" } else { "n/a" },
            "remediation": null,
        }),
        json!({
            "key": "system_proxy",
            "label": "系统代理",
            "ok": sys_proxy || !server_reach,
            "detail": if sys_proxy { "macOS networksetup applied" } else { "off" },
            "remediation": null,
        }),
    ];
    let ok = checks.iter().all(|c| c["ok"].as_bool().unwrap_or(false));
    Json(json!({
        "ok": ok,
        "checks": checks,
        "checked_at": epoch_secs(),
    }))
}

/// SSE forwarder: 把 [`EventBus<ClientEvent>`] 的事件以 named SSE event 推给前端。
///
/// `data` 字段直接是事件 payload(不再包 `{ts, payload}` 外层);早期版本曾包过,
/// 导致前端 `trafficStore.onTick` 取不到 `uplink_bytes` 字段、曲线全 0,
/// 现在统一为 server-app 的格式。
async fn serve_events(
    State(s): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = s.core.bus().subscribe();
    let cancel = s.core.cancel_token();
    let stream = BroadcastStream::new(rx)
        .take_while(move |_| !cancel.is_cancelled())
        .filter_map(|res| match res {
            Ok(ClientEvent { kind, payload, ts: _ }) => {
                Some(Ok::<_, Infallible>(
                    Event::default().event(kind).data(payload.to_string()),
                ))
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                debug!("[control_api] SSE lagged {n} events");
                None
            }
        });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

type BroadcastStreamRecvError = tokio_stream::wrappers::errors::BroadcastStreamRecvError;

async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": { "code": "NOT_FOUND", "message": "unknown route" } })),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// 通用错误响应包装(`{"error":{"code","message"}}`)
// ─────────────────────────────────────────────────────────────────────────────

struct ApiError {
    status: StatusCode,
    code: String,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: &str) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(json!({
                "error": { "code": self.code, "message": self.message }
            })),
        )
            .into_response()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// helper
// ─────────────────────────────────────────────────────────────────────────────

fn discovered_to_wire(s: &DiscoveredServer) -> Value {
    let pac_url = format!("http://{}:{}{}", s.host, s.port, s.pac);
    json!({
        "server_id": s.server_id,
        "name": s.name,
        "host": s.host,
        "port": s.port,
        "socks": s.socks,
        "api": s.api,
        "vpn": s.vpn,
        "version": s.version,
        "pac": s.pac,
        "pac_url": pac_url,
        "source": s.source,
        "last_seen_at": s.last_seen_at,
        "healthy": s.healthy,
    })
}

fn iso8601(epoch: f64) -> String {
    let secs = epoch as i64;
    let nsecs = ((epoch - secs as f64) * 1e9) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsecs)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| epoch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::ClientConfig;

    fn make_core() -> Arc<ClientCore> {
        Arc::new(ClientCore::new(ClientConfig::with_ports(0, 0)))
    }

    #[tokio::test]
    async fn healthz_returns_ui_shape() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/healthz"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["ready"], true);
        assert!(v["checks"].is_array());
        assert!(v["uptime_sec"].as_f64().unwrap() >= 0.0);
    }

    #[tokio::test]
    async fn connection_endpoint_returns_idle_initial() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/connection"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["state"], "idle");
        assert!(v["server"].is_null());
        assert_eq!(v["system_proxy_active"], false);
    }

    #[tokio::test]
    async fn servers_endpoint_returns_count_and_available() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/servers"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["count"], 0);
        assert_eq!(v["available"], true);
        assert!(v["servers"].is_array());
    }

    #[tokio::test]
    async fn traffic_endpoint_returns_totals_and_ts() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/traffic"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(v["ts"].as_f64().unwrap() > 1_700_000_000.0);
        assert_eq!(v["total_uplink"], 0);
        assert_eq!(v["total_downlink"], 0);
    }

    #[tokio::test]
    async fn cache_endpoint_returns_stats_and_entries() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/cache"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(v["entries"].is_array());
        assert!(v["stats"].is_object());
        assert_eq!(v["count"], 0);
    }

    #[tokio::test]
    async fn diagnose_endpoint_returns_5_checks() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/diagnose"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["checks"].as_array().unwrap().len(), 5);
        assert!(v["checked_at"].as_f64().unwrap() > 1_700_000_000.0);
    }

    #[tokio::test]
    async fn forget_all_returns_removed_count() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let v: Value = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/api/servers/forget_all"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["removed_count"], 0);
    }

    #[tokio::test]
    async fn connect_to_unknown_returns_404() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let resp = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{port}/api/connect/unknown%40host"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn unknown_route_returns_404_envelope() {
        let core = make_core();
        let port = start(core.clone(), 0).await.unwrap();
        let resp = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/no/such/route"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        let v: Value = resp.json().await.unwrap();
        assert_eq!(v["error"]["code"], "NOT_FOUND");
    }
}
