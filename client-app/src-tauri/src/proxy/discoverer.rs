//! `Discoverer` —— mDNS server 自动发现 + 本地历史持久化（用户曾连接过的 server）。
//!
//! 职责：
//! - 启动 `mdns-sd` 浏览 `_conduit._tcp.local.`，把发现的 server 解析为
//!   [`conduit_core::DiscoveredServer`]。
//! - 将每个发现合并到内存表 + `known-servers.json` 持久化文件。
//! - 通过 [`EventBus`] 推 `server_discovered` / `server_lost` 事件给 UI/SSE。
//!
//! W3 Sprint 3 当前阶段：实现核心订阅 + 持久化（基础版本，避免一次性塞太多）。
//! 后续可加 service-resolved 重试、TXT 校验失败处理等。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use log::{debug, info, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use conduit_core::mdns::{parse_txt, MdnsParseError, MdnsServiceInfo, SERVICE_TYPE};
use conduit_core::{DiscoveredServer, EventBus, ServerSource};

use super::core::ClientEvent;

pub struct Discoverer {
    bus: EventBus<ClientEvent>,
    state: Arc<Mutex<DiscovererState>>,
    cancel: CancellationToken,
    handle: Mutex<Option<JoinHandle<()>>>,
    storage_path: PathBuf,
}

struct DiscovererState {
    /// 当前内存中的 server 表（key = server_id）。
    servers: HashMap<String, DiscoveredServer>,
}

impl Discoverer {
    pub fn new(bus: EventBus<ClientEvent>) -> Arc<Self> {
        Arc::new(Self {
            bus,
            state: Arc::new(Mutex::new(DiscovererState {
                servers: HashMap::new(),
            })),
            cancel: CancellationToken::new(),
            handle: Mutex::new(None),
            storage_path: default_storage_path(),
        })
    }

    /// 全量快照（mDNS 在线 + 历史合并），用于 `GET /api/servers`。
    pub async fn snapshot(&self) -> Vec<DiscoveredServer> {
        let state = self.state.lock().await;
        let mut out: Vec<DiscoveredServer> = state.servers.values().cloned().collect();
        out.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        out
    }

    pub async fn get_by_id(&self, server_id: &str) -> Option<DiscoveredServer> {
        self.state.lock().await.servers.get(server_id).cloned()
    }

    /// 从内存与历史中移除单条 server。返回是否真的删除了。
    pub async fn forget(&self, server_id: &str) -> bool {
        let mut state = self.state.lock().await;
        let removed = state.servers.remove(server_id).is_some();
        let snapshot: Vec<DiscoveredServer> = state.servers.values().cloned().collect();
        drop(state);
        if removed {
            save_history(&self.storage_path, &snapshot);
        }
        removed
    }

    /// 清空"历史"类条目（用户曾连接过但当前不在线广播的）。
    /// 在线 mDNS server (`source = Mdns`) 与手动添加 (`source = Manual`) 不受影响,
    /// 与 UI 文案 "在线 server 不受影响" 严格对齐。
    /// 返回真正删除的条目数(供 toast 显示)。
    pub async fn forget_all(&self) -> usize {
        let mut state = self.state.lock().await;
        let before = state.servers.len();
        state.servers.retain(|_, srv| srv.source != ServerSource::History);
        let after = state.servers.len();
        let snapshot: Vec<DiscoveredServer> = state.servers.values().cloned().collect();
        drop(state);
        save_history(&self.storage_path, &snapshot);
        before - after
    }

    /// 启动 mDNS 浏览。失败时返回 Err（调用方决定是否阻断启动）。
    pub async fn start(self: &Arc<Self>) -> Result<(), String> {
        // 先把历史 known-servers.json 装载进内存（标 source=history）
        let history = load_history(&self.storage_path);
        if !history.is_empty() {
            let mut state = self.state.lock().await;
            for srv in history {
                state.servers.insert(srv.server_id.clone(), srv);
            }
            info!(
                "[discoverer] loaded {} historical servers from {}",
                state.servers.len(),
                self.storage_path.display()
            );
        }

        let daemon =
            ServiceDaemon::new().map_err(|e| format!("mdns daemon spawn failed: {e}"))?;
        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| format!("mdns browse failed: {e}"))?;

        let me = self.clone();
        let cancel = self.cancel.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        info!("[discoverer] shutdown requested");
                        let _ = daemon.shutdown();
                        return;
                    }
                    event = async { receiver.recv_async().await } => {
                        match event {
                            Ok(ServiceEvent::ServiceResolved(info)) => {
                                me.handle_resolved(&info).await;
                            }
                            Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                                me.handle_removed(&fullname).await;
                            }
                            Ok(ev) => debug!("[discoverer] mdns event: {ev:?}"),
                            Err(e) => {
                                warn!("[discoverer] mdns recv error: {e}, exiting browse loop");
                                return;
                            }
                        }
                    }
                }
            }
        });
        *self.handle.lock().await = Some(task);
        Ok(())
    }

    pub async fn stop(&self) {
        self.cancel.cancel();
        if let Some(h) = self.handle.lock().await.take() {
            let _ = h.await;
        }
        info!("[discoverer] stopped");
    }

    async fn handle_resolved(&self, info: &mdns_sd::ServiceInfo) {
        let txt: HashMap<String, String> = info
            .get_properties()
            .iter()
            .map(|p| (p.key().to_string(), p.val_str().to_string()))
            .collect();
        // 优先选 IPv4 全局地址（loopback / link-local IPv6 不可用于跨 LAN SOCKS）。
        // 选择顺序：IPv4 公/私网 → IPv4 link-local（169.254）→ IPv6 全局 → IPv6 link-local → hostname。
        let host = pick_preferred_address(info)
            .unwrap_or_else(|| info.get_hostname().to_string());
        let svc: MdnsServiceInfo = match parse_txt(&txt, info.get_port()) {
            Ok(v) => v,
            Err(MdnsParseError::MissingField(k)) => {
                warn!("[discoverer] TXT missing field '{k}', skipping");
                return;
            }
            Err(e) => {
                warn!("[discoverer] TXT parse error: {e:?}, skipping");
                return;
            }
        };
        let server_id = DiscoveredServer::make_server_id(&svc.name, &host, svc.http_port);
        let now = epoch_now();
        let server = DiscoveredServer {
            server_id: server_id.clone(),
            name: svc.name.clone(),
            host: host.clone(),
            port: svc.http_port,
            socks: svc.socks_port,
            api: svc.api_port,
            vpn: svc.vpn_on,
            version: svc.version.clone(),
            pac: svc.pac_path.clone(),
            source: ServerSource::Mdns,
            last_seen_at: now,
            healthy: true,
        };
        let mut state = self.state.lock().await;
        let is_new = !state.servers.contains_key(&server_id);
        state.servers.insert(server_id.clone(), server.clone());
        // 持久化（best-effort）
        let snapshot: Vec<DiscoveredServer> = state.servers.values().cloned().collect();
        drop(state);
        save_history(&self.storage_path, &snapshot);

        let kind = if is_new { "server_discovered" } else { "server_updated" };
        self.bus.publish(ClientEvent::new(
            kind,
            serde_json::to_value(&server).unwrap_or(serde_json::Value::Null),
        ));
        info!("[discoverer] {kind}: {server_id} → {}:{}", server.host, server.port);
    }

    async fn handle_removed(&self, fullname: &str) {
        // mDNS lost：把 source 标 history（保留在历史中）
        let mut state = self.state.lock().await;
        let mut payload: Option<DiscoveredServer> = None;
        for (_id, server) in state.servers.iter_mut() {
            // mdns-sd 的 fullname 形如 `name._conduit._tcp.local.`；用 name 前缀匹配
            if fullname.starts_with(&format!("{}.", server.name)) {
                server.source = ServerSource::History;
                server.healthy = false;
                payload = Some(server.clone());
                break;
            }
        }
        let snapshot: Vec<DiscoveredServer> = state.servers.values().cloned().collect();
        drop(state);
        save_history(&self.storage_path, &snapshot);

        if let Some(s) = payload {
            self.bus.publish(ClientEvent::new(
                "server_lost",
                serde_json::to_value(&s).unwrap_or(serde_json::Value::Null),
            ));
            info!("[discoverer] server_lost: {}", s.server_id);
        }
    }
}

/// 从 mDNS resolved info 里挑一个最适合 SOCKS5 跨 LAN 拨号的地址。
///
/// 优先级（从高到低）：
/// 1. IPv4 私网/公网（10/172.16/192.168/常规公网）
/// 2. IPv4 link-local（169.254.x.x，作为兜底）
/// 3. IPv6 全局
/// 4. IPv6 link-local（`fe80::…` 需要 zone-id，对 SOCKS5 不友好，最低优先级）
fn pick_preferred_address(info: &mdns_sd::ServiceInfo) -> Option<String> {
    use std::net::IpAddr;
    let mut v4_global: Option<IpAddr> = None;
    let mut v4_linklocal: Option<IpAddr> = None;
    let mut v6_global: Option<IpAddr> = None;
    let mut v6_linklocal: Option<IpAddr> = None;
    for addr in info.get_addresses() {
        match addr {
            IpAddr::V4(v4) => {
                if v4.is_link_local() {
                    v4_linklocal.get_or_insert(*addr);
                } else {
                    v4_global.get_or_insert(*addr);
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_unicast_link_local() {
                    v6_linklocal.get_or_insert(*addr);
                } else {
                    v6_global.get_or_insert(*addr);
                }
            }
        }
    }
    v4_global
        .or(v4_linklocal)
        .or(v6_global)
        .or(v6_linklocal)
        .map(|ip| ip.to_string())
}

fn default_storage_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        let mac_dir = home.join("Library").join("Application Support").join("Conduit");
        if mac_dir.exists() || mac_dir.parent().map(|p| p.exists()).unwrap_or(false) {
            return mac_dir.join("known-servers.json");
        }
        return home.join(".conduit").join("known-servers.json");
    }
    PathBuf::from("known-servers.json")
}

fn load_history(path: &PathBuf) -> Vec<DiscoveredServer> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<DiscoveredServer>>(&raw) {
        Ok(mut list) => {
            for srv in list.iter_mut() {
                srv.source = ServerSource::History;
                srv.healthy = false;
            }
            list
        }
        Err(e) => {
            warn!("[discoverer] known-servers.json malformed: {e}");
            Vec::new()
        }
    }
}

fn save_history(path: &PathBuf, servers: &[DiscoveredServer]) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!("[discoverer] mkdir {} failed: {e}", parent.display());
            return;
        }
    }
    let json = match serde_json::to_string_pretty(servers) {
        Ok(s) => s,
        Err(e) => {
            warn!("[discoverer] serialize known-servers failed: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(path, json) {
        warn!("[discoverer] write {} failed: {e}", path.display());
    }
}

fn epoch_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn snapshot_initial_is_empty() {
        let bus: EventBus<ClientEvent> = EventBus::new(8);
        let d = Discoverer::new(bus);
        assert!(d.snapshot().await.is_empty());
    }

    #[tokio::test]
    async fn save_and_load_history_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("known-servers.json");
        let server = DiscoveredServer {
            server_id: "alice@10.0.0.5:8080".into(),
            name: "alice".into(),
            host: "10.0.0.5".into(),
            port: 8080,
            socks: 1080,
            api: 8090,
            vpn: false,
            version: "0.2.0".into(),
            pac: "/proxy.pac".into(),
            source: ServerSource::Mdns,
            last_seen_at: 1_780_000_000.0,
            healthy: true,
        };
        save_history(&path, std::slice::from_ref(&server));
        let loaded = load_history(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].server_id, "alice@10.0.0.5:8080");
        // load_history 强制把 source 改成 History（提示这是上次见过的）
        assert_eq!(loaded[0].source, ServerSource::History);
        assert!(!loaded[0].healthy);
    }

    #[test]
    fn default_storage_path_returns_some_path() {
        let p = default_storage_path();
        assert!(p.to_string_lossy().contains("known-servers.json"));
    }

    /// `forget_all` 必须只清 source=History 的条目, 在线 mDNS / Manual server 不受影响,
    /// 与 UI 文案 "在线 server 不受影响" 严格对齐 (Bug A 回归保护)。
    #[tokio::test]
    async fn forget_all_keeps_online_mdns_and_manual_servers() {
        let bus: EventBus<ClientEvent> = EventBus::new(8);
        let d = Discoverer::new(bus);
        let mk = |name: &str, src: ServerSource| DiscoveredServer {
            server_id: format!("{name}@10.0.0.1:8080"),
            name: name.into(),
            host: "10.0.0.1".into(),
            port: 8080,
            socks: 1080,
            api: 8090,
            vpn: false,
            version: "0.2.0".into(),
            pac: "/proxy.pac".into(),
            source: src,
            last_seen_at: 1_780_000_000.0,
            healthy: true,
        };
        {
            let mut state = d.state.lock().await;
            for srv in [
                mk("live-mdns", ServerSource::Mdns),
                mk("manual-add", ServerSource::Manual),
                mk("hist-1", ServerSource::History),
                mk("hist-2", ServerSource::History),
            ] {
                state.servers.insert(srv.server_id.clone(), srv);
            }
        }
        let removed = d.forget_all().await;
        assert_eq!(removed, 2, "should remove only the 2 history entries");
        let snap = d.snapshot().await;
        let names: Vec<&str> = snap.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"live-mdns"));
        assert!(names.contains(&"manual-add"));
        assert!(!names.iter().any(|n| n.starts_with("hist-")));
    }
}
