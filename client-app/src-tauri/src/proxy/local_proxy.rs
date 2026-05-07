//! `LocalProxy` —— 客户端本地 SOCKS5 listener（系统代理面向的入口端口）。
//!
//! 实现要点：
//! - 监听 127.0.0.1:bind_port，接受 RFC1928 SOCKS5（NO-AUTH，CMD=CONNECT）。
//! - 对每条连接调 [`RouteResolver::decide`] 决策 direct / proxy。
//! - direct：本机 `TcpStream::connect(host:port)`。
//! - proxy：经由当前 `ServerEndpoint` 的 SOCKS5（一次嵌套握手）转上游 server。
//! - 双向 relay 用 [`conduit_core::bidirectional_relay`]，进度推到 [`TrafficMeter`]。
//! - 上游 endpoint 可 `set_server_endpoint(None)` 切回 idle 模式（全部直连）。

use std::sync::Arc;
use std::time::Duration;

use log::{debug, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use conduit_core::socks5_proto::{
    bnd_addr_len, consts as s5, encode_connect_request, encode_method_request_no_auth,
    encode_method_response, encode_reply, parse_address_bytes, parse_method_response,
    parse_reply_head, validate_version, Socks5Address,
};
use conduit_core::{bidirectional_relay, ProgressSink, RouteDirection};

use super::route_resolver::RouteResolver;
use super::traffic_meter::TrafficMeter;

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const DIRECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 上游 server 端点（host + SOCKS5 port）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEndpoint {
    pub host: String,
    pub socks_port: u16,
}

impl ServerEndpoint {
    pub fn label(&self) -> String {
        format!("{}:{}", self.host, self.socks_port)
    }
}

#[derive(Clone)]
pub struct LocalProxy {
    inner: Arc<Inner>,
}

struct Inner {
    bind_host: String,
    bind_port: u16,
    actual_port: Mutex<u16>,
    resolver: RouteResolver,
    upstream: Mutex<Option<ServerEndpoint>>,
    sink: Mutex<Option<Arc<dyn ProgressSink>>>,
    cancel: CancellationToken,
}

impl LocalProxy {
    pub fn new(bind_host: String, bind_port: u16, resolver: RouteResolver) -> Self {
        Self {
            inner: Arc::new(Inner {
                bind_host,
                bind_port,
                actual_port: Mutex::new(0),
                resolver,
                upstream: Mutex::new(None),
                sink: Mutex::new(None),
                cancel: CancellationToken::new(),
            }),
        }
    }

    pub async fn actual_port(&self) -> u16 {
        *self.inner.actual_port.lock().await
    }

    pub async fn set_server_endpoint(&self, endpoint: Option<ServerEndpoint>) {
        *self.inner.upstream.lock().await = endpoint;
    }

    pub async fn current_endpoint(&self) -> Option<ServerEndpoint> {
        self.inner.upstream.lock().await.clone()
    }

    pub async fn set_progress_sink(&self, sink: Option<TrafficMeter>) {
        *self.inner.sink.lock().await = sink.map(|m| Arc::new(m) as Arc<dyn ProgressSink>);
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.inner.cancel.clone()
    }

    /// 启动 accept loop。返回前会先 await listener bind 完成（拿到 actual port）。
    pub async fn start(&self) -> std::io::Result<u16> {
        let bind_addr = format!("{}:{}", self.inner.bind_host, self.inner.bind_port);
        let listener = TcpListener::bind(&bind_addr).await?;
        let actual_port = listener.local_addr()?.port();
        *self.inner.actual_port.lock().await = actual_port;
        info!("[local_proxy] SOCKS5 listening on {bind_addr} (actual_port={actual_port})");

        let inner = self.inner.clone();
        tokio::spawn(async move {
            run_accept_loop(inner, listener).await;
        });
        Ok(actual_port)
    }

    pub async fn stop(&self) {
        self.inner.cancel.cancel();
    }
}

async fn run_accept_loop(inner: Arc<Inner>, listener: TcpListener) {
    loop {
        tokio::select! {
            _ = inner.cancel.cancelled() => {
                info!("[local_proxy] accept loop exiting");
                return;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, peer)) => {
                        let inner_for_task = inner.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_socks5(inner_for_task, stream).await {
                                debug!("[local_proxy] {peer} session error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        warn!("[local_proxy] accept error: {e}");
                    }
                }
            }
        }
    }
}

async fn handle_socks5(inner: Arc<Inner>, mut client: TcpStream) -> std::io::Result<()> {
    // ---- step 1: 协商方法（用 conduit_core::socks5_proto 解 + 编） ----
    timeout(HANDSHAKE_TIMEOUT, async {
        let mut head = [0u8; 2];
        client.read_exact(&mut head).await?;
        validate_version(head[0]).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not socks5: {e}"))
        })?;
        let nmethods = head[1] as usize;
        let mut methods = vec![0u8; nmethods];
        client.read_exact(&mut methods).await?;
        // 一律 NO-AUTH 接受
        client.write_all(&encode_method_response(true)).await?;
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "method negotiation timeout"))??;

    // ---- step 2: 解析请求 ----
    let mut req_head = [0u8; 4];
    client.read_exact(&mut req_head).await?;
    if validate_version(req_head[0]).is_err() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad req version",
        ));
    }
    if req_head[1] != s5::CMD_CONNECT {
        write_local_reply(&mut client, s5::REP_CMD_NOT_SUPPORTED).await?;
        return Ok(());
    }
    let atyp = req_head[3];
    let address: Socks5Address = match atyp {
        s5::ATYP_IPV4 | s5::ATYP_IPV6 => {
            let len = bnd_addr_len(atyp).expect("ATYP_IPV4/IPV6 长度已知").unwrap();
            let mut raw = vec![0u8; len];
            client.read_exact(&mut raw).await?;
            parse_address_bytes(atyp, &raw).expect("固定长度 IP parse 不会失败")
        }
        s5::ATYP_DOMAIN => {
            let mut len_buf = [0u8; 1];
            client.read_exact(&mut len_buf).await?;
            let mut name = vec![0u8; len_buf[0] as usize];
            client.read_exact(&mut name).await?;
            parse_address_bytes(atyp, &name).expect("ATYP_DOMAIN parse 不会失败")
        }
        _ => {
            write_local_reply(&mut client, s5::REP_ATYP_NOT_SUPPORTED).await?;
            return Ok(());
        }
    };
    let host = address.host_string();
    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    // ---- step 3: 路由决策 ----
    let decision = inner.resolver.decide(&host, port).await;
    debug!(
        "[local_proxy] CONNECT {host}:{port} → {:?} (source={:?})",
        decision.direction, decision.source
    );

    // ---- step 4: 建上游连接 ----
    let upstream = match decision.direction {
        RouteDirection::Direct => connect_direct(&host, port).await,
        RouteDirection::Proxy => {
            let endpoint = inner.upstream.lock().await.clone();
            match endpoint {
                Some(ep) => connect_via_proxy(&ep, &host, port).await,
                None => {
                    // 没配上游，强制 direct
                    debug!("[local_proxy] no upstream configured, forcing DIRECT for {host}:{port}");
                    connect_direct(&host, port).await
                }
            }
        }
    };

    let upstream_stream = match upstream {
        Ok(s) => s,
        Err(e) => {
            warn!("[local_proxy] CONNECT {host}:{port} failed: {e}");
            // self-heal：cache flip 给下次机会（仅当 cache 命中时才有意义）
            if matches!(decision.source, super::route_resolver::DecisionSource::Cache) {
                let _ = inner.resolver.cache().flip(&host);
            }
            write_local_reply(&mut client, s5::REP_CONNECT_REFUSED).await?;
            return Ok(());
        }
    };
    write_local_reply(&mut client, s5::REP_OK).await?;

    // ---- step 5: relay ----
    let sink = inner.sink.lock().await.clone();
    let _bytes = bidirectional_relay(client, upstream_stream, sink).await;
    Ok(())
}

async fn connect_direct(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let target = format!("{host}:{port}");
    timeout(DIRECT_CONNECT_TIMEOUT, TcpStream::connect(target))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "direct connect timeout"))?
}

/// 嵌套 SOCKS5 握手：本机 → 上游 server SOCKS5 → 目标 host:port。
async fn connect_via_proxy(
    upstream: &ServerEndpoint,
    host: &str,
    port: u16,
) -> std::io::Result<TcpStream> {
    let mut s = timeout(
        PROXY_CONNECT_TIMEOUT,
        TcpStream::connect((upstream.host.as_str(), upstream.socks_port)),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "proxy connect timeout"))??;

    s.write_all(&encode_method_request_no_auth()).await?;
    let mut neg_resp = [0u8; 2];
    s.read_exact(&mut neg_resp).await?;
    parse_method_response(&neg_resp).map_err(|e| {
        std::io::Error::other(format!("upstream socks5 method negotiation: {e}"))
    })?;

    let req = encode_connect_request(&Socks5Address::Domain(host.to_string()), port)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
    s.write_all(&req).await?;

    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    let atyp = parse_reply_head(&head)
        .map_err(|e| std::io::Error::other(format!("upstream socks5 reply: {e}")))?;

    // 跳过 BND.ADDR：固定长度直接读；DOMAIN 先读 1 字节长度再按值读。
    match bnd_addr_len(atyp) {
        Ok(Some(len)) => {
            let mut b = vec![0u8; len];
            s.read_exact(&mut b).await?;
        }
        Ok(None) => {
            let mut len_buf = [0u8; 1];
            s.read_exact(&mut len_buf).await?;
            let mut name = vec![0u8; len_buf[0] as usize];
            s.read_exact(&mut name).await?;
        }
        Err(e) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("upstream socks5 reply: {e}"),
            ));
        }
    }
    let mut port_buf = [0u8; 2];
    s.read_exact(&mut port_buf).await?;
    Ok(s)
}

/// 给本地 SOCKS5 客户端回简化 reply（BND.ADDR/PORT 全 0），调用 conduit-core 编码。
async fn write_local_reply(client: &mut TcpStream, rep: u8) -> std::io::Result<()> {
    client
        .write_all(&encode_reply(rep, s5::ATYP_IPV4, &[0, 0, 0, 0], 0))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::route_cache::RouteCache;

    #[tokio::test]
    async fn server_endpoint_label_format() {
        let ep = ServerEndpoint {
            host: "10.1.2.3".into(),
            socks_port: 16615,
        };
        assert_eq!(ep.label(), "10.1.2.3:16615");
    }

    #[tokio::test]
    async fn local_proxy_starts_on_ephemeral_port() {
        let resolver = RouteResolver::new(RouteCache::new());
        let proxy = LocalProxy::new("127.0.0.1".into(), 0, resolver);
        let port = proxy.start().await.unwrap();
        assert!(port > 0);
        assert_eq!(proxy.actual_port().await, port);
        proxy.stop().await;
    }

    #[tokio::test]
    async fn set_get_endpoint_roundtrip() {
        let resolver = RouteResolver::new(RouteCache::new());
        let proxy = LocalProxy::new("127.0.0.1".into(), 0, resolver);
        assert!(proxy.current_endpoint().await.is_none());
        proxy
            .set_server_endpoint(Some(ServerEndpoint {
                host: "10.0.0.5".into(),
                socks_port: 16615,
            }))
            .await;
        let got = proxy.current_endpoint().await.unwrap();
        assert_eq!(got.host, "10.0.0.5");
        assert_eq!(got.socks_port, 16615);
    }
}
