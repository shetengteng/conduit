//! `TrafficMeter` —— 累计 client 端 SOCKS5 隧道流量。
//!
//! 简化版（不上 600s 滚动窗，UI 走 8 秒轮询自己拉总量）：维护单调累加的
//! `sent_total` / `recv_total`，并在每次累加后通过 EventBus 推 `traffic_tick`。
//! 历史窗口数据由 control_api 占位空 series（同 server-app 的 TrafficResponse）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use conduit_core::{EventBus, ProgressSink};

use super::core::ClientEvent;

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
}

impl ProgressSink for TrafficMeter {
    fn on_progress(&self, sent_delta: u64, recv_delta: u64) {
        if sent_delta == 0 && recv_delta == 0 {
            return;
        }
        let new_sent = self.inner.sent.fetch_add(sent_delta, Ordering::Relaxed) + sent_delta;
        let new_recv = self.inner.recv.fetch_add(recv_delta, Ordering::Relaxed) + recv_delta;
        let payload = serde_json::json!({
            "sent_total": new_sent,
            "recv_total": new_recv,
            "sent_delta": sent_delta,
            "recv_delta": recv_delta,
        });
        self.inner.bus.publish(ClientEvent {
            kind: "traffic_tick".into(),
            payload,
            ts: epoch_now(),
        });
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

    #[test]
    fn on_progress_accumulates_and_emits() {
        let bus: EventBus<ClientEvent> = EventBus::new(16);
        let mut sub = bus.subscribe();
        let meter = TrafficMeter::new(bus);
        meter.on_progress(100, 200);
        meter.on_progress(50, 75);
        assert_eq!(meter.snapshot(), (150, 275));
        let mut events = 0;
        while sub.try_recv().is_ok() {
            events += 1;
        }
        assert_eq!(events, 2);
    }

    #[test]
    fn zero_delta_does_not_emit() {
        let bus: EventBus<ClientEvent> = EventBus::new(16);
        let mut sub = bus.subscribe();
        let meter = TrafficMeter::new(bus);
        meter.on_progress(0, 0);
        assert_eq!(meter.snapshot(), (0, 0));
        assert!(sub.try_recv().is_err());
    }

    #[test]
    fn reset_clears_counters() {
        let bus: EventBus<ClientEvent> = EventBus::new(8);
        let meter = TrafficMeter::new(bus);
        meter.on_progress(1, 2);
        meter.reset();
        assert_eq!(meter.snapshot(), (0, 0));
    }
}
