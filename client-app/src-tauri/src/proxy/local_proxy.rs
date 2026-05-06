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
    // ---- step 1: 协商方法 ----
    timeout(HANDSHAKE_TIMEOUT, async {
        let mut head = [0u8; 2];
        client.read_exact(&mut head).await?;
        if head[0] != 0x05 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not socks5",
            ));
        }
        let nmethods = head[1] as usize;
        let mut methods = vec![0u8; nmethods];
        client.read_exact(&mut methods).await?;
        // 一律回 NO-AUTH（0x00）
        client.write_all(&[0x05, 0x00]).await?;
        Ok::<(), std::io::Error>(())
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "method negotiation timeout"))??;

    // ---- step 2: 解析请求 ----
    let mut req_head = [0u8; 4];
    client.read_exact(&mut req_head).await?;
    if req_head[0] != 0x05 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad req version",
        ));
    }
    if req_head[1] != 0x01 {
        // 仅接受 SOCKS5 CONNECT；其他命令一律回 0x07 command-not-supported
        write_reply(&mut client, 0x07).await?;
        return Ok(());
    }
    let atyp = req_head[3];
    let host = match atyp {
        0x01 => {
            let mut buf = [0u8; 4];
            client.read_exact(&mut buf).await?;
            std::net::Ipv4Addr::from(buf).to_string()
        }
        0x04 => {
            let mut buf = [0u8; 16];
            client.read_exact(&mut buf).await?;
            std::net::Ipv6Addr::from(buf).to_string()
        }
        0x03 => {
            let mut len_buf = [0u8; 1];
            client.read_exact(&mut len_buf).await?;
            let mut name = vec![0u8; len_buf[0] as usize];
            client.read_exact(&mut name).await?;
            String::from_utf8_lossy(&name).to_string()
        }
        _ => {
            write_reply(&mut client, 0x08).await?;
            return Ok(());
        }
    };
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
            write_reply(&mut client, 0x05).await?; // connection refused
            return Ok(());
        }
    };
    write_reply(&mut client, 0x00).await?; // succeeded

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

    // 方法协商：[ver=5, nmethods=1, methods=[0x00 no-auth]]
    s.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut neg_resp = [0u8; 2];
    s.read_exact(&mut neg_resp).await?;
    if neg_resp != [0x05, 0x00] {
        return Err(std::io::Error::other(
            "upstream socks5: no-auth not accepted",
        ));
    }

    // 请求 CONNECT host:port，地址类型用 DOMAIN
    let host_bytes = host.as_bytes();
    if host_bytes.len() > 255 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "host too long for socks5 domain",
        ));
    }
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8];
    req.extend_from_slice(host_bytes);
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;

    // 响应：[ver, rep, rsv, atyp, bnd_addr, bnd_port]
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0x00 {
        return Err(std::io::Error::other(format!(
            "upstream socks5 reply rep={:#x}",
            head[1]
        )));
    }
    // 跳过 bnd_addr
    match head[3] {
        0x01 => {
            let mut b = [0u8; 4];
            s.read_exact(&mut b).await?;
        }
        0x04 => {
            let mut b = [0u8; 16];
            s.read_exact(&mut b).await?;
        }
        0x03 => {
            let mut len_buf = [0u8; 1];
            s.read_exact(&mut len_buf).await?;
            let mut name = vec![0u8; len_buf[0] as usize];
            s.read_exact(&mut name).await?;
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "upstream socks5: bad atyp in reply",
            ))
        }
    }
    let mut port_buf = [0u8; 2];
    s.read_exact(&mut port_buf).await?;
    Ok(s)
}

async fn write_reply(client: &mut TcpStream, rep: u8) -> std::io::Result<()> {
    // 简化：BND.ADDR/PORT 全部 0
    client
        .write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
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
