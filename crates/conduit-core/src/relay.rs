//! 双向字节流转发（server 与 client 共用，HTTP CONNECT / SOCKS5 都靠这个 relay）。
//!
//! - 单向 64 KiB chunk 拷贝。
//! - 任一方向断开时尽量发送 EOF（half-close）让对端感知。
//! - 可选 [`ProgressSink`]：上行/下行各自累计字节数，一并交给注册表算速率。
//!
//! 设计要点（参见 `design/2026-05-06-2-Conduit-Rust-重写设计文档.md` §4.2）：
//! - 不抛 `io::Error`：转发结束的常态就是对端 close，调用方关心的是总字节数；
//!   如果两个方向都立刻失败也只会得到 `(0, 0)`，调用方据此清理 socket 即可。
//! - 同步 `ProgressSink`（通过 `Arc<dyn ProgressSink>` 在两个 half-pipe 之间共享），
//!   注册表内部用 `Mutex<u64>` 或 `AtomicU64` 自己累加即可。

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 单方向 chunk 大小。
pub const CHUNK: usize = 65536;

/// 上行/下行字节增量回调。`sent_delta` 是 a→b 方向，`recv_delta` 是 b→a 方向。
///
/// 实现需自带内部锁（如 `AtomicU64` / `Mutex`），因为两个 half-pipe 任务会并发调用。
pub trait ProgressSink: Send + Sync + 'static {
    fn on_progress(&self, sent_delta: u64, recv_delta: u64);
}

async fn half_pipe<R, W>(
    mut reader: R,
    mut writer: W,
    is_upstream: bool,
    sink: Option<Arc<dyn ProgressSink>>,
) -> u64
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    let mut total: u64 = 0;
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if writer.write_all(&buf[..n]).await.is_err() {
            break;
        }
        total += n as u64;
        if let Some(s) = sink.as_deref() {
            if is_upstream {
                s.on_progress(n as u64, 0);
            } else {
                s.on_progress(0, n as u64);
            }
        }
    }
    let _ = writer.shutdown().await;
    total
}

/// 双向转发 `a <-> b` 直到任一方向 EOF 或 IO 错误。
///
/// 返回 `(bytes_a_to_b, bytes_b_to_a)`。
///
/// `sink` 可选：传入 `Some(Arc::new(MySink))` 时每个 chunk 写入成功后会同步通知，
/// 上行（a→b）报 `(n, 0)`，下行（b→a）报 `(0, n)`。
pub async fn bidirectional_relay<A, B>(
    a: A,
    b: B,
    sink: Option<Arc<dyn ProgressSink>>,
) -> (u64, u64)
where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (a_reader, a_writer) = tokio::io::split(a);
    let (b_reader, b_writer) = tokio::io::split(b);
    let sink_clone = sink.clone();
    let h1 = tokio::spawn(async move { half_pipe(a_reader, b_writer, true, sink_clone).await });
    let h2 = tokio::spawn(async move { half_pipe(b_reader, a_writer, false, sink).await });
    let a_to_b = h1.await.unwrap_or(0);
    let b_to_a = h2.await.unwrap_or(0);
    (a_to_b, b_to_a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::duplex;

    struct AtomicSink {
        sent: AtomicU64,
        recv: AtomicU64,
    }

    impl ProgressSink for AtomicSink {
        fn on_progress(&self, sent_delta: u64, recv_delta: u64) {
            if sent_delta > 0 {
                self.sent.fetch_add(sent_delta, Ordering::Relaxed);
            }
            if recv_delta > 0 {
                self.recv.fetch_add(recv_delta, Ordering::Relaxed);
            }
        }
    }

    /// 构造两组 in-memory duplex pipes 模拟 client <-> proxy <-> upstream 的拓扑：
    /// - `client_side` <-> `client_seen_by_proxy`
    /// - `upstream_side` <-> `upstream_seen_by_proxy`
    ///
    /// proxy 内部把 `client_seen_by_proxy` 与 `upstream_seen_by_proxy` 用 relay 连起来。
    fn make_link(buf: usize) -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        duplex(buf)
    }

    #[tokio::test]
    async fn forwards_payload_in_both_directions() {
        let (mut client_side, client_seen_by_proxy) = make_link(8192);
        let (mut upstream_side, upstream_seen_by_proxy) = make_link(8192);

        let relay = tokio::spawn(async move {
            bidirectional_relay(client_seen_by_proxy, upstream_seen_by_proxy, None).await
        });

        client_side.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        client_side.shutdown().await.unwrap();

        let mut from_client = Vec::new();
        upstream_side.read_to_end(&mut from_client).await.unwrap();
        assert_eq!(&from_client, b"GET / HTTP/1.1\r\n\r\n");

        upstream_side.write_all(b"HTTP/1.1 200 OK\r\n\r\nhi").await.unwrap();
        upstream_side.shutdown().await.unwrap();
        let mut from_upstream = Vec::new();
        client_side.read_to_end(&mut from_upstream).await.unwrap();
        assert_eq!(&from_upstream, b"HTTP/1.1 200 OK\r\n\r\nhi");

        let (a_to_b, b_to_a) = relay.await.unwrap();
        assert_eq!(a_to_b, b"GET / HTTP/1.1\r\n\r\n".len() as u64);
        assert_eq!(b_to_a, b"HTTP/1.1 200 OK\r\n\r\nhi".len() as u64);
    }

    #[tokio::test]
    async fn progress_sink_accumulates_correctly() {
        let (mut client_side, client_seen_by_proxy) = make_link(8192);
        let (mut upstream_side, upstream_seen_by_proxy) = make_link(8192);

        let sink = Arc::new(AtomicSink {
            sent: AtomicU64::new(0),
            recv: AtomicU64::new(0),
        });

        let relay = tokio::spawn({
            let sink: Arc<dyn ProgressSink> = sink.clone();
            async move {
                bidirectional_relay(client_seen_by_proxy, upstream_seen_by_proxy, Some(sink))
                    .await
            }
        });

        client_side.write_all(&[7u8; 1024]).await.unwrap();
        client_side.shutdown().await.unwrap();
        let mut sink_buf = Vec::new();
        upstream_side.read_to_end(&mut sink_buf).await.unwrap();

        upstream_side.write_all(&[9u8; 2048]).await.unwrap();
        upstream_side.shutdown().await.unwrap();
        let mut down = Vec::new();
        client_side.read_to_end(&mut down).await.unwrap();

        let (a_to_b, b_to_a) = relay.await.unwrap();
        assert_eq!(a_to_b, 1024);
        assert_eq!(b_to_a, 2048);
        assert_eq!(sink.sent.load(Ordering::Relaxed), 1024);
        assert_eq!(sink.recv.load(Ordering::Relaxed), 2048);
    }

    #[tokio::test]
    async fn handles_large_chunked_payload() {
        let (mut client_side, client_seen_by_proxy) = make_link(8192);
        let (mut upstream_side, upstream_seen_by_proxy) = make_link(8192);

        let relay = tokio::spawn(async move {
            bidirectional_relay(client_seen_by_proxy, upstream_seen_by_proxy, None).await
        });

        // 250 KiB > CHUNK，强制走多次 read_loop。
        let payload = vec![42u8; 250 * 1024];
        let payload_len = payload.len() as u64;

        let writer_task = tokio::spawn({
            let payload = payload.clone();
            async move {
                client_side.write_all(&payload).await.unwrap();
                client_side.shutdown().await.unwrap();
                // 保留 client_side 不被 drop，避免 DuplexStream 关闭反向通道。
                drop(client_side);
            }
        });

        let mut received = Vec::new();
        upstream_side.read_to_end(&mut received).await.unwrap();
        writer_task.await.unwrap();

        // upstream 直接关闭，让反向 half-pipe 也收 EOF 自然退出。
        upstream_side.shutdown().await.unwrap();
        drop(upstream_side);

        let (a_to_b, b_to_a) = relay.await.unwrap();
        assert_eq!(a_to_b, payload_len);
        assert_eq!(b_to_a, 0);
        assert_eq!(received.len(), payload.len());
        assert!(received.iter().all(|&b| b == 42));
    }
}
