//! SOCKS5 server —— 平移自 Python `server-app/core/socks5_proxy.py`。
//!
//! 实现范围（与 Python 等价）：
//! - RFC 1928 method 协商（仅支持 NO-AUTH）
//! - CMD = CONNECT，其它命令回 `0x07 Command not supported`
//! - 地址类型：IPv4 / IPv6 / DOMAIN
//! - 端口白名单 → `0x02 Connection not allowed by ruleset`
//! - 上游 connect 失败：`0x05 Connection refused` / `0x03 Network unreachable`
//!   / `0x06 TTL expired`（超时）
//! - bind reply 用 upstream 实际 local addr（不可解析时回 0.0.0.0:0）
//!
//! 不实现 BIND / UDP ASSOCIATE，与 Python 端一致。

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use conduit_core::{bidirectional_relay, ProgressSink};
use log::{debug, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use super::config::ProxyConfig;
use super::core::ProxyCore;
use super::session::SessionRegistry;

const VER: u8 = 0x05;
const NO_AUTH: u8 = 0x00;
const NO_ACCEPTABLE: u8 = 0xFF;

const CMD_CONNECT: u8 = 0x01;

const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

const REP_OK: u8 = 0x00;
const REP_GENERAL: u8 = 0x01;
const REP_NOT_ALLOWED: u8 = 0x02;
const REP_NETWORK_UNREACH: u8 = 0x03;
const REP_CONNECT_REFUSED: u8 = 0x05;
const REP_TTL_EXPIRED: u8 = 0x06;
const REP_CMD_NOT_SUPPORTED: u8 = 0x07;
const REP_ATYP_NOT_SUPPORTED: u8 = 0x08;

/// SOCKS5 accept loop：监听 `cfg.bind:cfg.socks_port`，单连接独立 spawn。
pub async fn run(
    core: ProxyCore,
    cancel: CancellationToken,
    sessions: Arc<SessionRegistry>,
) -> std::io::Result<()> {
    let cfg = core.config();
    let bind = format!("{}:{}", cfg.bind, cfg.socks_port);
    let listener = TcpListener::bind(&bind).await?;
    info!("[socks5] listening on {bind}");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("[socks5] cancellation requested, shutting down accept loop");
                break;
            }
            res = listener.accept() => {
                let (sock, peer) = match res {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("[socks5] accept error: {e}");
                        continue;
                    }
                };
                let cfg = cfg.clone();
                let sessions = sessions.clone();
                tokio::spawn(async move {
                    let peer_ip = peer.ip().to_string();
                    if let Err(e) = handle_connection(sock, peer_ip.clone(), cfg, sessions).await {
                        debug!("[socks5] {peer_ip} session ended: {e}");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle_connection(
    mut client: TcpStream,
    peer_ip: String,
    cfg: Arc<ProxyConfig>,
    sessions: Arc<SessionRegistry>,
) -> std::io::Result<()> {
    if !cfg.is_client_allowed(&peer_ip) {
        warn!("[socks5] reject {peer_ip} (not in allowed_cidrs)");
        return Ok(());
    }
    let timeout = Duration::from_secs_f64(cfg.handshake_timeout_s);

    // ─── method 协商 ───
    let mut head = [0u8; 2];
    if let Err(e) = read_exact_with_timeout(&mut client, &mut head, timeout).await {
        debug!("[socks5] {peer_ip} read greeting failed: {e}");
        return Ok(());
    }
    let (ver, nmethods) = (head[0], head[1]);
    if ver != VER || nmethods == 0 {
        return Ok(());
    }
    let mut methods = vec![0u8; nmethods as usize];
    if let Err(e) = read_exact_with_timeout(&mut client, &mut methods, timeout).await {
        debug!("[socks5] {peer_ip} read methods failed: {e}");
        return Ok(());
    }
    if !methods.contains(&NO_AUTH) {
        let _ = client.write_all(&[VER, NO_ACCEPTABLE]).await;
        return Ok(());
    }
    client.write_all(&[VER, NO_AUTH]).await?;

    // ─── request 头 (VER, CMD, RSV, ATYP) ───
    let mut head4 = [0u8; 4];
    if let Err(e) = read_exact_with_timeout(&mut client, &mut head4, timeout).await {
        debug!("[socks5] {peer_ip} read request head failed: {e}");
        return Ok(());
    }
    let (req_ver, cmd, _rsv, atyp) = (head4[0], head4[1], head4[2], head4[3]);
    if req_ver != VER {
        return Ok(());
    }
    if cmd != CMD_CONNECT {
        let _ = client.write_all(&reply(REP_CMD_NOT_SUPPORTED, ATYP_IPV4, &[0u8; 4], 0)).await;
        return Ok(());
    }

    // ─── address ───
    let host: String = match atyp {
        ATYP_IPV4 => {
            let mut raw = [0u8; 4];
            read_exact_with_timeout(&mut client, &mut raw, timeout).await?;
            std::net::Ipv4Addr::from(raw).to_string()
        }
        ATYP_IPV6 => {
            let mut raw = [0u8; 16];
            read_exact_with_timeout(&mut client, &mut raw, timeout).await?;
            std::net::Ipv6Addr::from(raw).to_string()
        }
        ATYP_DOMAIN => {
            let mut len_buf = [0u8; 1];
            read_exact_with_timeout(&mut client, &mut len_buf, timeout).await?;
            let ln = len_buf[0] as usize;
            if ln == 0 {
                let _ = client.write_all(&reply(REP_GENERAL, ATYP_IPV4, &[0u8; 4], 0)).await;
                return Ok(());
            }
            let mut name = vec![0u8; ln];
            read_exact_with_timeout(&mut client, &mut name, timeout).await?;
            String::from_utf8_lossy(&name).into_owned()
        }
        _ => {
            let _ = client.write_all(&reply(REP_ATYP_NOT_SUPPORTED, ATYP_IPV4, &[0u8; 4], 0)).await;
            return Ok(());
        }
    };
    // ─── port (big-endian u16) ───
    let mut port_raw = [0u8; 2];
    read_exact_with_timeout(&mut client, &mut port_raw, timeout).await?;
    let port = u16::from_be_bytes(port_raw);

    if !cfg.is_connect_port_allowed(port) {
        warn!("[socks5] {peer_ip} CONNECT {host}:{port} rejected (port not allowed)");
        let _ = client.write_all(&reply(REP_NOT_ALLOWED, ATYP_IPV4, &[0u8; 4], 0)).await;
        return Ok(());
    }

    info!("[socks5] CONNECT {host}:{port} from {peer_ip}");
    let upstream = match tokio::time::timeout(
        Duration::from_secs_f64(cfg.connect_timeout_s),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            let _ = client.write_all(&reply(REP_CONNECT_REFUSED, ATYP_IPV4, &[0u8; 4], 0)).await;
            return Ok(());
        }
        Ok(Err(e)) => {
            warn!("[socks5] connect {host}:{port} failed: {e}");
            let _ = client.write_all(&reply(REP_NETWORK_UNREACH, ATYP_IPV4, &[0u8; 4], 0)).await;
            return Ok(());
        }
        Err(_) => {
            let _ = client.write_all(&reply(REP_TTL_EXPIRED, ATYP_IPV4, &[0u8; 4], 0)).await;
            return Ok(());
        }
    };

    let (bnd_atyp, bnd_addr_bytes, bnd_port) = bnd_from_target(&upstream);
    client
        .write_all(&reply(REP_OK, bnd_atyp, &bnd_addr_bytes, bnd_port))
        .await?;
    client.flush().await?;

    let session = sessions
        .add(peer_ip.clone(), "socks5", format!("{host}:{port}"))
        .await;
    let sink: Arc<dyn ProgressSink> = sessions.clone().sink_for(session.session_id.clone());
    let (sent, recv) = bidirectional_relay(client, upstream, Some(sink)).await;
    sessions.remove(&session.session_id).await;
    info!("[socks5] CONNECT {host}:{port} from {peer_ip} closed: sent={sent}B recv={recv}B");
    Ok(())
}

/// 拼装 SOCKS5 reply：`VER REP RSV ATYP BND.ADDR BND.PORT`。
fn reply(rep: u8, atyp: u8, bnd_addr: &[u8], bnd_port: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(6 + bnd_addr.len());
    out.push(VER);
    out.push(rep);
    out.push(0x00);
    out.push(atyp);
    out.extend_from_slice(bnd_addr);
    out.extend_from_slice(&bnd_port.to_be_bytes());
    out
}

/// 从 upstream socket 的 local_addr 推 bind reply 字段。
fn bnd_from_target(upstream: &TcpStream) -> (u8, Vec<u8>, u16) {
    match upstream.local_addr() {
        Ok(addr) => match addr.ip() {
            IpAddr::V4(v4) => (ATYP_IPV4, v4.octets().to_vec(), addr.port()),
            IpAddr::V6(v6) => (ATYP_IPV6, v6.octets().to_vec(), addr.port()),
        },
        Err(_) => (ATYP_IPV4, vec![0, 0, 0, 0], 0),
    }
}

async fn read_exact_with_timeout(
    s: &mut TcpStream,
    buf: &mut [u8],
    timeout: Duration,
) -> std::io::Result<()> {
    tokio::time::timeout(timeout, s.read_exact(buf))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "socks5 read timeout"))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{ProxyConfig, ProxyCore};
    use std::time::Duration as StdDuration;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener as TL, TcpStream as TS};

    /// 启动 ProxyCore + 等 SOCKS5 listener 就绪。返回 (core, http_port, socks_port)。
    async fn start_test_core() -> (ProxyCore, u16, u16) {
        let http = portpicker::pick_unused_port().unwrap();
        let socks = portpicker::pick_unused_port().unwrap();
        let api = portpicker::pick_unused_port().unwrap();
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        cfg.allowed_connect_ports = (1..=65535).collect();
        cfg.handshake_timeout_s = 2.0;
        cfg.connect_timeout_s = 2.0;
        let core = ProxyCore::new(cfg);
        // 临时挂 SOCKS5 task（S2.6 才会进 ProxyCore::start）
        let cancel = core.cancel_token();
        let sessions = core.sessions();
        let core_clone = core.clone();
        tokio::spawn(async move {
            let _ = super::run(core_clone, cancel, sessions).await;
        });
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", socks)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        (core, http, socks)
    }

    #[tokio::test]
    async fn no_auth_negotiation_succeeds() {
        let (core, _http, socks) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", socks)).await.unwrap();
        s.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut resp = [0u8; 2];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, [0x05, 0x00]);
        core.stop().await;
    }

    #[tokio::test]
    async fn rejects_unsupported_method() {
        let (core, _http, socks) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", socks)).await.unwrap();
        // 只声明 USERNAME/PASSWORD (0x02)
        s.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
        let mut resp = [0u8; 2];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp, [0x05, 0xFF]);
        core.stop().await;
    }

    #[tokio::test]
    async fn rejects_non_connect_command() {
        let (core, _http, socks) = start_test_core().await;
        let mut s = TS::connect(("127.0.0.1", socks)).await.unwrap();
        s.write_all(&[0x05, 0x01, 0x00]).await.unwrap(); // 协商
        let mut greet = [0u8; 2];
        s.read_exact(&mut greet).await.unwrap();
        // 发 CMD=BIND (0x02)
        s.write_all(&[0x05, 0x02, 0x00, 0x01, 127, 0, 0, 1, 0x00, 0x50])
            .await
            .unwrap();
        let mut resp = [0u8; 10];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp[1], REP_CMD_NOT_SUPPORTED);
        core.stop().await;
    }

    #[tokio::test]
    async fn connect_to_disallowed_port_returns_not_allowed() {
        let http = portpicker::pick_unused_port().unwrap();
        let socks = portpicker::pick_unused_port().unwrap();
        let api = portpicker::pick_unused_port().unwrap();
        let mut cfg = ProxyConfig::with_ports(http, socks, api);
        cfg.bind = "127.0.0.1".into();
        // 不放开任意端口，使用默认白名单（不含 6379）
        cfg.handshake_timeout_s = 2.0;
        let core = ProxyCore::new(cfg);
        let cancel = core.cancel_token();
        let sessions = core.sessions();
        let core_clone = core.clone();
        tokio::spawn(async move {
            let _ = super::run(core_clone, cancel, sessions).await;
        });
        for _ in 0..50 {
            if TS::connect(("127.0.0.1", socks)).await.is_ok() {
                break;
            }
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }

        let mut s = TS::connect(("127.0.0.1", socks)).await.unwrap();
        s.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut g = [0u8; 2];
        s.read_exact(&mut g).await.unwrap();
        // CONNECT 127.0.0.1:6379
        s.write_all(&[0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, 0x18, 0xEB])
            .await
            .unwrap();
        let mut resp = [0u8; 10];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp[1], REP_NOT_ALLOWED);
        core.stop().await;
    }

    #[tokio::test]
    async fn connect_tunnels_payload_to_upstream_echo_server() {
        let (core, _http, socks) = start_test_core().await;
        // 上游 echo
        let echo_listener = TL::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo_listener.local_addr().unwrap().port();
        let echo_task = tokio::spawn(async move {
            if let Ok((mut sock, _)) = echo_listener.accept().await {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            }
        });

        let mut s = TS::connect(("127.0.0.1", socks)).await.unwrap();
        s.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
        let mut g = [0u8; 2];
        s.read_exact(&mut g).await.unwrap();
        // CONNECT 127.0.0.1:<echo_port>
        let port_be = echo_port.to_be_bytes();
        s.write_all(&[0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, port_be[0], port_be[1]])
            .await
            .unwrap();
        let mut resp = [0u8; 10];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(resp[1], REP_OK, "expected SUCCEEDED, got {resp:?}");

        s.write_all(b"socks-tunnel-payload").await.unwrap();
        s.shutdown().await.unwrap();
        let mut back = Vec::new();
        s.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, b"socks-tunnel-payload");

        let _ = echo_task.await;
        core.stop().await;
    }

    // 注：refused/unreachable 路径无法用 ephemeral-port + drop 稳定模拟（OS
    // 可能让 SYN 进入 backlog 或被并发测试占用），改由代码审查覆盖。
}
