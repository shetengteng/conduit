//! `socks5_relay_smoke` —— 无 GUI / 无 webview 的最小端到端冒烟测试。
//!
//! 跑法：
//! ```bash
//! cargo run -p conduit-core --example socks5_relay_smoke --release
//! ```
//!
//! 校验路径：
//! 1. 起本进程内的 echo TCP server（ephemeral port）。
//! 2. 起本进程内的极简 SOCKS5 server（NO-AUTH + CONNECT），用
//!    `conduit_core::socks5_proto` 解码 + `bidirectional_relay` 转发。
//! 3. 用 `conduit_core::socks5_proto` 拼一个 SOCKS5 client 帧，连接 SOCKS5 server，
//!    CONNECT 到 echo server。
//! 4. 通过 SOCKS5 隧道写 1 MB 随机字节，关闭 client 写入侧，等 echo 回写。
//! 5. 校验收到的字节序列与发送一致 + ProgressSink 计数与字节数对得上。
//!
//! 任何一步失败 → exit 1，stdout 印失败原因。
//!
//! 这个 example 故意不依赖 server-app / client-app 的 Tauri shell —— 它覆盖
//! `socks5_proto` 编解码 + `bidirectional_relay` 双向流量 + `ProgressSink`
//! 三个核心 building block，是 v0.2.0 e2e 的"headless 基线"。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use conduit_core::socks5_proto::{
    bnd_addr_len, consts as s5, encode_connect_request, encode_method_request_no_auth,
    encode_method_response, encode_reply, parse_address_bytes, parse_method_response,
    parse_reply_head, validate_version, Socks5Address,
};
use conduit_core::{bidirectional_relay, ProgressSink};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PAYLOAD_BYTES: usize = 1_048_576; // 1 MiB

#[derive(Default)]
struct CountingSink {
    sent: AtomicU64,
    recv: AtomicU64,
}

impl ProgressSink for CountingSink {
    fn on_progress(&self, sent_delta: u64, recv_delta: u64) {
        self.sent.fetch_add(sent_delta, Ordering::Relaxed);
        self.recv.fetch_add(recv_delta, Ordering::Relaxed);
    }
}

async fn spawn_echo_server() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    Ok(port)
}

async fn spawn_minimal_socks5_server(sink: Arc<CountingSink>) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let sink = sink.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_socks5_session(client, sink).await {
                    eprintln!("[smoke] socks5 session error: {e}");
                }
            });
        }
    });
    Ok(port)
}

async fn handle_socks5_session(
    mut client: TcpStream,
    sink: Arc<CountingSink>,
) -> std::io::Result<()> {
    // method negotiation
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    if validate_version(head[0]).is_err() {
        return Err(std::io::Error::other("not socks5"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    client.read_exact(&mut methods).await?;
    let no_auth_ok = methods.contains(&s5::NO_AUTH);
    client
        .write_all(&encode_method_response(no_auth_ok))
        .await?;
    if !no_auth_ok {
        return Err(std::io::Error::other("no acceptable method"));
    }

    // request head + address + port
    let mut req = [0u8; 4];
    client.read_exact(&mut req).await?;
    if validate_version(req[0]).is_err() || req[1] != s5::CMD_CONNECT {
        return Err(std::io::Error::other("expected CONNECT"));
    }
    let atyp = req[3];
    let address: Socks5Address = match atyp {
        s5::ATYP_IPV4 | s5::ATYP_IPV6 => {
            let len = bnd_addr_len(atyp).unwrap().unwrap();
            let mut raw = vec![0u8; len];
            client.read_exact(&mut raw).await?;
            parse_address_bytes(atyp, &raw).unwrap()
        }
        s5::ATYP_DOMAIN => {
            let mut len_buf = [0u8; 1];
            client.read_exact(&mut len_buf).await?;
            let mut name = vec![0u8; len_buf[0] as usize];
            client.read_exact(&mut name).await?;
            parse_address_bytes(atyp, &name).unwrap()
        }
        _ => return Err(std::io::Error::other("invalid atyp")),
    };
    let mut port_raw = [0u8; 2];
    client.read_exact(&mut port_raw).await?;
    let port = u16::from_be_bytes(port_raw);

    // dial upstream + 200 OK reply
    let upstream = TcpStream::connect((address.host_string().as_str(), port)).await?;
    client
        .write_all(&encode_reply(s5::REP_OK, s5::ATYP_IPV4, &[0, 0, 0, 0], 0))
        .await?;

    let sink_dyn: Arc<dyn ProgressSink> = sink.clone();
    let (sent, recv) = bidirectional_relay(client, upstream, Some(sink_dyn)).await;
    eprintln!("[smoke] session closed: sent={sent} recv={recv}");
    Ok(())
}

async fn run_client_through_proxy(socks_port: u16, target_port: u16) -> std::io::Result<Vec<u8>> {
    let mut s = TcpStream::connect(("127.0.0.1", socks_port)).await?;

    // method negotiation
    s.write_all(&encode_method_request_no_auth()).await?;
    let mut neg = [0u8; 2];
    s.read_exact(&mut neg).await?;
    parse_method_response(&neg)
        .map_err(|e| std::io::Error::other(format!("method neg: {e}")))?;

    // CONNECT 127.0.0.1:target_port
    let req = encode_connect_request(&Socks5Address::V4([127, 0, 0, 1]), target_port).unwrap();
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    let atyp = parse_reply_head(&head)
        .map_err(|e| std::io::Error::other(format!("reply: {e}")))?;
    // skip BND.ADDR + BND.PORT
    if let Some(len) = bnd_addr_len(atyp).unwrap() {
        let mut b = vec![0u8; len];
        s.read_exact(&mut b).await?;
    }
    let mut bp = [0u8; 2];
    s.read_exact(&mut bp).await?;

    // payload roundtrip
    let payload: Vec<u8> = (0..PAYLOAD_BYTES).map(|i| (i % 251) as u8).collect();
    s.write_all(&payload).await?;
    s.shutdown().await?;
    let mut echo_back = Vec::with_capacity(PAYLOAD_BYTES);
    s.read_to_end(&mut echo_back).await?;
    Ok(echo_back)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    eprintln!("[smoke] socks5 relay headless smoke starting...");
    let sink = Arc::new(CountingSink::default());

    let echo_port = match spawn_echo_server().await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[smoke] echo server bind failed: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[smoke] echo server on 127.0.0.1:{echo_port}");

    let socks_port = match spawn_minimal_socks5_server(sink.clone()).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[smoke] socks5 server bind failed: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("[smoke] socks5 server on 127.0.0.1:{socks_port}");

    // 给 listener task 一个 yield 机会先把 accept loop 跑起来。
    tokio::time::sleep(Duration::from_millis(50)).await;

    let echo_back = match run_client_through_proxy(socks_port, echo_port).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[smoke] client roundtrip failed: {e}");
            std::process::exit(1);
        }
    };

    if echo_back.len() != PAYLOAD_BYTES {
        eprintln!(
            "[smoke] FAIL: echoed {} bytes, expected {PAYLOAD_BYTES}",
            echo_back.len()
        );
        std::process::exit(1);
    }
    let payload: Vec<u8> = (0..PAYLOAD_BYTES).map(|i| (i % 251) as u8).collect();
    if echo_back != payload {
        eprintln!("[smoke] FAIL: echoed payload bytes mismatch");
        std::process::exit(1);
    }

    // ProgressSink 至少看到 PAYLOAD 字节的 sent + recv（双向，因为 echo 等量回写）。
    let sent = sink.sent.load(Ordering::Relaxed);
    let recv = sink.recv.load(Ordering::Relaxed);
    eprintln!(
        "[smoke] roundtrip OK ({PAYLOAD_BYTES} bytes); sink sent={sent}B recv={recv}B"
    );
    if sent < PAYLOAD_BYTES as u64 || recv < PAYLOAD_BYTES as u64 {
        eprintln!("[smoke] FAIL: ProgressSink under-counted");
        std::process::exit(1);
    }
    eprintln!("[smoke] ✓ socks5_relay_smoke PASS");
}
