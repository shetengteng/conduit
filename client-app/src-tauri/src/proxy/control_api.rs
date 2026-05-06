//! `control_api` —— client-app loopback HTTP 控制面。
//!
//! UI 通过 `http://127.0.0.1:{api_port}` 访问。所有响应字段与
//! `client-app/ui/src/types/client.ts` 严格对齐（snake_case）。
//!
//! | Method | Path                              | 说明                             |
//! |--------|-----------------------------------|----------------------------------|
//! | GET    | `/healthz`                        | 进程探活（HealthzResponse）      |
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
//! | GET    | `/api/events`                     | SSE：所有 ClientEvent forward    |
//!
//! 极简手写字节级 HTTP，匹配 server-app 风格（不引 axum）。

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{debug, info, warn};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use conduit_core::{DiscoveredServer, RouteDirection};

use super::core::{ClientCore, ClientEvent};

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEADERS_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 启动 control_api，loopback 监听。返回实际监听端口。
pub async fn start(core: Arc<ClientCore>, bind_port: u16) -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", bind_port))
        .await
        .map_err(|e| format!("control_api bind failed: {e}"))?;
    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(bind_port);
    info!("[control_api] listening on http://127.0.0.1:{actual}");
    let cancel = core.cancel_token();
    tokio::spawn(async move {
        accept_loop(listener, core, cancel).await;
    });
    Ok(actual)
}

async fn accept_loop(listener: TcpListener, core: Arc<ClientCore>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("[control_api] accept loop exiting");
                return;
            }
            res = listener.accept() => {
                match res {
                    Ok((stream, _)) => {
                        let core = core.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_one(stream, core).await {
                                debug!("[control_api] session error: {e}");
                            }
                        });
                    }
                    Err(e) => warn!("[control_api] accept error: {e}"),
                }
            }
        }
    }
}

struct ParsedRequest {
    method: String,
    path: String,
    query: String,
    content_length: usize,
    head_end: usize,
}

async fn handle_one(mut stream: TcpStream, core: Arc<ClientCore>) -> std::io::Result<()> {
    // ---- read request line + headers ----
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 1024];
    let head_end;
    loop {
        let n = match timeout(READ_TIMEOUT, stream.read(&mut tmp)).await {
            Ok(Ok(n)) => n,
            _ => return Ok(()),
        };
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_HEADERS_BYTES {
            return write_simple(&mut stream, 413, "Payload Too Large", "headers too large").await;
        }
        if let Some(pos) = find_double_crlf(&buf) {
            head_end = pos + 4;
            break;
        }
    }
    let req = match parse_request(&buf, head_end) {
        Ok(r) => r,
        Err(msg) => return write_simple(&mut stream, 400, "Bad Request", msg).await,
    };

    // ---- read body if needed (small JSON only) ----
    let body = if req.content_length > 0 {
        if req.content_length > MAX_BODY_BYTES {
            return write_simple(&mut stream, 413, "Payload Too Large", "body too large").await;
        }
        let mut body = buf[req.head_end..].to_vec();
        while body.len() < req.content_length {
            let n = match timeout(READ_TIMEOUT, stream.read(&mut tmp)).await {
                Ok(Ok(n)) => n,
                _ => return Ok(()),
            };
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(req.content_length);
        body
    } else {
        Vec::new()
    };

    // CORS preflight：webview 从 vite dev origin (http://localhost:1421) fetch
    // 跨源到 127.0.0.1:api_port 时,某些 fetch (带自定义 headers / 非 simple method)
    // 会先发 OPTIONS。直接 204 返回放行所有 method/header,loopback only 不存在
    // 安全风险。simple GET 也需要响应里带 Allow-Origin,所以 write_json 总是带。
    if req.method == "OPTIONS" {
        return write_cors_preflight(&mut stream).await;
    }

    // ---- route ----
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/healthz") => write_json(&mut stream, 200, &healthz_payload(&core)).await,
        ("GET", "/api/connection") => {
            let snap = core.connection_snapshot().await;
            write_json(&mut stream, 200, &serde_json::to_value(&snap).unwrap_or_default()).await
        }
        ("GET", "/api/servers") => {
            let list = core.list_servers().await;
            write_json(&mut stream, 200, &servers_payload(&list)).await
        }
        ("POST", "/api/servers/forget") => {
            let server_id = match parse_json_field(&body, "server_id") {
                Some(s) => s,
                None => {
                    return write_error(
                        &mut stream,
                        400,
                        "MISSING_FIELD",
                        "field 'server_id' required",
                    )
                    .await
                }
            };
            let removed = core.discoverer().forget(&server_id).await;
            write_json(
                &mut stream,
                200,
                &json!({ "ok": true, "removed": removed, "server_id": server_id }),
            )
            .await
        }
        ("POST", "/api/servers/forget_all") => {
            let n = core.discoverer().forget_all().await;
            write_json(&mut stream, 200, &json!({ "ok": true, "removed_count": n })).await
        }
        ("GET", "/api/traffic") => {
            let (sent, recv) = core.traffic_snapshot();
            write_json(&mut stream, 200, &traffic_payload(sent, recv)).await
        }
        ("GET", "/api/cache") => {
            let snap = core.cache().snapshot();
            let filtered = filter_cache(snap, &req.query);
            write_json(&mut stream, 200, &cache_payload(filtered)).await
        }
        ("DELETE", "/api/cache") => {
            let n = core.cache().clear();
            write_json(&mut stream, 200, &json!({ "ok": true, "removed": n })).await
        }
        ("POST", "/api/disconnect") => match core.disconnect().await {
            Ok(snap) => {
                let mut v = serde_json::to_value(&snap).unwrap_or_default();
                if let Value::Object(ref mut obj) = v {
                    obj.insert("ok".into(), Value::Bool(true));
                }
                write_json(&mut stream, 200, &v).await
            }
            Err(e) => write_error(&mut stream, 500, "DISCONNECT_FAILED", &e).await,
        },
        ("POST", path) if path.starts_with("/api/connect/") => {
            let server_id_raw = &path["/api/connect/".len()..];
            let server_id = url_decode(server_id_raw);
            let server = match core.discoverer().get_by_id(&server_id).await {
                Some(s) => s,
                None => {
                    return write_error(&mut stream, 404, "NOT_FOUND", "server not in registry")
                        .await;
                }
            };
            match core.connect_to(server).await {
                Ok(snap) => {
                    write_json(&mut stream, 200, &serde_json::to_value(&snap).unwrap_or_default())
                        .await
                }
                Err(e) => write_error(&mut stream, 500, "CONNECT_FAILED", &e).await,
            }
        }
        ("GET", "/api/diagnose") => write_json(&mut stream, 200, &diagnose_payload(&core).await).await,
        ("GET", "/api/events") => stream_events(stream, core).await,
        _ => write_error(&mut stream, 404, "NOT_FOUND", "unknown route").await,
    }
}

// -------------------- payload builders --------------------

fn healthz_payload(core: &Arc<ClientCore>) -> Value {
    let uptime = epoch_now() - core.started_at();
    json!({
        "ready": true,
        "checks": [
            {"name": "process", "ok": true, "detail": "running"},
        ],
        "uptime_sec": uptime.max(0.0),
    })
}

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

fn servers_payload(servers: &[DiscoveredServer]) -> Value {
    let wire: Vec<Value> = servers.iter().map(discovered_to_wire).collect();
    json!({
        "count": wire.len(),
        "available": true,
        "servers": wire,
    })
}

fn traffic_payload(sent: u64, recv: u64) -> Value {
    let ts = epoch_now();
    json!({
        "ts": ts,
        "uplink_bytes": 0u64,
        "downlink_bytes": 0u64,
        "total_uplink": sent,
        "total_downlink": recv,
    })
}

fn cache_payload(entries: Vec<conduit_core::RouteEntry>) -> Value {
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
    let now = epoch_now();
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
    json!({
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
    })
}

async fn diagnose_payload(core: &Arc<ClientCore>) -> Value {
    let snap = core.connection_snapshot().await;
    let mdns_available = !core.list_servers().await.is_empty();
    let server_reach = matches!(
        snap.state,
        conduit_core::ConnectionState::Connected
    );
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
    json!({
        "ok": ok,
        "checks": checks,
        "checked_at": epoch_now(),
    })
}

fn filter_cache(
    entries: Vec<conduit_core::RouteEntry>,
    query: &str,
) -> Vec<conduit_core::RouteEntry> {
    let mut direction_filter: Option<RouteDirection> = None;
    let mut source_filter: Option<String> = None;
    let mut limit: Option<usize> = None;
    for kv in query.split('&') {
        let Some((k, v)) = kv.split_once('=') else { continue };
        match k {
            "direction" => {
                direction_filter = match v {
                    "direct" => Some(RouteDirection::Direct),
                    "proxy" => Some(RouteDirection::Proxy),
                    _ => None,
                }
            }
            "source" => source_filter = Some(v.to_string()),
            "limit" => limit = v.parse::<usize>().ok(),
            _ => {}
        }
    }
    let mut out: Vec<_> = entries
        .into_iter()
        .filter(|e| direction_filter.is_none_or(|d| d == e.direction))
        .filter(|e| source_filter.as_ref().is_none_or(|s| s == &e.source))
        .collect();
    if let Some(n) = limit {
        out.truncate(n);
    }
    out
}

// -------------------- SSE --------------------

async fn stream_events(mut stream: TcpStream, core: Arc<ClientCore>) -> std::io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\n\
        Content-Type: text/event-stream\r\n\
        Cache-Control: no-cache\r\n\
        Connection: keep-alive\r\n\
        Access-Control-Allow-Origin: *\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    let mut sub = core.bus().subscribe();
    let cancel = core.cancel_token();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            ev = sub.recv() => {
                match ev {
                    Ok(ClientEvent { kind, payload, ts }) => {
                        let frame = format!(
                            "event: {kind}\ndata: {}\n\n",
                            json!({"ts": ts, "payload": payload})
                        );
                        if stream.write_all(frame.as_bytes()).await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("[control_api] SSE lagged {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

// -------------------- request parsing --------------------

fn parse_request(buf: &[u8], head_end: usize) -> Result<ParsedRequest, &'static str> {
    let head_text = std::str::from_utf8(&buf[..head_end]).map_err(|_| "non-utf8 headers")?;
    let mut lines = head_text.split("\r\n");
    let req_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = req_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("bad request line");
    }
    let method = parts[0].to_string();
    let target = parts[1];
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.to_string(), String::new()),
    };
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }
    Ok(ParsedRequest {
        method,
        path,
        query,
        content_length,
        head_end,
    })
}

fn parse_json_field(body: &[u8], field: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    v.get(field).and_then(|x| x.as_str()).map(|s| s.to_string())
}

// -------------------- writers --------------------

async fn write_json(stream: &mut TcpStream, code: u16, payload: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let status = http_status_text(code);
    // 始终带 CORS Allow-Origin: *,因为 loopback only,任何 origin 都安全。
    // 没有它 webview (vite dev: http://localhost:1421) 跨源 fetch 会被同源策略拦。
    let head = format!(
        "HTTP/1.1 {code} {status}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Access-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&body).await?;
    Ok(())
}

async fn write_cors_preflight(stream: &mut TcpStream) -> std::io::Result<()> {
    let head = "HTTP/1.1 204 No Content\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Access-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\n\
        Access-Control-Allow-Headers: Content-Type, Accept\r\n\
        Access-Control-Max-Age: 3600\r\n\
        Connection: close\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    Ok(())
}

async fn write_error(
    stream: &mut TcpStream,
    code: u16,
    err_code: &str,
    msg: &str,
) -> std::io::Result<()> {
    let payload = json!({
        "error": { "code": err_code, "message": msg }
    });
    write_json(stream, code, &payload).await
}

async fn write_simple(
    stream: &mut TcpStream,
    code: u16,
    status: &str,
    msg: &str,
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {code} {status}\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Access-Control-Allow-Origin: *\r\n\r\n",
        msg.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(msg.as_bytes()).await?;
    Ok(())
}

fn http_status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn url_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn iso8601(epoch: f64) -> String {
    let secs = epoch as i64;
    let nsecs = ((epoch - secs as f64) * 1e9) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsecs)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| epoch.to_string())
}

fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
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
