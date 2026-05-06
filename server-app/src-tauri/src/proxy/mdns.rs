//! mDNS / Bonjour service advertiser —— 平移自 Python `server-app/core/mdns_advertiser.py`。
//!
//! 用 [`mdns_sd::ServiceDaemon`] 注册 `_conduit._tcp.local.` 服务，让同 LAN 的
//! Conduit Client 能自动发现 server。
//!
//! TXT 字段构造完全交给 [`conduit_core::mdns::build_txt`]，与 client-app 端的
//! [`conduit_core::mdns::parse_txt`] 形成单点契约——任何字段调整只需改 conduit-core。
//!
//! 行为差异 vs Python：
//! - 检测 LAN IP 优先用 `local-ip-address::local_ip()`（macOS 上对应默认路由
//!   的网卡），失败时 fallback 到 `127.0.0.1`，与 Python `socket.gethostbyname`
//!   的语义对齐。
//! - 重名时 mdns-sd 不像 Python `zeroconf` 自动追加 `#2`，但同名重新注册会
//!   覆盖旧条目（见 `ServiceDaemon::register` 文档）。该行为对我们够用。

use std::time::Duration;

use conduit_core::mdns::{build_txt, MdnsServiceInfo, DEFAULT_PAC_PATH, SERVICE_TYPE};
use log::{info, warn};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio_util::sync::CancellationToken;

use super::core::ProxyCore;

/// 当前服务版本（用于 TXT `version` 字段）。
pub const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 检测一个用于广播的 LAN IP。失败时返回 `127.0.0.1`，让本地 client 至少能用。
fn detect_lan_ip() -> String {
    match local_ip_address::local_ip() {
        Ok(ip) => ip.to_string(),
        Err(e) => {
            warn!("[mdns] failed to detect LAN IP: {e}, falling back to 127.0.0.1");
            "127.0.0.1".to_string()
        }
    }
}

fn detect_hostname() -> String {
    // hostname crate 没引入；用 std env 的 fallback，与 Python `gethostname` 类似
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !h.is_empty() {
            return h.split('.').next().unwrap_or(&h).to_string();
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                return s.trim().split('.').next().unwrap_or(s.trim()).to_string();
            }
        }
    }
    "conduit-server".into()
}

/// 启动 mDNS 服务广播。任务在 cancel-token 触发时 unregister + close daemon。
///
/// vpn 状态会在 `core.update_vpn(...)` 后由 `EventBus<ServerEvent>` 收到通知，
/// 这里订阅事件做"刷新 TXT"。
pub async fn run(core: ProxyCore, cancel: CancellationToken) {
    let cfg = core.config();
    if !cfg.mdns_enabled {
        info!("[mdns] disabled by config");
        cancel.cancelled().await;
        return;
    }

    let host_ip = if !cfg.pac_advertised_host.is_empty() {
        cfg.pac_advertised_host.clone()
    } else {
        detect_lan_ip()
    };
    let instance_name = if !cfg.mdns_service_name.is_empty() {
        cfg.mdns_service_name.clone()
    } else {
        detect_hostname()
    };

    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            warn!("[mdns] daemon spawn failed: {e}; advertise disabled");
            cancel.cancelled().await;
            return;
        }
    };

    let mut current_vpn = false;
    let info = build_service_info(&instance_name, &host_ip, &cfg, current_vpn);
    if let Err(e) = daemon.register(info.clone()) {
        warn!("[mdns] initial register failed: {e}; advertise disabled");
        let _ = daemon.shutdown();
        cancel.cancelled().await;
        return;
    }
    info!(
        "[mdns] advertised {} @ {}:{} (vpn={})",
        info.get_fullname(),
        host_ip,
        cfg.http_port,
        if current_vpn { "on" } else { "off" }
    );

    // 订阅 VPN 状态变更，刷新 TXT
    let mut bus_rx = core.event_bus().subscribe();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            evt = bus_rx.recv() => {
                let Ok(evt) = evt else { continue };
                if evt.kind != "vpn_changed" {
                    continue;
                }
                let new_vpn = evt.payload
                    .get("vpn_on").and_then(|v| v.as_bool()).unwrap_or(false);
                if new_vpn == current_vpn {
                    continue;
                }
                current_vpn = new_vpn;
                let updated = build_service_info(&instance_name, &host_ip, &cfg, new_vpn);
                if let Err(e) = daemon.register(updated.clone()) {
                    warn!("[mdns] re-register on vpn change failed: {e}");
                } else {
                    info!("[mdns] TXT updated: vpn={}", if new_vpn { "on" } else { "off" });
                }
            }
        }
    }

    if let Err(e) = daemon.unregister(&info.get_fullname()) {
        warn!("[mdns] unregister failed: {e}");
    }
    // 给 unregister goodbye 包一点发送时间
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = daemon.shutdown();
    info!("[mdns] daemon shut down");
}

/// 用 conduit-core 的 [`build_txt`] 渲染 TXT，再加上 mdns-sd 自己的 SRV 字段。
fn build_service_info(
    instance: &str,
    host_ip: &str,
    cfg: &super::config::ProxyConfig,
    vpn_on: bool,
) -> ServiceInfo {
    let core_info = MdnsServiceInfo {
        name: instance.to_string(),
        http_port: cfg.http_port,
        socks_port: cfg.socks_port,
        api_port: cfg.api_port,
        vpn_on,
        version: SERVICE_VERSION.to_string(),
        pac_path: DEFAULT_PAC_PATH.to_string(),
    };
    let txt = build_txt(&core_info);
    let server_fqdn = core_info.server_fqdn();

    ServiceInfo::new(
        SERVICE_TYPE,
        instance,
        &server_fqdn,
        host_ip,
        cfg.http_port,
        Some(txt),
    )
    .expect("build mdns ServiceInfo")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::ProxyConfig;

    #[test]
    fn build_service_info_uses_conduit_core_txt() {
        let cfg = ProxyConfig::with_ports(8081, 1081, 8091);
        let info = build_service_info("hosttest", "127.0.0.1", &cfg, true);
        assert_eq!(info.get_port(), 8081);
        assert_eq!(info.get_type(), SERVICE_TYPE);
        let props: std::collections::HashMap<String, String> = info
            .get_properties()
            .iter()
            .map(|p| (p.key().to_string(), p.val_str().to_string()))
            .collect();
        assert_eq!(props.get("port").map(String::as_str), Some("8081"));
        assert_eq!(props.get("socks").map(String::as_str), Some("1081"));
        assert_eq!(props.get("api").map(String::as_str), Some("8091"));
        assert_eq!(props.get("vpn").map(String::as_str), Some("on"));
        assert!(props.get("pac").map(|s| s == DEFAULT_PAC_PATH).unwrap_or(false));
    }

    #[test]
    fn build_service_info_off_vpn_renders_off() {
        let cfg = ProxyConfig::default();
        let info = build_service_info("hosttest", "127.0.0.1", &cfg, false);
        let props: std::collections::HashMap<String, String> = info
            .get_properties()
            .iter()
            .map(|p| (p.key().to_string(), p.val_str().to_string()))
            .collect();
        assert_eq!(props.get("vpn").map(String::as_str), Some("off"));
    }

    #[test]
    fn detect_hostname_returns_non_empty() {
        let h = detect_hostname();
        assert!(!h.is_empty());
        assert!(!h.contains('\n'));
    }
}
