//! `healthz` —— 通用 HTTP healthz 轮询，server-app / client-app 共用。
//!
//! 之前在 server-app/src-tauri/src/healthz.rs 与 client-app/src-tauri/src/healthz.rs
//! 各有一份完全相同副本，现在下沉到 conduit-core，两端 re-export。
//!
//! 设计要点：
//! - 错误类型本地化为 [`HealthzError`]，不污染 [`crate::ConduitError`]
//!   （boot 流程错误属于 tauri shell 关注点，不属于代理业务错误）。
//! - 200ms 轮询 + 1500ms 单次请求超时（boot 流程对延迟较敏感）。
//! - 调用方拿到 `Err` 后通常直接 `.to_string()` 用于 UI emit + log。

use std::time::Duration;

use log::{debug, info};
use thiserror::Error;

const POLL_INTERVAL_MS: u64 = 200;
const REQUEST_TIMEOUT_MS: u64 = 1500;

#[derive(Debug, Error)]
pub enum HealthzError {
    #[error("healthz timeout after {0}s")]
    Timeout(u64),
    #[error("http client build: {0}")]
    Build(String),
}

/// 阻塞轮询 `http://127.0.0.1:{api_port}/healthz` 直到返回 2xx 或 `max_seconds` 超时。
///
/// 返回 `Ok(())` 表示就绪；返回 `Err(HealthzError::Timeout(_))` 表示超时。
pub async fn wait_until_ready(api_port: u16, max_seconds: u64) -> Result<(), HealthzError> {
    let url = format!("http://127.0.0.1:{}/healthz", api_port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(REQUEST_TIMEOUT_MS))
        .build()
        .map_err(|e| HealthzError::Build(e.to_string()))?;

    let deadline = std::time::Instant::now() + Duration::from_secs(max_seconds);
    let mut attempts: u32 = 0;
    loop {
        attempts += 1;
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                info!("healthz ready after {} attempts ({})", attempts, url);
                return Ok(());
            }
            Ok(resp) => debug!("healthz attempt {} -> http {}", attempts, resp.status()),
            Err(e) => debug!("healthz attempt {} -> {}", attempts, e),
        }

        if std::time::Instant::now() >= deadline {
            return Err(HealthzError::Timeout(max_seconds));
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[tokio::test]
    async fn timeout_when_no_server_listening() {
        // 拿一个绝对没人监听的端口（占住后立刻丢）→ 但这里要的是"一直被拒绝"。
        // 用 127.0.0.1:1（reserved，多数系统没监听）作为简单替身。
        let r = wait_until_ready(1, 1).await;
        match r {
            Err(HealthzError::Timeout(1)) => {}
            other => panic!("expected Timeout(1), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn returns_ok_when_healthz_serves_200() {
        // 构造一个一次性 HTTP server 在 ephemeral port 上等一次连接、回 200。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let server_task = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            // 至少响应若干次（轮询会重试）
            for _ in 0..3 {
                if let Ok((mut s, _)) = listener.accept().await {
                    let _ = s
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await;
                    let _ = s.shutdown().await;
                }
            }
        });

        let r = wait_until_ready(port, 5).await;
        assert!(r.is_ok(), "expected Ok, got {r:?}");
        server_task.abort();
    }

    #[test]
    fn error_display_matches() {
        assert_eq!(HealthzError::Timeout(30).to_string(), "healthz timeout after 30s");
        assert!(HealthzError::Build("x".into()).to_string().starts_with("http client build"));
    }
}
