//! HTTP forward proxy (基于 hyper 1.x server + client)。
//!
//! ## 实装范围
//!
//! - **PAC serving**: `GET /proxy.pac` / `GET /wpad.dat` 返回
//!   [`conduit_core::PAC_TEMPLATE`] (来自 `crates/conduit-core/assets/proxy.pac`),
//!   并替换 `__PROXY_HOST__` / `__PROXY_PORT__` 占位符。
//! - **CONNECT 隧道**: 解析 host:port → 校验端口白名单 → 上游 TcpStream → 回
//!   `200 Connection Established` → 经 [`hyper::upgrade::on`] 拿 upgraded
//!   stream → 用 [`conduit_core::bidirectional_relay`] 做透传,会话登记
//!   / 字节计数交给 [`super::session::SessionRegistry`]。
//! - **absolute-URI 转发** (`GET http://host[:port]/path` 等 forward-proxy 形态):
//!   解析 absolute URI → 删除 hop-by-hop headers → 强制 `Connection: close` →
//!   用 hyper client 直发 upstream,流式回写响应,同时 wrap 进度
//!   sink 累计字节(只统计 body 字节,头不计)。
//! - **`GET /api/clients/heartbeat`**: 返回 `{ok, created, ttl_sec}` JSON,
//!   让 LAN 端 client-app 把自己注册为"已联通但暂无流量"的被动 peer。
//! - **`GET /status`**: 返回当前 [`super::core::ServerStatus`] JSON。
//! - **`GET /check?host=...`**: 返回 PAC 决策 JSON。
//! - **CIDR 校验**: peer IP 不在 `allowed_cidrs` 任一条目内 → 403。
//!
//! ## v0.2.3 起改用 hyper 1
//!
//! 之前 ~750 行手写 HTTP/1.1 解析+ 路由分发 + hop-by-hop 过滤 + chunked 防御 +
//! BufReader pending buffer reunite。现在 hyper 提供完整 HTTP/1.1 server +
//! upgrade 支持,本文件只剩业务路由 + ACL + relay 集成。
//! 详见 `design/2026-05-07-1-Conduit-第三方库替换计划.md` 阶段 5。
//!
//! ## 限制
//!
//! - chunked **请求体** 在 absolute-URI 转发模式下仍返 501 Not Implemented。
//!   这与 v0.2.2 行为一致:浏览器走 absolute-URI 已经罕见,CLI 工具(curl)
//!   不会主动 chunk 上行;真正需要时再补。
//! - hyper 自带 keep-alive,absolute-URI 转发响应强制 `Connection: close`,
//!   行为与旧版 forward proxy 一致。

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use conduit_core::{bidirectional_relay, PacRules, ProgressSink, PAC_TEMPLATE};
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1 as server_http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::client::legacy::Client as LegacyClient;
use hyper_util::rt::{TokioExecutor, TokioIo};
use log::{debug, info, warn};
use percent_encoding::percent_decode_str;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::config::ProxyConfig;
use super::core::ProxyCore;
use super::session::SessionRegistry;

/// 单条 request line 上限 (hyper 自带 8KB 默认,这里在 builder 里收一下)。
const MAX_REQUEST_LINE: usize = 8 * 1024;
/// headers 累计上限。
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// 启动 HTTP forward proxy 的 accept loop,直到 [`ProxyCore`] 取消。
pub async fn run(
    core: ProxyCore,
    cancel: CancellationToken,
    sessions: Arc<SessionRegistry>,
) -> std::io::Result<()> {
    let cfg = core.config();
    let bind = format!("{}:{}", cfg.bind, cfg.http_port);
    let listener = TcpListener::bind(&bind).await?;
    info!("[http] listening on {bind}");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("[http] cancellation requested, shutting down accept loop");
                break;
            }
            res = listener.accept() => {
                let (sock, peer) = match res {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("[http] accept error: {e}");
                        continue;
                    }
                };
                let core = core.clone();
                let sessions = sessions.clone();
                let cancel_child = cancel.clone();
                tokio::spawn(async move {
                    serve_one(sock, peer, core, sessions, cancel_child).await;
                });
            }
        }
    }
    Ok(())
}

/// 单连接 hyper service。捕获 peer 用于 ACL 校验。
async fn serve_one(
    sock: TcpStream,
    peer: SocketAddr,
    core: ProxyCore,
    sessions: Arc<SessionRegistry>,
    cancel: CancellationToken,
) {
    let peer_ip = peer.ip().to_string();
    let cfg = core.config();
    if !cfg.is_client_allowed(&peer_ip) {
        warn!("[http] reject {peer_ip} (not in allowed_cidrs)");
        // 不走 service,直接写一个 403 字节级响应再关闭。
        let _ = write_raw_403(sock).await;
        return;
    }

    let io = TokioIo::new(sock);
    let svc = service_fn(move |req: Request<Incoming>| {
        let core = core.clone();
        let sessions = sessions.clone();
        let peer_ip = peer_ip.clone();
        let cancel = cancel.clone();
        async move { handle_request(req, peer_ip, core, sessions, cancel).await }
    });

    let mut builder = server_http1::Builder::new();
    builder
        .max_buf_size(MAX_HEADER_BYTES + MAX_REQUEST_LINE)
        // CONNECT upgrade 必须走 with_upgrades 路径。
        ;
    if let Err(e) = builder.serve_connection(io, svc).with_upgrades().await {
        // hyper 在 client 主动断开时会返 IncompleteMessage / Closed / 类似;视作 debug。
        debug!("[http] connection ended: {e}");
    }
}

/// 单连接 503 写入 (在 ACL 拒绝时直接关闭连接)。
async fn write_raw_403(sock: TcpStream) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut s = sock;
    s.write_all(
        b"HTTP/1.1 403 Forbidden\r\n\
        Content-Type: text/plain\r\n\
        Content-Length: 22\r\n\
        Connection: close\r\n\r\n\
        client IP not allowed\n",
    )
    .await?;
    s.flush().await?;
    Ok(())
}

/// 主 router。
async fn handle_request(
    req: Request<Incoming>,
    peer_ip: String,
    core: ProxyCore,
    sessions: Arc<SessionRegistry>,
    _cancel: CancellationToken,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Infallible> {
    let cfg = core.config();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();

    // 1. CONNECT 隧道
    if method == Method::CONNECT {
        return Ok(handle_connect(req, &cfg, &peer_ip, sessions).await);
    }

    // 2. 静态短 endpoint (优先走 path 匹配,以避免 absolute-URI 解析消歧)。
    if method == Method::GET {
        match path.as_str() {
            "/proxy.pac" | "/wpad.dat" => {
                info!("[http] PAC served to {peer_ip}");
                return Ok(serve_pac_response(&cfg));
            }
            "/api/clients/heartbeat" => {
                let qs = uri.query().unwrap_or("");
                return Ok(serve_heartbeat(qs, &peer_ip, &sessions).await);
            }
            "/status" => {
                let body = serde_json::to_vec_pretty(&core.status().await)
                    .unwrap_or_else(|_| b"{}".to_vec());
                return Ok(json_response(StatusCode::OK, body));
            }
            "/check" => {
                let qs = uri.query().unwrap_or("");
                let pac_rules_opt = core.pac_rules().await;
                return Ok(serve_check(qs, pac_rules_opt.as_deref()));
            }
            _ => {}
        }
    }

    // 3. absolute-URI forward proxy: scheme + authority 都在 URI 上 (hyper 已解析)。
    if let (Some(scheme), Some(authority)) = (uri.scheme_str(), uri.authority()) {
        if scheme.eq_ignore_ascii_case("http") {
            let host = authority.host().to_string();
            let port = authority.port_u16().unwrap_or(80);
            let origin_target = match uri.path_and_query() {
                Some(pq) => pq.to_string(),
                None => "/".to_string(),
            };
            return Ok(handle_absolute_uri(
                req,
                &cfg,
                &method,
                &origin_target,
                &host,
                port,
                &peer_ip,
                sessions,
            )
            .await);
        }
    }

    Ok(plain_response(
        StatusCode::BAD_REQUEST,
        "forward proxy requires CONNECT or absolute-URI request\n".as_bytes(),
    ))
}

/// CONNECT host:port → 200 Connection Established → upgrade → bidirectional_relay。
async fn handle_connect(
    req: Request<Incoming>,
    cfg: &ProxyConfig,
    peer_ip: &str,
    sessions: Arc<SessionRegistry>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    let authority = match req.uri().authority() {
        Some(a) => a.clone(),
        None => return plain_response(StatusCode::BAD_REQUEST, b""),
    };
    let host = authority.host().to_string();
    let port = authority.port_u16().unwrap_or(443);
    if !cfg.is_connect_port_allowed(port) {
        warn!("[http] CONNECT {host}:{port} from {peer_ip} rejected (port not allowed)");
        return plain_response(StatusCode::FORBIDDEN, b"port not allowed\n");
    }
    info!("[http] CONNECT {host}:{port} from {peer_ip}");

    // 先连上游,失败短路;成功后才回 200,避免 hyper upgrade 之后才发现连不上 → 客户端错认为隧道 OK 然后立刻 EOF。
    let upstream = match tokio::time::timeout(
        Duration::from_secs_f64(cfg.connect_timeout_s),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            warn!("[http] CONNECT {host}:{port} from {peer_ip} FAILED: {e}");
            return plain_response(
                StatusCode::BAD_GATEWAY,
                format!("connect to {host}:{port} failed: {e}\n").as_bytes(),
            );
        }
        Err(_) => {
            warn!("[http] CONNECT {host}:{port} from {peer_ip} TIMEOUT");
            return plain_response(
                StatusCode::GATEWAY_TIMEOUT,
                format!("connect to {host}:{port} timed out\n").as_bytes(),
            );
        }
    };

    // 注册 session,准备 sink。
    let session = sessions
        .clone()
        .add(peer_ip.to_string(), "http", format!("{host}:{port}"))
        .await;
    let sink: Arc<dyn ProgressSink> = sessions.clone().sink_for(session.session_id.clone());
    let session_id = session.session_id.clone();
    let host_for_log = host.clone();
    let peer_for_log = peer_ip.to_string();

    // upgrade future 异步等待 client 收到 200 后接管 socket。
    tokio::spawn(async move {
        match hyper::upgrade::on(req).await {
            Ok(upgraded) => {
                let upgraded_io = TokioIo::new(upgraded);
                let (sent, recv) =
                    bidirectional_relay(upgraded_io, upstream, Some(sink)).await;
                sessions.remove(&session_id).await;
                info!(
                    "[http] CONNECT {host_for_log}:{port} from {peer_for_log} closed: \
                    sent={sent}B recv={recv}B"
                );
            }
            Err(e) => {
                sessions.remove(&session_id).await;
                debug!("[http] CONNECT upgrade failed for {peer_for_log}: {e}");
            }
        }
    });

    // 200 Connection Established + 空 body,hyper 看到这个会触发 upgrade。
    Response::builder()
        .status(StatusCode::OK)
        .body(empty_body())
        .expect("static response builder must not fail")
}

/// HTTP/1.1 hop-by-hop headers (RFC 7230 §6.1) — forward proxy 必须删除。
const HOP_BY_HOP_HEADERS: [&str; 8] = [
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "upgrade",
];

fn is_hop_by_hop(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    HOP_BY_HOP_HEADERS.iter().any(|h| *h == lower) || lower == "transfer-encoding"
}

/// absolute-URI 转发: 改写为 origin-form,删除 hop-by-hop,强制 close,转发到 upstream。
#[allow(clippy::too_many_arguments)]
async fn handle_absolute_uri(
    req: Request<Incoming>,
    cfg: &ProxyConfig,
    method: &Method,
    origin_target: &str,
    up_host: &str,
    up_port: u16,
    peer_ip: &str,
    sessions: Arc<SessionRegistry>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    if !cfg.is_connect_port_allowed(up_port) {
        warn!("[http] absolute-URI {method} {up_host}:{up_port} from {peer_ip} rejected (port not allowed)");
        return plain_response(StatusCode::FORBIDDEN, b"port not allowed\n");
    }

    // chunked request body 我们仍不支持(与旧版行为一致)。
    if let Some(te) = req.headers().get("transfer-encoding") {
        if te
            .to_str()
            .map(|v| v.to_ascii_lowercase().contains("chunked"))
            .unwrap_or(false)
        {
            return plain_response(
                StatusCode::NOT_IMPLEMENTED,
                b"chunked request body not supported by forward proxy\n",
            );
        }
    }

    info!("[http] {method} http://{up_host}:{up_port}{origin_target} from {peer_ip}");

    // 重组 outgoing request: URI 必须是 absolute-form,hyper-util Client 用其
    // host:port 决定 connect 目标,wire 上自动 normalise 到 origin-form。
    let (mut parts, body) = req.into_parts();
    let absolute = format!("http://{up_host}:{up_port}{origin_target}");
    parts.uri = match absolute.parse::<http::Uri>() {
        Ok(u) => u,
        Err(_) => return plain_response(StatusCode::BAD_REQUEST, b""),
    };

    // 删 hop-by-hop。
    let mut hdr_names: Vec<http::HeaderName> = Vec::new();
    for k in parts.headers.keys() {
        if is_hop_by_hop(k.as_str()) {
            hdr_names.push(k.clone());
        }
    }
    for k in hdr_names {
        parts.headers.remove(k);
    }
    // 必备 Host (没有就补一个)。
    if !parts.headers.contains_key("host") {
        let host_value = if up_port == 80 {
            up_host.to_string()
        } else {
            format!("{up_host}:{up_port}")
        };
        if let Ok(v) = http::HeaderValue::from_str(&host_value) {
            parts.headers.insert("host", v);
        }
    }
    parts
        .headers
        .insert("connection", http::HeaderValue::from_static("close"));

    // 把 Incoming body box 成 BoxBody<Bytes, hyper::Error>(hyper-util legacy client
    // 要求 body 错误类型实现 std::error::Error,Incoming 的原生 error type 是 hyper::Error)。
    let req_to_upstream: Request<BoxBody<Bytes, hyper::Error>> =
        Request::from_parts(parts, body.boxed());

    // hyper-util legacy client 可以方便地接 SocketAddr 形式 upstream。
    use hyper_util::client::legacy::connect::HttpConnector;
    let mut connector = HttpConnector::new();
    connector.set_connect_timeout(Some(Duration::from_secs_f64(cfg.connect_timeout_s)));
    let client: LegacyClient<HttpConnector, BoxBody<Bytes, hyper::Error>> =
        LegacyClient::builder(TokioExecutor::new())
            .pool_max_idle_per_host(0)
            .build(connector);

    let session = sessions
        .clone()
        .add(
            peer_ip.to_string(),
            "http",
            format!("{up_host}:{up_port}{origin_target}"),
        )
        .await;
    let sink: Arc<dyn ProgressSink> = sessions.clone().sink_for(session.session_id.clone());
    let session_id = session.session_id.clone();

    let resp = match client.request(req_to_upstream).await {
        Ok(r) => r,
        Err(e) => {
            sessions.remove(&session_id).await;
            warn!("[http] absolute-URI to {up_host}:{up_port} failed: {e}");
            // hyper-util client 的 error 不能精确区分 connect timeout vs refused;统一回 502。
            return plain_response(
                StatusCode::BAD_GATEWAY,
                format!("upstream connect or send failed: {e}\n").as_bytes(),
            );
        }
    };

    let (parts, body) = resp.into_parts();
    // 把 upstream 的 body 在透传过程中累计字节数。
    let counted = body
        .map_frame(move |frame| {
            if let Some(data) = frame.data_ref() {
                sink.on_progress(0, data.len() as u64);
            }
            frame
        })
        .map_err(|e| std::io::Error::other(format!("upstream body error: {e}")))
        .boxed();
    let resp_out = Response::from_parts(parts, counted);

    // session 在 body close 时已经累加完;但是我们没法精确捕获"客户端读完最后字节"的时机,
    // 简化为发出 response 后立即移除(rate 计算只看 peer_totals,不会丢)。
    sessions.remove(&session_id).await;
    resp_out
}

/// PAC 文件渲染并响应。代理 host 走 [`super::effective_advertised_host`]。
fn serve_pac_response(cfg: &ProxyConfig) -> Response<BoxBody<Bytes, std::io::Error>> {
    let proxy_host = super::effective_advertised_host(cfg);
    let body = PAC_TEMPLATE
        .replace("__PROXY_HOST__", &proxy_host)
        .replace("__PROXY_PORT__", &cfg.http_port.to_string());
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/x-ns-proxy-autoconfig")
        .header("cache-control", "no-store")
        .header("connection", "close")
        .body(full_body(body.into_bytes()))
        .expect("static response builder must not fail")
}

async fn serve_heartbeat(
    qs: &str,
    peer_ip: &str,
    sessions: &SessionRegistry,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    let mut name = "anonymous".to_string();
    let mut version = "unknown".to_string();
    for kv in qs.split('&') {
        let (k, v) = match kv.split_once('=') {
            Some(p) => p,
            None => continue,
        };
        match k {
            "name" if !v.is_empty() => name = url_decode(v),
            "version" if !v.is_empty() => version = url_decode(v),
            _ => {}
        }
    }
    let created = sessions.touch_passive(peer_ip, &name, &version).await;
    let body = serde_json::json!({
        "ok": true,
        "created": created,
        "ttl_sec": super::session::PASSIVE_CLIENT_TTL_SEC as u64,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    info!("[http] heartbeat from {peer_ip} name={name} version={version} (new={created})");
    json_response(StatusCode::OK, body_bytes)
}

fn serve_check(
    qs: &str,
    rules: Option<&PacRules>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    let mut host = String::new();
    for kv in qs.split('&') {
        if let Some(("host", v)) = kv.split_once('=') {
            host = url_decode(v).to_lowercase();
            break;
        }
    }
    if host.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            b"{\"error\": \"missing host parameter, use /check?host=foo.com\"}\n".to_vec(),
        );
    }
    let Some(rules) = rules else {
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            b"{\"error\": \"PAC rules not loaded on server\"}\n".to_vec(),
        );
    };
    let decision = rules.find_proxy(&host);
    let payload = serde_json::json!({
        "host": host,
        "proxy": decision.proxy,
        "matched_section": decision.matched_section,
        "matched_pattern": decision.matched_pattern,
    });
    let body = serde_json::to_vec_pretty(&payload).unwrap_or_else(|_| b"{}".to_vec());
    json_response(StatusCode::OK, body)
}

// ─────────────────────────────────────────────────────────────────────────────
// 通用响应 helper
// ─────────────────────────────────────────────────────────────────────────────

fn json_response(
    status: StatusCode,
    body: Vec<u8>,
) -> Response<BoxBody<Bytes, std::io::Error>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .header("cache-control", "no-store")
        .header("connection", "close")
        .body(full_body(body))
        .expect("static response builder must not fail")
}

fn plain_response(
    status: StatusCode,
    body: &[u8],
) -> Response<BoxBody<Bytes, std::io::Error>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .header("cache-control", "no-store")
        .header("connection", "close")
        .body(full_body(body.to_vec()))
        .expect("static response builder must not fail")
}

fn empty_body() -> BoxBody<Bytes, std::io::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full_body(bytes: Vec<u8>) -> BoxBody<Bytes, std::io::Error> {
    Full::new(Bytes::from(bytes))
        .map_err(|never| match never {})
        .boxed()
}

/// 解码 `application/x-www-form-urlencoded` 风格的 percent-escape (含 `%XX`)。
///
/// 走 [`percent_encoding::percent_decode_str`] (RFC 3986),与原手写实现行为一致:
/// - `%20` → space
/// - 非完整 `%XY` → 保持原样
/// - `+` 不视为 space
fn url_decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{ProxyConfig, ProxyCore};
    use std::time::Duration as StdDuration;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener as TL, TcpStream as TS};

    /// 启动一个临时 ProxyCore,返回 (core, http_port)。
    async fn start_test_core() -> (ProxyCore, u16) {
        let http = portpicker::pick_unused_port().expect("free port");
        let socks = portpicker::pick_unused_port().expect("free port");
        let api = portpicker::pick_unused_port().expect("free port");
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        // 测试用: 放开任意 loopback 端口,方便上游 echo 在随机端口跑
        cfg.allowed_connect_ports = (1..=65535).collect();
        let core = ProxyCore::new(cfg);
        core.start().await.expect("start");
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", http)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        (core, http)
    }

    /// 把 reader 全部读到 EOF,返回 String。
    async fn read_to_string(mut s: TS) -> String {
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn pac_endpoint_returns_template_with_substituted_host_port() {
        let (core, port) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /proxy.pac HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let body = read_to_string(s).await;
        assert!(body.starts_with("HTTP/1.1 200 OK"));
        assert!(
            body.contains("application/x-ns-proxy-autoconfig"),
            "missing PAC content-type: {body}"
        );
        assert!(!body.contains("__PROXY_HOST__"), "host placeholder leaked: {body}");
        assert!(!body.contains("__PROXY_PORT__"), "port placeholder leaked: {body}");
        assert!(
            body.contains(&format!("127.0.0.1:{port}")),
            "missing substituted host:port: {body}"
        );
        core.stop().await;
    }

    #[tokio::test]
    async fn heartbeat_endpoint_returns_ok_and_registers_passive_client() {
        let (core, port) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(
            b"GET /api/clients/heartbeat?name=alice-mac&version=0.2.0 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let body = read_to_string(s).await;
        assert!(body.contains("\"ok\":true"), "{body}");
        assert!(body.contains("\"created\":true"), "{body}");
        let passives = core.sessions().passive_clients().await;
        assert_eq!(passives.len(), 1);
        assert_eq!(passives[0].client_name, "alice-mac");
        core.stop().await;
    }

    #[tokio::test]
    async fn heartbeat_query_url_decodes_special_chars_into_passive_record() {
        let (core, port) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(
            b"GET /api/clients/heartbeat?name=Bob%27s%20iPhone&version=v1%2E2%2E3 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let body = read_to_string(s).await;
        assert!(body.contains("\"ok\":true"), "{body}");
        let passives = core.sessions().passive_clients().await;
        assert_eq!(passives.len(), 1, "expect exactly one passive registered");
        assert_eq!(passives[0].client_name, "Bob's iPhone");
        assert_eq!(passives[0].version, "v1.2.3");
        core.stop().await;
    }

    #[tokio::test]
    async fn heartbeat_repeated_call_does_not_duplicate_passive_record() {
        let (core, port) = start_test_core().await;
        for _ in 0..3 {
            let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(
                b"GET /api/clients/heartbeat?name=mac01&version=0.2.0 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
            let _ = read_to_string(s).await;
        }
        let passives = core.sessions().passive_clients().await;
        assert_eq!(passives.len(), 1, "passive registry must dedup by peer_ip");
        assert_eq!(passives[0].client_name, "mac01");
        core.stop().await;
    }

    #[tokio::test]
    async fn connect_tunnels_payload_to_upstream_echo_server() {
        let (core, port) = start_test_core().await;

        let echo_listener = TL::bind("127.0.0.1:0").await.unwrap();
        let echo_addr = echo_listener.local_addr().unwrap();
        let echo_task = tokio::spawn(async move {
            if let Ok((mut sock, _)) = echo_listener.accept().await {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            }
        });

        let mut client = TS::connect(("127.0.0.1", port)).await.unwrap();
        let req = format!(
            "CONNECT 127.0.0.1:{p} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n",
            p = echo_addr.port()
        );
        client.write_all(req.as_bytes()).await.unwrap();

        // 读 200 响应头(直到 \r\n\r\n)
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            let n = client.read(&mut byte).await.unwrap();
            assert!(n > 0, "premature EOF before 200 line");
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
            if head.len() > 1024 {
                panic!("response head too long: {}", String::from_utf8_lossy(&head));
            }
        }
        let head_str = String::from_utf8_lossy(&head);
        assert!(head_str.starts_with("HTTP/1.1 200"), "{head_str}");

        client.write_all(b"hello-tunnel").await.unwrap();
        client.shutdown().await.unwrap();
        let mut echo_back = Vec::new();
        client.read_to_end(&mut echo_back).await.unwrap();
        assert_eq!(echo_back, b"hello-tunnel");

        let _ = echo_task.await;
        core.stop().await;
    }

    #[tokio::test]
    async fn absolute_uri_get_is_forwarded_to_upstream_origin_form() {
        let (core, port) = start_test_core().await;

        let upstream = TL::bind("127.0.0.1:0").await.unwrap();
        let up_addr = upstream.local_addr().unwrap();
        let up_task = tokio::spawn(async move {
            let (mut sock, _) = upstream.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let mut got = Vec::new();
            loop {
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                if got.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&got).to_string();
            sock.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nUPSTREAMOK",
            )
            .await
            .unwrap();
            head
        });

        let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
        let req = format!(
            "GET http://127.0.0.1:{p}/hello HTTP/1.1\r\nHost: x\r\nProxy-Connection: keep-alive\r\nUser-Agent: t\r\nConnection: close\r\n\r\n",
            p = up_addr.port()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let body = read_to_string(s).await;
        assert!(body.starts_with("HTTP/1.1 200"), "client should see upstream 200: {body}");
        assert!(body.contains("UPSTREAMOK"), "client should see upstream body: {body}");

        let upstream_head = up_task.await.unwrap();
        assert!(
            upstream_head.starts_with("GET /hello HTTP/1.1\r\n"),
            "request-line must be rewritten to origin-form, got: {upstream_head}"
        );
        assert!(
            !upstream_head.to_ascii_lowercase().contains("proxy-connection"),
            "Proxy-Connection must be stripped: {upstream_head}"
        );
        assert!(
            upstream_head.to_ascii_lowercase().contains("connection: close"),
            "Connection: close must be forced: {upstream_head}"
        );
        core.stop().await;
    }

    #[tokio::test]
    async fn absolute_uri_chunked_request_body_returns_501() {
        let (core, port) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(
            b"POST http://127.0.0.1:80/upload HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let body = read_to_string(s).await;
        assert!(body.starts_with("HTTP/1.1 501"), "{body}");
        core.stop().await;
    }

    #[tokio::test]
    async fn connect_to_disallowed_port_returns_403() {
        let http = portpicker::pick_unused_port().unwrap();
        let socks = portpicker::pick_unused_port().unwrap();
        let api = portpicker::pick_unused_port().unwrap();
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        // 故意不放开 6379;CONNECT 6379 应该被拒
        let core = ProxyCore::new(cfg);
        core.start().await.unwrap();
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", http)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }

        let mut s = TS::connect(("127.0.0.1", http)).await.unwrap();
        s.write_all(b"CONNECT 127.0.0.1:6379 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let body = read_to_string(s).await;
        assert!(body.starts_with("HTTP/1.1 403"), "{body}");
        assert!(body.contains("port not allowed"), "{body}");
        core.stop().await;
    }
}
