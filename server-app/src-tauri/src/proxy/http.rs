//! HTTP forward proxy（hyper-free，纯 tokio + 手写 HTTP/1.1 解析）。
//!
//! S2.2 第一轮覆盖：
//! - **PAC serving**：`GET /proxy.pac` / `GET /wpad.dat` 返回
//!   [`conduit_core::PAC_TEMPLATE`]（来自 `crates/conduit-core/assets/proxy.pac`），
//!   并替换 `__PROXY_HOST__` / `__PROXY_PORT__` 占位符。
//! - **CONNECT 隧道**：解析 host:port → 校验端口白名单 → 上游 TcpStream → 回
//!   `200 Connection Established` → 用 [`conduit_core::bidirectional_relay`]
//!   做透传，并把会话登记 / 字节计数交给 [`super::session::SessionRegistry`]。
//! - **`GET /api/clients/heartbeat`**：返回 `{ok, created, ttl_sec}` JSON，
//!   让 LAN 端 client-app 把自己注册为"已联通但暂无流量"的被动 peer。
//! - **`GET /status`**：返回当前 [`super::core::ServerStatus`] JSON。
//! - **`GET /check?host=...`**：返回 PAC 决策 JSON。
//! - **CIDR 校验**：peer IP 不在 `allowed_cidrs` 任一条目内 → 403。
//!
//! 当前未覆盖（留给 S2.2 第二轮）：
//! - absolute-URI 转发（`GET http://...` 而非 CONNECT）
//! - CORS preflight
//! - PAC 决策驱动的 outbound policy（DIRECT-first / VPN-only），需要 S2.5
//!   完成 outbound 模块后接入。

use std::sync::Arc;
use std::time::Duration;

use conduit_core::{bidirectional_relay, PacRules, ProgressSink, PAC_TEMPLATE};
use log::{debug, info, warn};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::core::ProxyCore;
use super::session::SessionRegistry;

/// 单条 request line 上限（防御 DoS / 错误请求）。
const MAX_REQUEST_LINE: usize = 8192;

/// 全部 headers（含 request line）的累计上限。
const MAX_HEADERS_BYTES: usize = 64 * 1024;

/// 初次 accept 后等待客户端发完 request line 的最大时长。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 启动 HTTP forward proxy 的 accept loop，直到 [`ProxyCore`] 取消。
///
/// 返回的 task 在 cancel-token 触发后退出。所有客户端连接独立 spawn，单个连接
/// 出错不影响监听 loop。
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
                    let peer_ip = peer.ip().to_string();
                    if let Err(e) =
                        handle_connection(sock, peer_ip.clone(), core, sessions, cancel_child).await
                    {
                        debug!("[http] {peer_ip} connection ended: {e}");
                    }
                });
            }
        }
    }
    Ok(())
}

/// 单次 HTTP 客户端连接处理：分发到 CONNECT / PAC / status / check / heartbeat。
async fn handle_connection(
    stream: TcpStream,
    peer_ip: String,
    core: ProxyCore,
    sessions: Arc<SessionRegistry>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let cfg = core.config();
    if !cfg.is_client_allowed(&peer_ip) {
        warn!("[http] reject {peer_ip} (not in allowed_cidrs)");
        let mut s = stream;
        send_simple(&mut s, 403, "Forbidden", b"client IP not allowed\n", None).await?;
        return Ok(());
    }

    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::with_capacity(8 * 1024, read_half);

    let request_line = match read_request_line(&mut reader).await? {
        Some(line) => line,
        None => return Ok(()), // 空连接 / EOF
    };
    if request_line.len() > MAX_REQUEST_LINE {
        send_simple(&mut write_half, 414, "URI Too Long", b"", None).await?;
        return Ok(());
    }
    let (method, target, _version) = match parse_request_line(&request_line) {
        Some(v) => v,
        None => {
            send_simple(&mut write_half, 400, "Bad Request", b"", None).await?;
            return Ok(());
        }
    };

    let path_only = target.split('?').next().unwrap_or("").to_string();
    if method == "GET" && (path_only == "/proxy.pac" || path_only == "/wpad.dat") {
        drain_headers(&mut reader).await.ok();
        serve_pac(&mut write_half, &cfg.pac_advertised_host, cfg.http_port, &cfg.bind).await?;
        info!("[http] PAC served to {peer_ip}");
        return Ok(());
    }
    if method == "GET" && path_only == "/api/clients/heartbeat" {
        drain_headers(&mut reader).await.ok();
        serve_heartbeat(&mut write_half, &target, &peer_ip, &sessions).await?;
        return Ok(());
    }
    if method == "GET" && path_only == "/status" {
        drain_headers(&mut reader).await.ok();
        let status = core.status().await;
        let body = serde_json::to_vec_pretty(&status).unwrap_or_else(|_| b"{}".to_vec());
        send_simple(
            &mut write_half,
            200,
            "OK",
            &body,
            Some("application/json; charset=utf-8"),
        )
        .await?;
        return Ok(());
    }
    if method == "GET" && path_only == "/check" {
        drain_headers(&mut reader).await.ok();
        serve_check(&mut write_half, &target, core.pac_rules().await.as_deref()).await?;
        return Ok(());
    }
    if method == "CONNECT" {
        return handle_connect(reader, write_half, &cfg, &target, &peer_ip, sessions, cancel).await;
    }

    // absolute-URI 等转发能力留给 S2.2 第二轮，先返回 501 让客户端知道未实现。
    send_simple(
        &mut write_half,
        501,
        "Not Implemented",
        b"absolute-URI forwarding not yet ported (W2 S2.2 round-2)\n",
        None,
    )
    .await
}

/// 解析 `CONNECT host:port HTTP/1.1` → 建上游 TcpStream → 回 200 → bidirectional relay。
async fn handle_connect(
    mut reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    cfg: &super::config::ProxyConfig,
    target: &str,
    peer_ip: &str,
    sessions: Arc<SessionRegistry>,
    _cancel: CancellationToken,
) -> std::io::Result<()> {
    let (host, port) = match parse_authority(target) {
        Some(v) => v,
        None => {
            send_simple(&mut write_half, 400, "Bad Request", b"", None).await?;
            return Ok(());
        }
    };
    if !cfg.is_connect_port_allowed(port) {
        warn!("[http] CONNECT {host}:{port} from {peer_ip} rejected (port not allowed)");
        send_simple(
            &mut write_half,
            403,
            "Forbidden",
            b"port not allowed\n",
            None,
        )
        .await?;
        return Ok(());
    }
    drain_headers(&mut reader).await.ok();

    info!("[http] CONNECT {host}:{port} from {peer_ip}");
    let upstream = match tokio::time::timeout(
        Duration::from_secs_f64(cfg.connect_timeout_s),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            warn!("[http] CONNECT {host}:{port} from {peer_ip} FAILED: {e}");
            send_simple(
                &mut write_half,
                502,
                "Bad Gateway",
                format!("connect to {host}:{port} failed: {e}\n").as_bytes(),
                None,
            )
            .await?;
            return Ok(());
        }
        Err(_) => {
            warn!("[http] CONNECT {host}:{port} from {peer_ip} TIMEOUT");
            send_simple(
                &mut write_half,
                504,
                "Gateway Timeout",
                format!("connect to {host}:{port} timed out\n").as_bytes(),
                None,
            )
            .await?;
            return Ok(());
        }
    };

    write_half
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    write_half.flush().await?;

    let session = sessions
        .add(peer_ip.to_string(), "http", format!("{host}:{port}"))
        .await;
    let sink: Arc<dyn ProgressSink> = sessions.clone().sink_for(session.session_id.clone());

    let client_stream = reunite_stream(reader, write_half);
    let (sent, recv) = bidirectional_relay(client_stream, upstream, Some(sink)).await;
    sessions.remove(&session.session_id).await;
    info!(
        "[http] CONNECT {host}:{port} from {peer_ip} closed: sent={sent}B recv={recv}B"
    );
    Ok(())
}

/// 把 `BufReader<OwnedReadHalf>` 与 `OwnedWriteHalf` 重新拼回单个 [`TcpStream`] 形态，
/// 使 [`bidirectional_relay`] 能拿到一个实现了 `AsyncRead + AsyncWrite` 的整体。
///
/// 实现细节：tokio 的 `OwnedReadHalf` / `OwnedWriteHalf` 通过 `reunite` 还原，
/// 但我们用了 `BufReader`，所以需要先把 `BufReader` 中尚未消费的字节回读出来——
/// 此处 `BufReader::buffer().is_empty()` 一定成立，因为 CONNECT 流程在
/// `drain_headers` 后下一字节就是 client 发往 target 的明文数据；如果 buffer
/// 还有残留 byte，relay 会丢失它们。所以 debug-assert 一下做防御。
fn reunite_stream(
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    write_half: tokio::net::tcp::OwnedWriteHalf,
) -> TcpStream {
    debug_assert!(
        reader.buffer().is_empty(),
        "BufReader must be drained before reunite (would lose pre-buffered bytes)"
    );
    let read_half = reader.into_inner();
    read_half
        .reunite(write_half)
        .expect("read/write halves came from the same stream")
}

/// 读取一行 request line（含末尾 `\r\n`）。空连接 / EOF 返回 `None`。
async fn read_request_line(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> std::io::Result<Option<String>> {
    let mut buf = String::new();
    let read = tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.read_line(&mut buf)).await;
    let n = match read {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(None),
    };
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(buf))
}

/// `METHOD TARGET HTTP/x.y\r\n` → `(method_upper, target, version)`。
fn parse_request_line(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let mut it = trimmed.splitn(3, ' ');
    let method = it.next()?.to_uppercase();
    let target = it.next()?.to_string();
    let version = it.next()?.to_string();
    Some((method, target, version))
}

/// 把 headers 全部读到双 CRLF（请求体不消费）。返回时 `reader` 的位置就是请求体起点。
async fn drain_headers(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> std::io::Result<()> {
    let mut total = 0usize;
    let mut line = String::new();
    loop {
        line.clear();
        let n = tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "drain headers"))??;
        if n == 0 {
            return Ok(()); // EOF before \r\n\r\n
        }
        total += n;
        if total > MAX_HEADERS_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "headers too large",
            ));
        }
        if line == "\r\n" || line == "\n" {
            return Ok(());
        }
    }
}

fn parse_authority(authority: &str) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        // [ipv6]:port
        let (host, after) = rest.split_once(']')?;
        let port = after.strip_prefix(':')?.parse().ok().unwrap_or(443);
        return Some((host.to_string(), port));
    }
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().unwrap_or(443)),
        None => (authority, 443),
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

/// PAC 文件渲染并发出。`advertised_host` 为空时用 bind 地址；进一步为空时用 127.0.0.1。
async fn serve_pac(
    out: &mut tokio::net::tcp::OwnedWriteHalf,
    advertised_host: &str,
    http_port: u16,
    bind: &str,
) -> std::io::Result<()> {
    let proxy_host = if !advertised_host.is_empty() {
        advertised_host
    } else if !bind.is_empty() && bind != "0.0.0.0" {
        bind
    } else {
        "127.0.0.1"
    };
    let body = PAC_TEMPLATE
        .replace("__PROXY_HOST__", proxy_host)
        .replace("__PROXY_PORT__", &http_port.to_string());
    send_simple(
        out,
        200,
        "OK",
        body.as_bytes(),
        Some("application/x-ns-proxy-autoconfig"),
    )
    .await
}

/// `GET /api/clients/heartbeat?name=xxx&version=yyy` → 200 JSON。
async fn serve_heartbeat(
    out: &mut tokio::net::tcp::OwnedWriteHalf,
    target: &str,
    peer_ip: &str,
    sessions: &SessionRegistry,
) -> std::io::Result<()> {
    let qs = target.split('?').nth(1).unwrap_or("");
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
    send_simple(
        out,
        200,
        "OK",
        &body_bytes,
        Some("application/json; charset=utf-8"),
    )
    .await
}

/// `GET /check?host=...` → PAC 决策 JSON。
async fn serve_check(
    out: &mut tokio::net::tcp::OwnedWriteHalf,
    target: &str,
    rules: Option<&PacRules>,
) -> std::io::Result<()> {
    let qs = target.split('?').nth(1).unwrap_or("");
    let mut host = String::new();
    for kv in qs.split('&') {
        if let Some(("host", v)) = kv.split_once('=') {
            host = url_decode(v).to_lowercase();
            break;
        }
    }
    if host.is_empty() {
        let body = b"{\"error\": \"missing host parameter, use /check?host=foo.com\"}\n";
        return send_simple(
            out,
            400,
            "Bad Request",
            body,
            Some("application/json; charset=utf-8"),
        )
        .await;
    }
    let Some(rules) = rules else {
        let body = b"{\"error\": \"PAC rules not loaded on server\"}\n";
        return send_simple(
            out,
            503,
            "Service Unavailable",
            body,
            Some("application/json; charset=utf-8"),
        )
        .await;
    };
    let decision = rules.find_proxy(&host);
    let payload = serde_json::json!({
        "host": host,
        "proxy": decision.proxy,
        "matched_section": decision.matched_section,
        "matched_pattern": decision.matched_pattern,
    });
    let body = serde_json::to_vec_pretty(&payload).unwrap_or_else(|_| b"{}".to_vec());
    send_simple(
        out,
        200,
        "OK",
        &body,
        Some("application/json; charset=utf-8"),
    )
    .await
}

/// 极简 percent-decode：仅解码 `%xx`，其它字符（含 `+`）保持原样。
/// 我们只用它处理 query string 里的 client name / version，所以足够。
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                hex_nibble(bytes[i + 1]),
                hex_nibble(bytes[i + 2]),
            ) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned())
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 发出一个 HTTP/1.1 响应（Connection: close + Cache-Control: no-store）。
async fn send_simple<W: AsyncWriteExt + Unpin>(
    out: &mut W,
    status_code: u16,
    reason: &str,
    body: &[u8],
    content_type: Option<&str>,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        body.len()
    );
    if let Some(ct) = content_type {
        head.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        out.write_all(body).await?;
    }
    out.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_line_basic() {
        let (m, t, v) = parse_request_line("CONNECT example.com:443 HTTP/1.1\r\n").unwrap();
        assert_eq!(m, "CONNECT");
        assert_eq!(t, "example.com:443");
        assert_eq!(v, "HTTP/1.1");
    }

    #[test]
    fn parse_request_line_lowercase_method_normalised() {
        let (m, _, _) = parse_request_line("get /proxy.pac HTTP/1.1\r\n").unwrap();
        assert_eq!(m, "GET");
    }

    #[test]
    fn parse_authority_default_443() {
        assert_eq!(parse_authority("example.com").unwrap(), ("example.com".into(), 443));
    }

    #[test]
    fn parse_authority_custom_port() {
        assert_eq!(parse_authority("example.com:8080").unwrap(), ("example.com".into(), 8080));
    }

    #[test]
    fn parse_authority_ipv6_form() {
        let (h, p) = parse_authority("[::1]:1234").unwrap();
        assert_eq!(h, "::1");
        assert_eq!(p, 1234);
    }

    #[test]
    fn parse_authority_rejects_empty_host() {
        assert!(parse_authority(":443").is_none());
    }

    #[test]
    fn url_decode_handles_percent_and_unencoded() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("plain"), "plain");
        assert_eq!(url_decode("name%3Dvalue"), "name=value");
    }

    #[test]
    fn url_decode_passes_through_invalid_percent() {
        assert_eq!(url_decode("%XY"), "%XY");
    }

    // ----- integration tests: start ProxyCore on a real ephemeral port -----
    use crate::proxy::{ProxyConfig, ProxyCore};
    use std::time::Duration as StdDuration;
    use tokio::io::AsyncReadExt as _;
    use tokio::net::{TcpListener as TL, TcpStream as TS};

    /// 启动一个临时 ProxyCore，返回 (core, http_port)。
    async fn start_test_core() -> (ProxyCore, u16) {
        let http = portpicker::pick_unused_port().expect("free port");
        let socks = portpicker::pick_unused_port().expect("free port");
        let api = portpicker::pick_unused_port().expect("free port");
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        // 测试用：放开任意 loopback 端口，方便上游 echo 在随机端口跑
        cfg.allowed_connect_ports = (1..=65535).collect();
        let core = ProxyCore::new(cfg);
        core.start().await.expect("start");
        // 等 listener bind 完成（accept loop 是异步 spawn 的）
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", http)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        (core, http)
    }

    /// 把 reader 全部读到 EOF，返回 String（响应体一定是 ASCII / UTF-8）。
    async fn read_to_string(mut s: TS) -> String {
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn pac_endpoint_returns_template_with_substituted_host_port() {
        let (core, port) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET /proxy.pac HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let body = read_to_string(s).await;
        assert!(body.starts_with("HTTP/1.1 200 OK"));
        assert!(
            body.contains("application/x-ns-proxy-autoconfig"),
            "missing PAC content-type: {body}"
        );
        // PAC 模板的 `__PROXY_HOST__` / `__PROXY_PORT__` 必须已被替换
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
            b"GET /api/clients/heartbeat?name=alice-mac&version=0.2.0 HTTP/1.1\r\nHost: x\r\n\r\n",
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
    async fn connect_tunnels_payload_to_upstream_echo_server() {
        let (core, port) = start_test_core().await;

        // 起一个 echo server（每收到字节就原样发回，最后 close）
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

        // 读 200 Connection Established 响应头（直到 \r\n\r\n）
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

        // 隧道建立后写入 → 应原样返回
        client.write_all(b"hello-tunnel").await.unwrap();
        client.shutdown().await.unwrap();
        let mut echo_back = Vec::new();
        client.read_to_end(&mut echo_back).await.unwrap();
        assert_eq!(echo_back, b"hello-tunnel");

        let _ = echo_task.await;
        core.stop().await;
    }

    #[tokio::test]
    async fn connect_to_disallowed_port_returns_403() {
        let http = portpicker::pick_unused_port().unwrap();
        let socks = portpicker::pick_unused_port().unwrap();
        let api = portpicker::pick_unused_port().unwrap();
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        // 故意不放开 6379；CONNECT 6379 应该被拒
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
