//! `ClientCore` —— client-app 的进程内运行时句柄。
//!
//! 职责：
//! - 持有 [`ClientConfig`]、[`EventBus<ClientEvent>`]、`RouteCache`、`RouteResolver`、
//!   `LocalProxy`、`Discoverer`、可选 `Heartbeat` / `TrafficMeter` / `MacSystemProxy`。
//! - 暴露 5 步连接状态机：[`Self::connect_to`] / [`Self::disconnect`] /
//!   [`Self::connection_snapshot`]。
//! - control_api（loopback HTTP）通过 `Arc<ClientCore>` 调本类公开方法。
//!
//! 5 步定义（见 [`CONNECT_STEPS`]）：
//! 1. `probe`           可达性检查（HTTP/SOCKS TCP 三次握手）
//! 2. `fetch_pac`       拉取 PAC（`GET http://{host}:{port}/proxy.pac`）
//! 3. `prefill_cache`   解析 PAC 1+2 段，预填 `RouteCache`
//! 4. `switch_endpoint` `LocalProxy` 切换上游 + 启用系统代理
//! 5. `start_heartbeat` 启动 `Heartbeat` 协程
//!
//! 任一步失败都会广播 `connect_progress(status=failed)` 并把 state 翻 `Failed`，
//! 终态广播 `connect_done`（payload 与 [`ConnectionSnapshot`] 同 + `server_id` 顶层）。

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use conduit_core::pac::PacRules;
use conduit_core::{
    ConnectProgress, ConnectStepStatus, ConnectedServerSummary, ConnectionHeartbeat,
    ConnectionSnapshot, ConnectionState, DiscoveredServer, EventBus, RouteDirection,
};

use super::config::ClientConfig;
use super::connectivity::{probe, Heartbeat, DEFAULT_PROBE_TIMEOUT};
use super::discoverer::Discoverer;
use super::local_proxy::{LocalProxy, ServerEndpoint};
use super::route_cache::{RouteCache, DEFAULT_PREFILL_TTL_SEC};
use super::route_resolver::RouteResolver;
use super::system_proxy::MacSystemProxy;
use super::traffic_meter::TrafficMeter;

/// 跨平台 diag 写文件:macOS 走 `system_proxy_sc::diag_log_pub`(写
/// `~/Library/Logs/Conduit/conduit-client.log`),其它平台 noop。
/// 用来对照 connect_lock 进出和 SC API 调用的时间戳。
#[cfg(target_os = "macos")]
fn diag_log(msg: &str) {
    super::system_proxy_sc::diag_log_pub(msg);
}

#[cfg(not(target_os = "macos"))]
fn diag_log(_msg: &str) {}

/// 进程内事件总线消息体，转发给 control API SSE / Tauri webview event listener。
///
/// `kind` 取值：`heartbeat_changed` / `traffic_tick` /
/// `connect_progress` / `connect_done` / `connection_state_changed` / `route_decision` / ...
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEvent {
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts: f64,
}

impl ClientEvent {
    pub fn new(kind: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
            ts: epoch_now(),
        }
    }
}

/// 5 步连接进度的常量定义（连接到 server 时按顺序执行）。
pub const CONNECT_STEPS: [(&str, &str); 5] = [
    ("probe", "可达性检查"),
    ("fetch_pac", "拉取 PAC"),
    ("prefill_cache", "解析 PAC 预填路由"),
    ("switch_endpoint", "切换上游 server"),
    ("start_heartbeat", "启动心跳与系统代理"),
];

const PAC_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct ClientCore {
    inner: Arc<Inner>,
}

struct Inner {
    config: ClientConfig,
    bus: EventBus<ClientEvent>,
    cache: RouteCache,
    resolver: RouteResolver,
    local_proxy: LocalProxy,
    discoverer: Arc<Discoverer>,
    traffic_meter: TrafficMeter,
    /// 当前心跳协程；连接成功才有；disconnect 时停掉。
    heartbeat: Mutex<Option<Arc<Heartbeat>>>,
    /// 系统代理是否被本进程接管（disconnect 时回滚 → disable）。
    system_proxy_active: Mutex<bool>,
    system_proxy: MacSystemProxy,
    /// 5 步状态机：当前连接生命周期。
    connection: Mutex<ConnectionRecord>,
    /// connect/disconnect 串行化锁。
    connect_lock: Mutex<()>,
    /// 全局取消（用于 Tauri 进程退出时优雅停掉所有协程）。
    cancel: CancellationToken,
    started_at: f64,
}

#[derive(Debug, Clone)]
struct ConnectionRecord {
    state: ConnectionState,
    server: Option<DiscoveredServer>,
    connected_since: Option<f64>,
    last_error: Option<String>,
}

impl Default for ConnectionRecord {
    fn default() -> Self {
        Self {
            state: ConnectionState::Idle,
            server: None,
            connected_since: None,
            last_error: None,
        }
    }
}

impl ClientCore {
    pub fn new(config: ClientConfig) -> Self {
        let bus: EventBus<ClientEvent> = EventBus::new(256);
        let cache = RouteCache::new();
        let resolver = RouteResolver::new(cache.clone());
        let local_proxy = LocalProxy::new(
            config.bind_host.clone(),
            config.bind_port,
            resolver.clone(),
        );
        let discoverer = Discoverer::new(bus.clone());
        let traffic_meter = TrafficMeter::new(bus.clone());

        Self {
            inner: Arc::new(Inner {
                config,
                bus,
                cache,
                resolver,
                local_proxy,
                discoverer,
                traffic_meter,
                heartbeat: Mutex::new(None),
                system_proxy_active: Mutex::new(false),
                system_proxy: MacSystemProxy,
                connection: Mutex::new(ConnectionRecord::default()),
                connect_lock: Mutex::new(()),
                cancel: CancellationToken::new(),
                started_at: epoch_now(),
            }),
        }
    }

    pub fn config(&self) -> &ClientConfig {
        &self.inner.config
    }

    pub fn bus(&self) -> EventBus<ClientEvent> {
        self.inner.bus.clone()
    }

    pub fn cache(&self) -> &RouteCache {
        &self.inner.cache
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.inner.cancel.clone()
    }

    pub fn discoverer(&self) -> Arc<Discoverer> {
        self.inner.discoverer.clone()
    }

    pub fn local_proxy(&self) -> &LocalProxy {
        &self.inner.local_proxy
    }

    pub fn traffic_meter(&self) -> &TrafficMeter {
        &self.inner.traffic_meter
    }

    pub fn started_at(&self) -> f64 {
        self.inner.started_at
    }

    /// 启动 LocalProxy + Discoverer + （可选）pre-fill 系统代理切换。
    /// control_api 由调用方单独 spawn（拿 ClientCore 的 Arc 传进去）。
    pub async fn start(&self) -> Result<(), String> {
        // 1) 启动 LocalProxy SOCKS5 监听器
        let port = self
            .inner
            .local_proxy
            .start()
            .await
            .map_err(|e| format!("local_proxy start failed: {e}"))?;
        info!("[client_core] local_proxy listening on 127.0.0.1:{port}");

        // 2) 流量计量 sink 接线 + 启动 1Hz 聚合 emitter 协程。
        //    on_progress 热路径只做 atomic fetch_add(零分配),
        //    实际 SSE traffic_tick publish 由 emitter 1Hz 聚合发出,
        //    避免大下载场景每 64KiB chunk 都广播 SSE 拖慢 relay 速率。
        self.inner
            .local_proxy
            .set_progress_sink(Some(self.inner.traffic_meter.clone()))
            .await;
        let _h_traffic = self
            .inner
            .traffic_meter
            .spawn_emitter(self.inner.cancel.clone());

        // 3) Discoverer（mDNS）—— 失败不阻断启动
        if let Err(e) = self.inner.discoverer.start().await {
            warn!("[client_core] discoverer start failed (continuing): {e}");
        }

        // 4) 启动时系统代理 stale cleanup
        if self.inner.config.enable_system_proxy
            && self
                .inner
                .system_proxy
                .cleanup_if_pointing_to_us(&self.inner.config.bind_host, port)
        {
            warn!("[client_core] cleaned stale system proxy from previous run");
        }

        // 广播 ready
        self.inner.bus.publish(ClientEvent::new(
            "ready",
            serde_json::json!({ "version": env!("CARGO_PKG_VERSION") }),
        ));

        Ok(())
    }

    pub async fn stop(&self) {
        // 心跳
        if let Some(hb) = self.inner.heartbeat.lock().await.take() {
            hb.stop().await;
        }
        // 系统代理回滚
        self.rollback_system_proxy().await;
        // local_proxy
        self.inner.local_proxy.stop().await;
        // 3) 启动 mDNS 发现器
        self.inner.discoverer.stop().await;
        // 全局取消
        self.inner.cancel.cancel();
    }

    /// 当前连接快照，control_api `GET /api/connection` 用。
    pub async fn connection_snapshot(&self) -> ConnectionSnapshot {
        let rec = self.inner.connection.lock().await.clone();
        let hb_state = match self.inner.heartbeat.lock().await.as_ref() {
            Some(hb) => Some(ConnectionHeartbeat::from(&hb.snapshot().await)),
            None => None,
        };
        ConnectionSnapshot {
            ok: matches!(rec.state, ConnectionState::Connected | ConnectionState::Idle),
            state: rec.state,
            server: rec.server.as_ref().map(ConnectedServerSummary::from),
            connected_since: rec.connected_since,
            system_proxy_active: *self.inner.system_proxy_active.lock().await,
            heartbeat: hb_state,
            last_error: rec.last_error,
        }
    }

    // -------------------- 5 步状态机 --------------------

    /// 触发完整 5 步连接流程；任一步失败立即 short-circuit。
    pub async fn connect_to(&self, server: DiscoveredServer) -> Result<ConnectionSnapshot, String> {
        diag_log(&format!(
            "core::connect_to ENTER server={}",
            server.server_id
        ));
        let _guard = self.inner.connect_lock.lock().await;
        diag_log(&format!(
            "core::connect_to LOCK_ACQUIRED server={}",
            server.server_id
        ));

        // 已连到同一个 server 直接幂等返回，避免重复跑 5 步流程。
        //
        // **关键 bug 修复(死锁)**:旧代码在持有 `self.inner.connection.lock()`
        // guard 的同时调 `self.connection_snapshot().await`,而 snapshot 内部
        // 又会去 lock 同一把 `connection` mutex。tokio::sync::Mutex 不可重
        // 入,这就是死锁。症状是 connect_to 拿到 connect_lock 后无任何进展,
        // 后续所有 disconnect/connect 都卡在 connect_lock(用户实测"反复连
        // 接 5 次后必然卡 5 步等待中")。
        //
        // 修复:用单独 scope 取出需要的标志位,**guard drop 后**再调 snapshot。
        let already_connected_to_same = {
            let rec = self.inner.connection.lock().await;
            rec.state == ConnectionState::Connected
                && rec.server.as_ref().map(|s| &s.server_id) == Some(&server.server_id)
        };
        if already_connected_to_same {
            diag_log("core::connect_to IDEMPOTENT_RETURN");
            return Ok(self.connection_snapshot().await);
        }

        self.set_state(ConnectionState::Connecting, Some(server.clone()), None)
            .await;

        let server_id = server.server_id.clone();

        // step 1: probe
        if let Err(e) = self.step_probe(&server).await {
            self.fail_connect(&server_id, &e).await;
            return Err(e);
        }

        // step 2: fetch_pac
        let pac_text = match self.step_fetch_pac(&server).await {
            Ok(t) => t,
            Err(e) => {
                self.fail_connect(&server_id, &e).await;
                return Err(e);
            }
        };

        // 第 3 步：prefill_cache（拉 PAC 跑一遍 prime hot host）
        if let Err(e) = self.step_prefill_cache(&server, &pac_text).await {
            self.fail_connect(&server_id, &e).await;
            return Err(e);
        }

        // 第 4 步：switch_endpoint（设上游 + 必要时开系统代理）
        if let Err(e) = self.step_switch_endpoint(&server).await {
            self.fail_connect(&server_id, &e).await;
            return Err(e);
        }

        // 第 5 步：start_heartbeat（probe + 通知 server passive registry）
        if let Err(e) = self.step_start_heartbeat(&server).await {
            self.fail_connect(&server_id, &e).await;
            return Err(e);
        }

        // ---- 5 步全部成功 ----
        let now = epoch_now();
        let label = format!("{}:{}", server.host, server.socks);
        {
            let mut rec = self.inner.connection.lock().await;
            rec.state = ConnectionState::Connected;
            rec.server = Some(server.clone());
            rec.connected_since = Some(now);
            rec.last_error = None;
        }
        let snap = self.connection_snapshot().await;
        self.publish_state_changed(ConnectionState::Connected, Some(&server_id), None);
        self.publish_connect_done(&server_id, &snap);
        info!("[client_core] connected to {server_id} ({label})");
        Ok(snap)
    }

    pub async fn disconnect(&self) -> Result<ConnectionSnapshot, String> {
        diag_log("core::disconnect ENTER");
        let _guard = self.inner.connect_lock.lock().await;
        diag_log("core::disconnect LOCK_ACQUIRED");
        // 同 connect_to 的死锁修复:不能持有 connection.lock() 的同时调
        // snapshot()(snapshot 内部会再 lock 同一把 mutex,tokio Mutex 不可
        // 重入)。先 drop guard,再调 snapshot。
        let already_idle = {
            let rec = self.inner.connection.lock().await;
            rec.state == ConnectionState::Idle
        };
        if already_idle {
            diag_log("core::disconnect IDEMPOTENT_RETURN");
            return Ok(self.connection_snapshot().await);
        }
        self.set_state(ConnectionState::Disconnecting, None, None).await;

        if let Some(hb) = self.inner.heartbeat.lock().await.take() {
            hb.stop().await;
        }
        self.rollback_system_proxy().await;
        self.inner.local_proxy.set_server_endpoint(None).await;

        {
            let mut rec = self.inner.connection.lock().await;
            rec.state = ConnectionState::Idle;
            rec.server = None;
            rec.connected_since = None;
        }
        self.publish_state_changed(ConnectionState::Idle, None, None);
        Ok(self.connection_snapshot().await)
    }

    // -------------------- 5 个 step 实现 --------------------

    async fn step_probe(&self, server: &DiscoveredServer) -> Result<(), String> {
        self.publish_progress(0, ConnectStepStatus::Running, &server.server_id, "");
        let r = probe(
            &server.host,
            server.port,
            server.socks,
            server.vpn,
            DEFAULT_PROBE_TIMEOUT,
        )
        .await;
        if !r.ok {
            let msg = r.error.unwrap_or_else(|| "probe failed".into());
            self.publish_progress(0, ConnectStepStatus::Failed, &server.server_id, &msg);
            return Err(msg);
        }
        self.publish_progress(
            0,
            ConnectStepStatus::Ok,
            &server.server_id,
            &format!("latency={:.0}ms", r.latency_ms),
        );
        Ok(())
    }

    async fn step_fetch_pac(&self, server: &DiscoveredServer) -> Result<String, String> {
        self.publish_progress(1, ConnectStepStatus::Running, &server.server_id, "");
        let url = format!("http://{}:{}{}", server.host, server.port, server.pac);
        let client = reqwest::Client::builder()
            .timeout(PAC_FETCH_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|e| format!("http client build: {e}"))?;
        let resp = client.get(&url).send().await.map_err(|e| {
            let m = format!("fetch_pac: {e}");
            self.publish_progress(1, ConnectStepStatus::Failed, &server.server_id, &m);
            m
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            let m = format!("fetch_pac body: {e}");
            self.publish_progress(1, ConnectStepStatus::Failed, &server.server_id, &m);
            m
        })?;
        if !status.is_success() {
            let m = format!("fetch_pac http status {status}");
            self.publish_progress(1, ConnectStepStatus::Failed, &server.server_id, &m);
            return Err(m);
        }
        self.publish_progress(
            1,
            ConnectStepStatus::Ok,
            &server.server_id,
            &format!("{} bytes", text.len()),
        );
        Ok(text)
    }

    async fn step_prefill_cache(
        &self,
        server: &DiscoveredServer,
        pac_text: &str,
    ) -> Result<(), String> {
        self.publish_progress(2, ConnectStepStatus::Running, &server.server_id, "");
        let rules = PacRules::parse(pac_text);
        // 第 1 段（本地 / 私有网段）→ DIRECT
        let mut count = 0usize;
        for d in rules.local_domains.iter().chain(rules.cn_direct_domains.iter()) {
            let host = d.trim_start_matches('.');
            self.inner.cache.set_with_ttl(
                host,
                RouteDirection::Direct,
                "pac",
                DEFAULT_PREFILL_TTL_SEC,
            );
            count += 1;
        }
        // 第 2 段（公司内网域名）→ PROXY
        for d in rules.internal_domains.iter() {
            let host = d.trim_start_matches('.');
            self.inner.cache.set_with_ttl(
                host,
                RouteDirection::Proxy,
                "pac",
                DEFAULT_PREFILL_TTL_SEC,
            );
            count += 1;
        }
        // 第 3 段（其它兜底）→ PROXY
        for d in rules.fallback_domains.iter() {
            let host = d.trim_start_matches('.');
            self.inner.cache.set_with_ttl(
                host,
                RouteDirection::Proxy,
                "pac",
                DEFAULT_PREFILL_TTL_SEC,
            );
            count += 1;
        }
        self.publish_progress(
            2,
            ConnectStepStatus::Ok,
            &server.server_id,
            &format!("{count} rules"),
        );
        Ok(())
    }

    async fn step_switch_endpoint(&self, server: &DiscoveredServer) -> Result<(), String> {
        self.publish_progress(3, ConnectStepStatus::Running, &server.server_id, "");
        let endpoint = ServerEndpoint {
            host: server.host.clone(),
            socks_port: server.socks,
        };
        self.inner
            .local_proxy
            .set_server_endpoint(Some(endpoint))
            .await;

        // System proxy 切换是"锦上添花":失败时只发警告 event,**不阻断连接**。
        // macOS 上 system_proxy.enable() 走 SystemConfiguration framework + 进程级
        // AuthorizationRef 缓存(详见 system_proxy_sc.rs):**首次连接弹 1 次原生
        // 密码框,之后整个进程内 0 次**。用户在密码框点取消 / 输错 → 失败但 Local
        // proxy 仍可用,UI 横幅提示手动配 SOCKS5,heartbeat 照常启动。
        let mut system_proxy_warning: Option<String> = None;
        if self.inner.config.enable_system_proxy {
            if let Err(e) = self.try_enable_system_proxy().await {
                warn!("[client_core] system_proxy enable failed (continuing anyway): {e}");
                system_proxy_warning = Some(e);
            }
        }

        let actual_port = self.inner.local_proxy.actual_port().await;
        let sp_state = if system_proxy_warning.is_some() {
            format!("系统代理切换失败,需手动配 SOCKS5 :{actual_port}")
        } else if self.inner.config.enable_system_proxy {
            "系统代理已切换".to_string()
        } else {
            format!("系统代理未启用,请手动配 SOCKS5 :{actual_port}")
        };

        if let Some(msg) = &system_proxy_warning {
            self.inner.bus.publish(ClientEvent::new(
                "system_proxy_warning",
                serde_json::json!({
                    "server_id": server.server_id,
                    "message": msg,
                    "manual_socks_port": actual_port,
                }),
            ));
        }

        self.publish_progress(
            3,
            ConnectStepStatus::Ok,
            &server.server_id,
            &format!(
                "upstream={}:{} · 本机 SOCKS5 :{actual_port} · {sp_state}",
                server.host, server.socks
            ),
        );
        Ok(())
    }

    async fn step_start_heartbeat(&self, server: &DiscoveredServer) -> Result<(), String> {
        self.publish_progress(4, ConnectStepStatus::Running, &server.server_id, "");
        let hb = Heartbeat::new(
            self.inner.bus.clone(),
            server.host.clone(),
            server.port,
            server.socks,
            local_client_name(),
            env!("CARGO_PKG_VERSION").to_string(),
        );
        hb.clone().start().await;
        *self.inner.heartbeat.lock().await = Some(hb);
        self.publish_progress(
            4,
            ConnectStepStatus::Ok,
            &server.server_id,
            "heartbeat started",
        );
        Ok(())
    }

    /// 任一 step 失败的统一收尾: 回滚部分已建立的状态 (heartbeat / system_proxy /
    /// local_proxy.endpoint) -> 把 ConnectionState 翻 Failed -> publish 事件.
    /// 不回滚的话 partial state 会卡住, 下一次 connect 出现奇怪状态.
    async fn fail_connect(&self, server_id: &str, error: &str) {
        warn!("[client_core] connect_to {server_id} failed: {error}");

        if let Some(hb) = self.inner.heartbeat.lock().await.take() {
            hb.stop().await;
        }
        self.rollback_system_proxy().await;
        self.inner.local_proxy.set_server_endpoint(None).await;

        {
            let mut rec = self.inner.connection.lock().await;
            rec.state = ConnectionState::Failed;
            rec.last_error = Some(error.to_string());
        }
        self.publish_state_changed(ConnectionState::Failed, Some(server_id), Some(error));
        let snap = self.connection_snapshot().await;
        self.publish_connect_done(server_id, &snap);
    }

    // -------------------- 私有 helper --------------------

    async fn set_state(
        &self,
        state: ConnectionState,
        server: Option<DiscoveredServer>,
        error: Option<String>,
    ) {
        {
            let mut rec = self.inner.connection.lock().await;
            rec.state = state;
            if server.is_some() {
                rec.server = server.clone();
            }
            rec.last_error = error.clone();
        }
        let server_id = server.as_ref().map(|s| s.server_id.clone());
        self.publish_state_changed(state, server_id.as_deref(), error.as_deref());
    }

    fn publish_state_changed(
        &self,
        state: ConnectionState,
        server_id: Option<&str>,
        error: Option<&str>,
    ) {
        let payload = serde_json::json!({
            "state": state.as_str(),
            "server_id": server_id,
            "error": error,
        });
        self.inner
            .bus
            .publish(ClientEvent::new("connection_state_changed", payload));
    }

    fn publish_progress(
        &self,
        step_idx: usize,
        status: ConnectStepStatus,
        server_id: &str,
        detail: &str,
    ) {
        let (key, label) = CONNECT_STEPS[step_idx];
        diag_log(&format!(
            "publish_progress step={} status={:?} detail={}",
            step_idx + 1,
            status,
            detail
        ));
        let payload = ConnectProgress {
            step: (step_idx + 1) as u8,
            total: CONNECT_STEPS.len() as u8,
            key,
            label,
            status,
            detail: detail.to_string(),
            server_id: server_id.to_string(),
        };
        self.inner.bus.publish(ClientEvent::new(
            "connect_progress",
            serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null),
        ));
    }

    fn publish_connect_done(&self, server_id: &str, snap: &ConnectionSnapshot) {
        let mut payload = serde_json::to_value(snap).unwrap_or(serde_json::Value::Null);
        if let serde_json::Value::Object(ref mut obj) = payload {
            obj.insert("server_id".into(), serde_json::Value::String(server_id.into()));
        }
        self.inner
            .bus
            .publish(ClientEvent::new("connect_done", payload));
    }

    /// 尝试设置系统代理. 失败时返回 Err,但**调用方 step_switch_endpoint 不会**
    /// **据此中断 connect 流程**——只发 system_proxy_warning event 让前端 toast.
    /// macOS 上走 SC framework + AuthorizationRef 缓存(首次弹 1 次密码,之后进
    /// 程内 0 次).
    ///
    /// 实现说明:`system_proxy.enable()` 内部调 macOS SCPreferences API,**单次
    /// 阻塞 2-4 秒**(每个 NetworkService ~0.5-1s,典型 4 张网卡总耗时 2-4s)。
    /// 不能直接 `await` 在 tokio runtime 上,会卡死 worker thread → SSE 进度事
    /// 件无法 publish → UI 看到"5 步全等待中"。所以走 `spawn_blocking` 把同步
    /// SC 调用挪到专用 blocking 线程池,UI 进度事件可以正常推送。
    async fn try_enable_system_proxy(&self) -> Result<(), String> {
        let actual_port = self.inner.local_proxy.actual_port().await;
        let host = self.inner.config.bind_host.clone();
        diag_log("try_enable_system_proxy SPAWN_BLOCKING");
        let result = tokio::task::spawn_blocking(move || {
            super::system_proxy::MacSystemProxy.enable(&host, actual_port)
        })
        .await
        .map_err(|e| format!("system_proxy enable task panic: {e}"))?;
        diag_log(&format!("try_enable_system_proxy DONE result={result:?}"));
        result?;
        info!(
            "[client_core] system proxy → {}:{actual_port}",
            self.inner.config.bind_host
        );
        *self.inner.system_proxy_active.lock().await = true;
        Ok(())
    }

    async fn rollback_system_proxy(&self) {
        diag_log("rollback_system_proxy ENTER");
        let mut active = self.inner.system_proxy_active.lock().await;
        if !*active {
            diag_log("rollback_system_proxy SKIP (not active)");
            return;
        }
        diag_log("rollback_system_proxy SPAWN_BLOCKING");
        let result = tokio::task::spawn_blocking(|| {
            super::system_proxy::MacSystemProxy.disable()
        })
        .await;
        diag_log(&format!("rollback_system_proxy DONE result={result:?}"));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!("[client_core] system_proxy disable failed: {e}"),
            Err(e) => warn!("[client_core] system_proxy disable task panic: {e}"),
        }
        *active = false;
    }

    /// （供 control_api 用）枚举 mDNS 发现到的 server 列表。
    pub async fn list_servers(&self) -> Vec<conduit_core::DiscoveredServer> {
        self.inner.discoverer.snapshot().await
    }

    /// （供 control_api 用）流量统计 snapshot。
    pub fn traffic_snapshot(&self) -> (u64, u64) {
        self.inner.traffic_meter.snapshot()
    }
}

fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// 取本机 hostname 作为客户端名上报给 server,fallback 到 `anonymous`。
/// 不依赖 hostname crate;先读 $HOSTNAME,再 fallback 到 macOS `scutil --get LocalHostName`。
fn local_client_name() -> String {
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !h.is_empty() {
            return h.split('.').next().unwrap_or(&h).to_string();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil")
            .args(["--get", "LocalHostName"])
            .output()
        {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !name.is_empty() {
                return name;
            }
        }
    }
    "anonymous".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 标 `#[ignore]`：会启动 mDNS daemon + 调 macOS networksetup，
    // 适合手动跑（`cargo test -- --ignored`），不进 CI / 默认 cargo test。
    #[tokio::test]
    #[ignore]
    async fn new_then_start_then_stop_does_not_panic() {
        let cfg = ClientConfig::with_ports(0, 0);
        let core = ClientCore::new(cfg);
        let result = core.start().await;
        assert!(result.is_ok(), "start failed: {result:?}");
        let snap = core.disconnect().await.unwrap();
        assert_eq!(snap.state, ConnectionState::Idle);
        core.stop().await;
    }

    #[tokio::test]
    async fn connection_snapshot_initial_is_idle() {
        let cfg = ClientConfig::with_ports(0, 0);
        let core = ClientCore::new(cfg);
        let snap = core.connection_snapshot().await;
        assert_eq!(snap.state, ConnectionState::Idle);
        assert!(snap.heartbeat.is_none());
        assert!(snap.server.is_none());
    }

    #[test]
    fn connect_steps_have_5_entries_with_zh_labels() {
        assert_eq!(CONNECT_STEPS.len(), 5);
        assert_eq!(CONNECT_STEPS[0].0, "probe");
        assert_eq!(CONNECT_STEPS[4].0, "start_heartbeat");
    }

    #[tokio::test]
    async fn connect_to_unreachable_server_emits_failed_progress_and_done() {
        // 192.0.2.1 是 TEST-NET-1，保证不可路由
        let cfg = ClientConfig::with_ports(0, 0);
        let core = ClientCore::new(cfg);
        let mut sub = core.bus().subscribe();

        let server = DiscoveredServer {
            server_id: "ghost@192.0.2.1:8080".into(),
            name: "ghost".into(),
            host: "192.0.2.1".into(),
            port: 8080,
            socks: 1080,
            api: 8090,
            vpn: false,
            version: "0.2.0".into(),
            pac: "/proxy.pac".into(),
            source: conduit_core::ServerSource::Manual,
            last_seen_at: 1.0,
            healthy: true,
        };
        let _ = core.connect_to(server).await;

        // 等几个事件出来
        let mut got_failed_progress = false;
        let mut got_done = false;
        for _ in 0..30 {
            if let Ok(Ok(ev)) =
                tokio::time::timeout(Duration::from_millis(200), sub.recv()).await
            {
                match ev.kind.as_str() {
                    "connect_progress" if ev.payload["status"] == "failed" => {
                        got_failed_progress = true;
                    }
                    "connect_done" => {
                        got_done = true;
                    }
                    _ => {}
                }
            }
            if got_failed_progress && got_done {
                break;
            }
        }
        assert!(got_failed_progress, "expected at least one failed progress event");
        assert!(got_done, "expected connect_done event");

        let snap = core.connection_snapshot().await;
        assert_eq!(snap.state, ConnectionState::Failed);
        assert!(snap.last_error.is_some());
    }
}
