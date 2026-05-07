//! `ProxyCore` —— server-app 内嵌的代理总入口（HTTP / SOCKS5 / mDNS / 控制 API）。
//!
//! W2 Sprint 2 阶段先建骨架：
//! - `new(cfg)` —— 创建 EventBus / 端口预留 / cancel-token 等共享状态；
//! - `start()` —— 后续 sub-task 里依次拉起 HTTP / SOCKS5 / mDNS / system-proxy
//!   监听任务，并把所有 task 的 [`tokio::task::JoinHandle`] 收纳进来；
//! - `stop()` —— 先发取消信号，再 join 所有任务，确保端口释放；
//! - `status()` —— 给 IPC `get_status` 用，返回当前 [`ServerStatus`]。
//!
//! 当前 S2.1 阶段 `start` / `stop` 内部还是 `todo!()`：等 S2.2 ~ S2.6 把各模块
//! 实装后再回填。这样可以让 `ProxyCore::new(cfg)` 这个 API 立刻被 Tauri shell
//! 引用，方便 S2.7 提前接 IPC 骨架。

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use conduit_core::time::epoch_secs;
use conduit_core::{EventBus, PacRules, PAC_TEMPLATE};

use super::config::ProxyConfig;
use super::session::SessionRegistry;

/// 进程内事件流单条载荷，转发给 control API SSE / Tauri webview event listener。
#[derive(Debug, Clone, Serialize)]
pub struct ServerEvent {
    pub kind: String,
    pub payload: serde_json::Value,
    /// epoch 秒（UTC，与 UI 端 `Date.now()/1000` 对齐）。
    pub ts: f64,
}

/// 当前服务运行状态快照，给 `get_status` IPC 用。
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub http_port: u16,
    pub socks_port: u16,
    pub api_port: u16,
    pub bind: String,
    pub running: bool,
    pub uptime_sec: f64,
    pub mdns_enabled: bool,
    pub vpn_on: bool,
    /// 当前激活的 VPN 接口名（如 `utun5`）；未检测到 VPN 时为 `None`。
    pub vpn_iface: Option<String>,
    /// 系统**当前默认路由**是否经过 VPN 接口 (Tunnel/Ppp/is_tun)。
    /// 真实反映"出口流量是否走 VPN", 与 vpn_on (有 VPN 接口) 区分:
    /// 用户可能开了 VPN 但没切默认路由 (split tunnel) → vpn_on=true, default_via=false。
    pub default_route_via_vpn: bool,
    /// 当前在线客户端数（passive heartbeat + 进行中会话合并）。
    pub clients_online: usize,
    /// 进行中代理会话数。
    pub active_sessions: usize,
}

/// Server proxy 总入口。
///
/// 内部所有可变状态都通过 `Arc<Mutex<_>>` 管理，使 `ProxyCore` 本身可以 `Clone`
/// 并在 Tauri command handler / 后台 task 之间自由共享。
#[derive(Clone)]
pub struct ProxyCore {
    cfg: Arc<ProxyConfig>,
    bus: EventBus<ServerEvent>,
    cancel: CancellationToken,
    sessions: Arc<SessionRegistry>,
    pac_rules: Arc<Mutex<Option<Arc<PacRules>>>>,
    inner: Arc<Mutex<CoreInner>>,
}

struct CoreInner {
    started_at: Option<std::time::Instant>,
    handles: Vec<JoinHandle<()>>,
    vpn_on: bool,
    /// 当前激活的 VPN 接口名（如 `utun5`），由 [`super::vpn_detect`] 周期更新。
    /// `None` 表示未检测到 VPN。
    vpn_iface: Option<String>,
    /// 系统默认路由是否走 VPN, 由 [`super::vpn_detect`] 周期更新。
    default_route_via_vpn: bool,
}

impl ProxyCore {
    /// 创建实例但不启动监听。EventBus 容量 256（够 UI/SSE 同时订阅）。
    /// PAC rules 在构造时就加载（来自 embedded `proxy.pac`），让 `/check`
    /// / outbound policy 在 `start()` 之前就能给出决策。
    pub fn new(cfg: ProxyConfig) -> Self {
        let mut rules = PacRules::parse(PAC_TEMPLATE);
        let host = super::effective_advertised_host(&cfg);
        rules.update_proxy_target(&host, cfg.http_port);
        Self {
            cfg: Arc::new(cfg),
            bus: EventBus::new(256),
            cancel: CancellationToken::new(),
            sessions: SessionRegistry::new(),
            pac_rules: Arc::new(Mutex::new(Some(Arc::new(rules)))),
            inner: Arc::new(Mutex::new(CoreInner {
                started_at: None,
                handles: Vec::new(),
                vpn_on: false,
                vpn_iface: None,
                default_route_via_vpn: false,
            })),
        }
    }

    /// 取已注册的 EventBus，让 Tauri shell 用 `bus.subscribe()` 拿
    /// `Receiver<ServerEvent>` 后转发到 `app.emit("server-event", _)`。
    pub fn event_bus(&self) -> EventBus<ServerEvent> {
        self.bus.clone()
    }

    /// 当前生效的配置（不可修改）。Tauri commands 可以借此读端口给 UI 显示。
    pub fn config(&self) -> Arc<ProxyConfig> {
        self.cfg.clone()
    }

    /// 共享的会话注册表，HTTP / SOCKS5 handler 都向它登记会话。
    pub fn sessions(&self) -> Arc<SessionRegistry> {
        self.sessions.clone()
    }

    /// 取当前 PAC 规则（用于 `/check` IPC、outbound policy 派发等）。
    pub async fn pac_rules(&self) -> Option<Arc<PacRules>> {
        self.pac_rules.lock().await.clone()
    }

    /// 取共享的 cancel token，让额外的子任务可以挂在同一个生命周期上。
    /// 当前 lib.rs 的 graceful shutdown 走 `stop()`；该 helper 留给单测 /
    /// 未来 outbound 模块用。
    #[allow(dead_code)]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// 拉起 HTTP / SOCKS5 / mDNS / system-proxy 等所有监听任务。
    /// 所有任务都用同一个 `CancellationToken` 做协作式 stop。
    pub async fn start(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().await;
        if inner.started_at.is_some() {
            return Err("ProxyCore already running".into());
        }
        inner.started_at = Some(std::time::Instant::now());

        // S2.2：HTTP 正向代理 accept loop
        let core = self.clone();
        let cancel = self.cancel.clone();
        let sessions = self.sessions.clone();
        let h_http = tokio::spawn(async move {
            if let Err(e) = super::http::run(core, cancel, sessions).await {
                log::error!("[http] accept loop terminated: {e}");
            }
        });
        inner.handles.push(h_http);

        // S2.3 SOCKS5
        let core = self.clone();
        let cancel = self.cancel.clone();
        let sessions = self.sessions.clone();
        let h_socks = tokio::spawn(async move {
            if let Err(e) = super::socks5::run(core, cancel, sessions).await {
                log::error!("[socks5] accept loop terminated: {e}");
            }
        });
        inner.handles.push(h_socks);

        // S2.4：mDNS 广播任务
        let core = self.clone();
        let cancel = self.cancel.clone();
        let h_mdns = tokio::spawn(async move {
            super::mdns::run(core, cancel).await;
        });
        inner.handles.push(h_mdns);

        // S2.7 Control API server（兼容 UI 现有 REST/SSE）
        let core = self.clone();
        let cancel = self.cancel.clone();
        let h_ctl = tokio::spawn(async move {
            if let Err(e) = super::control_api::run(core, cancel).await {
                log::error!("[ctl] control api terminated: {e}");
            }
        });
        inner.handles.push(h_ctl);

        // VPN 接口检测协程：周期 list_afinet_netifas，看是否有 utun*/ppp*/tun*
        // 拿到 IPv4。状态翻转才推 SSE event_state_changed，UI 由此切换徽标。
        let core = self.clone();
        let cancel = self.cancel.clone();
        let h_vpn = tokio::spawn(async move {
            super::vpn_detect::run(core, cancel).await;
        });
        inner.handles.push(h_vpn);

        // 流量发射协程: 1Hz 拉 SessionRegistry::peer_totals 做差算 per-peer bps,
        // publish traffic_tick。修复"实时流量窗口不显示数据"+ ClientList 的
        // 上下行始终为 0 的 bug —— 之前后端从未 publish 过 traffic_tick。
        let core = self.clone();
        let cancel = self.cancel.clone();
        let h_traffic = tokio::spawn(async move {
            super::traffic_emitter::run(core, cancel).await;
        });
        inner.handles.push(h_traffic);

        Ok(())
    }

    /// 触发取消并等待所有后台任务结束。
    ///
    /// 实现策略：先 cancel，再**短暂持锁**把 handles 拿出来，立刻释放锁，
    /// 然后才 join 各个 task。这样 task 内即使持续调用 `inner.lock()`
    /// （如 [`super::vpn_detect`] 周期 update_vpn）也不会与 stop 死锁。
    pub async fn stop(&self) {
        self.cancel.cancel();
        let handles: Vec<JoinHandle<()>> = {
            let mut inner = self.inner.lock().await;
            inner.handles.drain(..).collect()
        };
        for h in handles {
            let _ = h.await;
        }
        let mut inner = self.inner.lock().await;
        inner.started_at = None;
    }

    /// 返回给 UI 显示用的状态快照。
    pub async fn status(&self) -> ServerStatus {
        let (running, uptime, vpn_on, vpn_iface, default_via) = {
            let inner = self.inner.lock().await;
            (
                inner.started_at.is_some(),
                inner.started_at.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0),
                inner.vpn_on,
                inner.vpn_iface.clone(),
                inner.default_route_via_vpn,
            )
        };
        let active_sessions = self.sessions.active_count().await;
        let passive = self.sessions.passive_count().await;
        ServerStatus {
            http_port: self.cfg.http_port,
            socks_port: self.cfg.socks_port,
            api_port: self.cfg.api_port,
            bind: self.cfg.bind.clone(),
            running,
            uptime_sec: uptime,
            mdns_enabled: self.cfg.mdns_enabled,
            vpn_on,
            vpn_iface,
            default_route_via_vpn: default_via,
            clients_online: passive + active_sessions,
            active_sessions,
        }
    }

    /// 当前 VPN 状态快照 (vpn_on, vpn_iface)，供 [`super::mdns`] 启动期同步读取
    /// 真实初始状态使用，避免 mDNS 首次 advertise 用 false 默认值与实际不符。
    pub async fn vpn_snapshot(&self) -> (bool, Option<String>) {
        let inner = self.inner.lock().await;
        (inner.vpn_on, inner.vpn_iface.clone())
    }

    /// 由 [`super::vpn_detect`] 周期检测协程调用，刷新 vpn 状态并广播事件。
    ///
    /// 内部对 (vpn_on, vpn_iface, default_route_via_vpn) 三元组做去重：任一字段
    /// 变化才会 publish `vpn_state_changed` event，避免每 5s 都发抖动事件。
    /// Event 名与 payload 字段(`available` / `iface` / `default_route_via_vpn`)
    /// 严格对齐前端 `VpnStateChangedPayload`(server-app/ui/src/types/proxy.ts)。
    pub async fn update_vpn(
        &self,
        vpn_on: bool,
        vpn_iface: Option<String>,
        default_route_via_vpn: bool,
    ) {
        let mut inner = self.inner.lock().await;
        if inner.vpn_on == vpn_on
            && inner.vpn_iface == vpn_iface
            && inner.default_route_via_vpn == default_route_via_vpn
        {
            return;
        }
        inner.vpn_on = vpn_on;
        inner.vpn_iface = vpn_iface.clone();
        inner.default_route_via_vpn = default_route_via_vpn;
        drop(inner);
        self.bus.publish(ServerEvent {
            kind: "vpn_state_changed".into(),
            payload: serde_json::json!({
                "available": vpn_on,
                "iface": vpn_iface,
                "default_route_via_vpn": default_route_via_vpn,
            }),
            ts: epoch_secs(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_then_status_reports_not_running() {
        let core = ProxyCore::new(ProxyConfig::with_ports(18080, 11080, 18090));
        let s = core.status().await;
        assert_eq!(s.http_port, 18080);
        assert_eq!(s.socks_port, 11080);
        assert_eq!(s.api_port, 18090);
        assert!(!s.running);
        assert_eq!(s.uptime_sec, 0.0);
        assert_eq!(s.clients_online, 0);
    }

    #[tokio::test]
    async fn start_then_status_reports_running() {
        let core = ProxyCore::new(ProxyConfig::default());
        core.start().await.unwrap();
        let s = core.status().await;
        assert!(s.running);
    }

    #[tokio::test]
    async fn double_start_is_rejected() {
        let core = ProxyCore::new(ProxyConfig::default());
        core.start().await.unwrap();
        let err = core.start().await.unwrap_err();
        assert!(err.contains("already running"));
    }

    #[tokio::test]
    async fn stop_clears_running_state() {
        let core = ProxyCore::new(ProxyConfig::default());
        core.start().await.unwrap();
        core.stop().await;
        assert!(!core.status().await.running);
    }

    #[tokio::test]
    async fn vpn_snapshot_reflects_latest_update() {
        let core = ProxyCore::new(ProxyConfig::default());
        assert_eq!(core.vpn_snapshot().await, (false, None));
        core.update_vpn(true, Some("utun5".into()), false).await;
        assert_eq!(core.vpn_snapshot().await, (true, Some("utun5".into())));
        core.update_vpn(false, None, false).await;
        assert_eq!(core.vpn_snapshot().await, (false, None));
    }

    #[tokio::test]
    async fn update_vpn_publishes_event_only_on_change() {
        let core = ProxyCore::new(ProxyConfig::default());
        let mut rx = core.event_bus().subscribe();
        core.update_vpn(true, Some("utun5".into()), true).await;
        let evt = rx.recv().await.unwrap();
        assert_eq!(evt.kind, "vpn_state_changed");
        assert_eq!(evt.payload["available"], serde_json::Value::Bool(true));
        assert_eq!(evt.payload["iface"], serde_json::Value::String("utun5".into()));
        assert_eq!(
            evt.payload["default_route_via_vpn"],
            serde_json::Value::Bool(true)
        );

        // 同三元组,不应重复 publish
        core.update_vpn(true, Some("utun5".into()), true).await;
        let again = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
        assert!(again.is_err(), "状态未变时不应再次 publish");

        // iface 变化算变更
        core.update_vpn(true, Some("ppp0".into()), true).await;
        let evt = rx.recv().await.unwrap();
        assert_eq!(evt.payload["iface"], serde_json::Value::String("ppp0".into()));

        // 仅 default_route_via_vpn 翻转也算变更 (split tunnel 切换场景)。
        core.update_vpn(true, Some("ppp0".into()), false).await;
        let evt = rx.recv().await.unwrap();
        assert_eq!(
            evt.payload["default_route_via_vpn"],
            serde_json::Value::Bool(false)
        );
    }
}
