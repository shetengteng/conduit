//! `TrafficMeter` —— 累计 client 端 SOCKS5 隧道流量。
//!
//! 简化版（不上 600s 滚动窗，UI 走 8 秒轮询自己拉总量）：维护单调累加的
//! `sent` / `recv`,并由 [`spawn_emitter`] 起的协程**1Hz 聚合 publish**
//! `traffic_tick` event。
//!
//! 性能要点(v0.2.2 之后):
//! - 之前版本 `on_progress` 每个 64 KiB chunk 都 `serde_json::json!` + bus
//!   publish。在大下载 (100MB/s ~ 1600 chunks/s) 场景下,每秒数千次 JSON
//!   构造 + broadcast send + SSE socket write 会显著挤占 relay 线程,导致
//!   下载速率骤降(用户实测从原始 ~100MB/s 跌到几 MB/s)。
//! - 现版本 `on_progress` 只做 atomic fetch_add(零分配,~10ns),所有 SSE
//!   广播挪到 1Hz 协程里——一秒钟最多 1 次 JSON 序列化 + broadcast,
//!   relay 热路径恢复零分配。
//! - 1Hz 节奏与 UI `TrafficChart` 60 个采样窗口/60 秒的渲染节拍 1:1 对齐,
//!   也与 server-app `traffic_emitter` 一致。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use conduit_core::{EventBus, ProgressSink};
use tokio_util::sync::CancellationToken;

use super::core::ClientEvent;

const TICK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct TrafficMeter {
    inner: Arc<Inner>,
}

struct Inner {
    bus: EventBus<ClientEvent>,
    sent: AtomicU64,
    recv: AtomicU64,
}

impl TrafficMeter {
    pub fn new(bus: EventBus<ClientEvent>) -> Self {
        Self {
            inner: Arc::new(Inner {
                bus,
                sent: AtomicU64::new(0),
                recv: AtomicU64::new(0),
            }),
        }
    }

    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.inner.sent.load(Ordering::Relaxed),
            self.inner.recv.load(Ordering::Relaxed),
        )
    }

    pub fn reset(&self) {
        self.inner.sent.store(0, Ordering::Relaxed);
        self.inner.recv.store(0, Ordering::Relaxed);
    }

    /// 启动 1Hz 聚合发射协程: 取出 since-last 增量, publish 一次 traffic_tick。
    /// 静默期(本秒无任何字节)且上一秒也静默时跳过 publish, 避免空闲打扰 SSE。
    pub fn spawn_emitter(&self, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut last_sent: u64 = 0;
            let mut last_recv: u64 = 0;
            let mut last_nonzero = false;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(TICK_INTERVAL) => {}
                }
                let total_sent = inner.sent.load(Ordering::Relaxed);
                let total_recv = inner.recv.load(Ordering::Relaxed);
                let dsent = total_sent.saturating_sub(last_sent);
                let drecv = total_recv.saturating_sub(last_recv);
                last_sent = total_sent;
                last_recv = total_recv;
                let any_nonzero = dsent > 0 || drecv > 0;
                let should_publish = any_nonzero || last_nonzero;
                last_nonzero = any_nonzero;
                if !should_publish {
                    continue;
                }
                let ts = epoch_now();
                let payload = serde_json::json!({
                    "ts": ts,
                    "uplink_bytes": dsent,
                    "downlink_bytes": drecv,
                    "total_uplink": total_sent,
                    "total_downlink": total_recv,
                });
                inner.bus.publish(ClientEvent {
                    kind: "traffic_tick".into(),
                    payload,
                    ts,
                });
            }
        })
    }
}

impl ProgressSink for TrafficMeter {
    fn on_progress(&self, sent_delta: u64, recv_delta: u64) {
        // 数据中转热路径——只做原子累加, 零分配 / 不广播。
        // SSE 广播由 [`Self::spawn_emitter`] 1Hz 协程聚合完成。
        if sent_delta > 0 {
            self.inner.sent.fetch_add(sent_delta, Ordering::Relaxed);
        }
        if recv_delta > 0 {
            self.inner.recv.fetch_add(recv_delta, Ordering::Relaxed);
        }
    }
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
    use tokio::sync::broadcast::error::RecvError;

    #[test]
    fn on_progress_accumulates_without_publishing_in_hot_path() {
        // 热路径不再 publish: 1000 次累加只有 atomic fetch_add, 不应有任何 SSE event 产生。
        // 这是 v0.2.2 的关键性能修复——之前每 chunk publish 在大下载场景里会
        // 把下载速率打到几 MB/s。
        let bus: EventBus<ClientEvent> = EventBus::new(16);
        let mut sub = bus.subscribe();
        let meter = TrafficMeter::new(bus);
        for _ in 0..1000 {
            meter.on_progress(100, 200);
        }
        assert_eq!(meter.snapshot(), (100_000, 200_000));
        assert!(matches!(sub.try_recv(), Err(_)));
    }

    #[test]
    fn zero_delta_is_noop() {
        let bus: EventBus<ClientEvent> = EventBus::new(16);
        let mut sub = bus.subscribe();
        let meter = TrafficMeter::new(bus);
        meter.on_progress(0, 0);
        assert_eq!(meter.snapshot(), (0, 0));
        assert!(matches!(sub.try_recv(), Err(_)));
    }

    #[test]
    fn reset_clears_counters() {
        let bus: EventBus<ClientEvent> = EventBus::new(8);
        let meter = TrafficMeter::new(bus);
        meter.on_progress(1, 2);
        meter.reset();
        assert_eq!(meter.snapshot(), (0, 0));
    }

    /// spawn_emitter 协程应当在收到流量后 1Hz 聚合 publish 一次 traffic_tick,
    /// payload 字段和量级与累加结果对齐(uplink_bytes=本秒增量, total_uplink=累计)。
    #[tokio::test]
    async fn emitter_publishes_aggregated_tick_every_second() {
        let bus: EventBus<ClientEvent> = EventBus::new(16);
        let mut sub = bus.subscribe();
        let meter = TrafficMeter::new(bus);
        for _ in 0..10 {
            meter.on_progress(2048, 4096);
        }
        let cancel = CancellationToken::new();
        let h = meter.spawn_emitter(cancel.clone());

        let evt = tokio::time::timeout(Duration::from_millis(2500), async {
            loop {
                match sub.recv().await {
                    Ok(e) if e.kind == "traffic_tick" => return e,
                    Ok(_) => continue,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => panic!("event bus closed"),
                }
            }
        })
        .await
        .expect("expected traffic_tick within 2.5s");
        cancel.cancel();
        let _ = h.await;

        assert_eq!(evt.payload["uplink_bytes"].as_u64().unwrap(), 20_480);
        assert_eq!(evt.payload["downlink_bytes"].as_u64().unwrap(), 40_960);
        assert_eq!(evt.payload["total_uplink"].as_u64().unwrap(), 20_480);
        assert_eq!(evt.payload["total_downlink"].as_u64().unwrap(), 40_960);
    }

    /// 静默期不应刷屏发 0 帧 —— 仅在有流量 OR 上一秒有流量时 publish。
    #[tokio::test]
    async fn emitter_does_not_publish_when_idle() {
        let bus: EventBus<ClientEvent> = EventBus::new(16);
        let mut sub = bus.subscribe();
        let meter = TrafficMeter::new(bus);
        let cancel = CancellationToken::new();
        let h = meter.spawn_emitter(cancel.clone());
        let res = tokio::time::timeout(Duration::from_millis(2200), async {
            loop {
                match sub.recv().await {
                    Ok(e) if e.kind == "traffic_tick" => return Some(e),
                    Ok(_) => continue,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                }
            }
        })
        .await;
        cancel.cancel();
        let _ = h.await;
        assert!(res.is_err(), "should not publish when idle");
    }
}
