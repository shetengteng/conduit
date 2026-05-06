//! 进程内事件总线（基于 `tokio::sync::broadcast`，多订阅 fan-out）。
//!
//! 基于 [`tokio::sync::broadcast`] 的多订阅 channel：单一 publisher（持有
//! `EventBus` 本身），多个 subscriber（通过 [`EventBus::subscribe`] 拿到
//! `Receiver<T>`）。容量满时旧消息被覆盖，订阅者会收到 `RecvError::Lagged`。
//!
//! 典型用法：
//! ```ignore
//! use conduit_core::events::EventBus;
//!
//! #[derive(Clone, Debug)]
//! enum DemoEvent { A(u32), B(String) }
//!
//! let bus: EventBus<DemoEvent> = EventBus::new(64);
//! let mut rx = bus.subscribe();
//! bus.publish(DemoEvent::A(42));
//! // 订阅者可在异步上下文中 `rx.recv().await` 拿到事件。
//! ```
//!
//! 设计参考：`design/2026-05-06-2-Conduit-Rust-重写设计文档.md` §4.3。

use std::fmt::Debug;

use tokio::sync::broadcast;

/// 多订阅事件总线。`T` 必须 `Clone + Send + 'static` 以便跨任务传递。
///
/// `EventBus` 本身是 `Clone`，clone 后多个 publisher 共享同一个内部 channel。
#[derive(Debug, Clone)]
pub struct EventBus<T: Clone + Send + 'static> {
    tx: broadcast::Sender<T>,
}

impl<T: Clone + Send + 'static> EventBus<T> {
    /// 新建一个容量为 `capacity` 的事件总线。
    /// 当订阅者跟不上 publish 速度时，最老的消息会被丢弃，订阅者下次 `recv` 拿到 `RecvError::Lagged(n)`。
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// 发布一个事件。**当前没有订阅者时不报错**（fan-out 0 路被视为正常 no-op）。
    pub fn publish(&self, event: T) {
        // broadcast::Sender::send 在零订阅者时返回 SendError，对我们是 OK 的；忽略即可。
        let _ = self.tx.send(event);
    }

    /// 订阅事件。新订阅者只能收到订阅之后 publish 的事件，订阅之前的消息看不到。
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.tx.subscribe()
    }

    /// 当前活跃订阅者数（含已 subscribe 但还没释放 receiver 的）。
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::RecvError;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Evt {
        Connected(u32),
        Disconnected(u32),
    }

    #[tokio::test]
    async fn publish_then_subscribe_misses_old_messages() {
        let bus: EventBus<Evt> = EventBus::new(8);
        bus.publish(Evt::Connected(1)); // 订阅前发的，丢弃
        let mut rx = bus.subscribe();
        bus.publish(Evt::Connected(2));
        assert_eq!(rx.recv().await.unwrap(), Evt::Connected(2));
    }

    #[tokio::test]
    async fn multi_subscribers_each_get_a_copy() {
        let bus: EventBus<Evt> = EventBus::new(8);
        let mut rx_a = bus.subscribe();
        let mut rx_b = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        bus.publish(Evt::Connected(7));
        bus.publish(Evt::Disconnected(7));
        assert_eq!(rx_a.recv().await.unwrap(), Evt::Connected(7));
        assert_eq!(rx_b.recv().await.unwrap(), Evt::Connected(7));
        assert_eq!(rx_a.recv().await.unwrap(), Evt::Disconnected(7));
        assert_eq!(rx_b.recv().await.unwrap(), Evt::Disconnected(7));
    }

    #[tokio::test]
    async fn capacity_overflow_yields_lagged_error() {
        let bus: EventBus<u32> = EventBus::new(2);
        let mut rx = bus.subscribe();
        for i in 0..5 {
            bus.publish(i);
        }
        // tokio::broadcast 容量满时给最早丢弃的订阅者下次 recv 一次 Lagged，
        // 然后继续给 buffer 中尚存的最老消息（这里是 3）。
        match rx.recv().await {
            Err(RecvError::Lagged(n)) => assert!(n >= 1, "expected lagged > 0, got {n}"),
            other => panic!("expected Lagged, got {other:?}"),
        }
        assert_eq!(rx.recv().await.unwrap(), 3);
        assert_eq!(rx.recv().await.unwrap(), 4);
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_is_silent() {
        let bus: EventBus<u32> = EventBus::new(4);
        // 不订阅；publish 不应 panic 或返回错误给调用方。
        bus.publish(1);
        bus.publish(2);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn clone_publisher_shares_underlying_channel() {
        let bus_a: EventBus<u32> = EventBus::new(4);
        let bus_b = bus_a.clone();
        let mut rx = bus_a.subscribe();
        bus_b.publish(99);
        assert_eq!(rx.recv().await.unwrap(), 99);
    }
}
