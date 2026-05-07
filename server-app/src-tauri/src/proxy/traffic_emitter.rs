//! 周期 publish `traffic_tick` 事件, 让 server-app/ui 的实时流量曲线 +
//! ClientList 上下行 bps 列拿到数据。
//!
//! 数据来源: [`SessionRegistry::peer_totals_snapshot`]——peer 级别 monotonic
//! 累计字节, 不会因 session remove 而回退。每 [`TICK_INTERVAL`] 拉一次快照,
//! 与上一次做差除以间隔得到 per-peer bps, 然后 publish。
//!
//! 与前端契约 `TrafficTickPayload`(`server-app/ui/src/types/proxy.ts`) 严格对齐:
//! ```json
//! { "ts": 1700000000.0, "per_peer": { "10.0.0.5": { "sent_bps": 1024, "recv_bps": 2048 } } }
//! ```
//!
//! 设计要点:
//! - **始终 1Hz publish**(包括所有 peer 全 0 的快照): 前端 `applyTick` 依赖
//!   持续到达的 tick 推动时间轴,若静默期跳过 publish, UI 流量曲线就不会
//!   随时间向左滚动, 看起来"曲线静止 / 不前进"。1 帧 ~50 字节,SSE 带宽
//!   开销可忽略。
//! - 协程退出条件与 `vpn_detect` 一致: 监听 `cancel` token。

use std::collections::HashMap;
use std::time::Duration;

use conduit_core::time::epoch_secs;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::core::{ProxyCore, ServerEvent};

const TICK_INTERVAL: Duration = Duration::from_secs(1);

/// 启动 traffic 发射协程。`cancel` 触发时退出。
pub async fn run(core: ProxyCore, cancel: CancellationToken) {
    let mut prev: HashMap<String, (u64, u64)> = HashMap::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(TICK_INTERVAL) => {}
        }

        let snapshot = core.sessions().peer_totals_snapshot().await;
        let dt_sec = TICK_INTERVAL.as_secs_f64();

        let mut per_peer = serde_json::Map::with_capacity(snapshot.len());
        for (peer, sent_total, recv_total) in &snapshot {
            let (prev_sent, prev_recv) = prev.get(peer).copied().unwrap_or((0, 0));
            let dsent = sent_total.saturating_sub(prev_sent);
            let drecv = recv_total.saturating_sub(prev_recv);
            let sent_bps = (dsent as f64 / dt_sec).round() as u64;
            let recv_bps = (drecv as f64 / dt_sec).round() as u64;
            per_peer.insert(
                peer.clone(),
                json!({ "sent_bps": sent_bps, "recv_bps": recv_bps }),
            );
        }
        prev = snapshot.into_iter().map(|(p, s, r)| (p, (s, r))).collect();

        let payload = json!({
            "ts": epoch_secs(),
            "per_peer": serde_json::Value::Object(per_peer),
        });
        core.event_bus().publish(ServerEvent {
            kind: "traffic_tick".into(),
            payload,
            ts: epoch_secs(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::ProxyConfig;
    use tokio::sync::broadcast::error::RecvError;

    /// 有 session 字节累加时, traffic_emitter 应该 publish 一帧带正
    /// per_peer.sent_bps / recv_bps 的事件。
    #[tokio::test]
    async fn emits_traffic_tick_when_peer_has_bytes() {
        let core = ProxyCore::new(ProxyConfig::default());
        let mut rx = core.event_bus().subscribe();
        let sessions = core.sessions();
        let info = sessions
            .add("10.0.0.42".into(), "http", "example.com:443".into())
            .await;
        let sink = sessions.sink_for(info.session_id.clone());
        sink.on_progress(2048, 4096);

        let cancel = CancellationToken::new();
        let core_clone = core.clone();
        let cancel_clone = cancel.clone();
        let h = tokio::spawn(async move {
            run(core_clone, cancel_clone).await;
        });

        // 第一帧应在 ~1s 后到, 给 2s 余量。
        let evt = tokio::time::timeout(Duration::from_millis(2500), async {
            loop {
                match rx.recv().await {
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

        let per_peer = evt.payload["per_peer"].as_object().expect("per_peer obj");
        let peer = per_peer["10.0.0.42"].clone();
        let sent_bps = peer["sent_bps"].as_u64().unwrap();
        let recv_bps = peer["recv_bps"].as_u64().unwrap();
        assert!(sent_bps > 0, "sent_bps should be positive, got {sent_bps}");
        assert!(recv_bps > 0, "recv_bps should be positive, got {recv_bps}");
    }

    /// 完全静默期(无任何 peer / 全 0 字节)也必须 1Hz publish, 让前端
    /// `applyTick` 持续推动时间轴, 否则流量曲线 X 轴静止不前进。
    /// payload.per_peer 在无 peer 时是空对象。
    #[tokio::test]
    async fn emits_empty_tick_when_idle() {
        let core = ProxyCore::new(ProxyConfig::default());
        let mut rx = core.event_bus().subscribe();
        let cancel = CancellationToken::new();
        let core_clone = core.clone();
        let cancel_clone = cancel.clone();
        let h = tokio::spawn(async move {
            run(core_clone, cancel_clone).await;
        });
        let evt = tokio::time::timeout(Duration::from_millis(2500), async {
            loop {
                match rx.recv().await {
                    Ok(e) if e.kind == "traffic_tick" => return e,
                    Ok(_) => continue,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => panic!("event bus closed"),
                }
            }
        })
        .await
        .expect("expected traffic_tick within 2.5s even when idle");
        cancel.cancel();
        let _ = h.await;
        let per_peer = evt.payload["per_peer"].as_object().expect("per_peer obj");
        assert!(per_peer.is_empty(), "idle tick should have empty per_peer");
        assert!(evt.payload["ts"].as_f64().unwrap() > 0.0);
    }
}
