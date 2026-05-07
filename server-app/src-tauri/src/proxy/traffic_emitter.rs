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
//! - **零负载早退**: 当快照中所有 peer 当前秒 bps 全 0 且**上一秒也全 0** 时
//!   不 publish, 避免空闲期不停推 0 帧打扰 SSE 连接。仍保持记账状态以便下次
//!   有流量时能立刻发出。
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
    let mut last_nonzero = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(TICK_INTERVAL) => {}
        }

        let snapshot = core.sessions().peer_totals_snapshot().await;
        let dt_sec = TICK_INTERVAL.as_secs_f64();

        let mut per_peer = serde_json::Map::with_capacity(snapshot.len());
        let mut any_nonzero = false;
        for (peer, sent_total, recv_total) in &snapshot {
            let (prev_sent, prev_recv) = prev.get(peer).copied().unwrap_or((0, 0));
            let dsent = sent_total.saturating_sub(prev_sent);
            let drecv = recv_total.saturating_sub(prev_recv);
            let sent_bps = (dsent as f64 / dt_sec).round() as u64;
            let recv_bps = (drecv as f64 / dt_sec).round() as u64;
            if sent_bps > 0 || recv_bps > 0 {
                any_nonzero = true;
            }
            per_peer.insert(
                peer.clone(),
                json!({ "sent_bps": sent_bps, "recv_bps": recv_bps }),
            );
        }
        prev = snapshot.into_iter().map(|(p, s, r)| (p, (s, r))).collect();

        // 只在: 有流量 OR 刚从有流量切到 0(让前端 series 收尾画到 0) 时 publish。
        // 持续静默期不打扰 SSE。
        let should_publish = any_nonzero || last_nonzero;
        last_nonzero = any_nonzero;
        if !should_publish {
            continue;
        }

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

    /// 完全静默期不该刷屏 publish 0 帧 —— 仅在 peer_totals 为空 / 全 0 时
    /// 应当不发任何事件。
    #[tokio::test]
    async fn does_not_emit_when_all_peers_idle() {
        let core = ProxyCore::new(ProxyConfig::default());
        let mut rx = core.event_bus().subscribe();
        let cancel = CancellationToken::new();
        let core_clone = core.clone();
        let cancel_clone = cancel.clone();
        let h = tokio::spawn(async move {
            run(core_clone, cancel_clone).await;
        });
        // 给 emitter 跑两个 tick 的时间, 期间不应发 traffic_tick。
        let res = tokio::time::timeout(Duration::from_millis(2200), async {
            loop {
                match rx.recv().await {
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
        assert!(res.is_err(), "should not emit traffic_tick when idle");
    }
}
