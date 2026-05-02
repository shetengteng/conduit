use std::time::Duration;

use log::{debug, info};

use crate::error::ConduitError;

const POLL_INTERVAL_MS: u64 = 200;

/// 阻塞轮询 client_main.py 控制 API 的 /healthz 端点直到成功或超时。
pub async fn wait_until_ready(api_port: u16, max_seconds: u64) -> Result<(), ConduitError> {
    let url = format!("http://127.0.0.1:{}/healthz", api_port);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
        .map_err(ConduitError::Http)?;

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
            return Err(ConduitError::HealthzTimeout(max_seconds));
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
