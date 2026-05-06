//! Server proxy 配置 —— W2 Sprint 2 平移自 Python `server-app/core/config.py`。
//!
//! 字段名与默认值严格对齐 Python 版本，便于 UI 端 invoke 调用直接复用现有 schema。
//! 未来通过 `clap` 派生 CLI 参数；当前 S2.1 阶段先保证结构与 Python 等价，
//! 后续 sub-task 接入命令行 / Tauri IPC 时再补 derive。

use std::collections::HashSet;
use std::net::IpAddr;
use std::str::FromStr;

use ipnet::IpNet;

/// 默认允许的客户端来源 CIDR 列表（与 Python `Config.allowed_cidrs` 对齐）。
pub const DEFAULT_ALLOWED_CIDRS: &[&str] = &[
    "192.168.0.0/16",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "127.0.0.0/8",
];

/// 默认允许 CONNECT 的目标端口集合（与 Python `Config.allowed_connect_ports` 对齐）。
pub const DEFAULT_ALLOWED_CONNECT_PORTS: &[u16] =
    &[80, 443, 22, 8080, 8443, 8118, 8888, 9000, 9443];

/// PAC URL 的两个等价端点（HTTP proxy 监听端同时 serve 这两个路径）。
/// 当前 `proxy/http.rs` 直接 hardcode 这两条路径；保留常量给 control_api / IPC 反查用。
#[allow(dead_code)]
pub const PAC_ENDPOINTS: &[&str] = &["/proxy.pac", "/wpad.dat"];

/// Server-side 运行时配置。
///
/// 跟 Python `Config` 字段一一对应；UI 切 invoke 后传入的 JSON 会反序列化到这里
/// （S2.7 阶段会加 `serde::Deserialize` 派生）。
///
/// 部分字段（`api_bind_loopback_only` / `pac_file_path` / `log_level` /
/// `redact_query` / `direct_first` / `direct_first_timeout_s` /
/// `direct_cache_ttl_s` / `physical_iface_ip` / `traffic_sample_window_sec`）
/// 当前未被代码读取，留作 S2.5（DIRECT-first 路由）/ S2.7-round2（命令行参数）
/// 接入位。`#[allow(dead_code)]` 抑制 sprint-中-warning。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ProxyConfig {
    pub bind: String,
    pub http_port: u16,
    pub socks_port: u16,

    pub api_port: u16,
    pub api_bind_loopback_only: bool,

    pub mdns_enabled: bool,
    pub mdns_service_name: String,

    pub allowed_cidrs: Vec<String>,
    pub allowed_connect_ports: HashSet<u16>,

    pub pac_file_path: String,
    pub pac_advertised_host: String,

    pub log_level: String,
    pub redact_query: bool,

    pub handshake_timeout_s: f64,
    pub connect_timeout_s: f64,

    pub direct_first: bool,
    pub direct_first_timeout_s: f64,
    pub direct_cache_ttl_s: f64,
    pub physical_iface_ip: String,

    pub traffic_sample_window_sec: u64,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".into(),
            http_port: 8080,
            socks_port: 1080,
            api_port: 8090,
            api_bind_loopback_only: true,
            mdns_enabled: true,
            mdns_service_name: String::new(),
            allowed_cidrs: DEFAULT_ALLOWED_CIDRS.iter().map(|s| (*s).into()).collect(),
            allowed_connect_ports: DEFAULT_ALLOWED_CONNECT_PORTS.iter().copied().collect(),
            pac_file_path: "proxy.pac".into(),
            pac_advertised_host: String::new(),
            log_level: "INFO".into(),
            redact_query: true,
            handshake_timeout_s: 10.0,
            connect_timeout_s: 10.0,
            direct_first: true,
            direct_first_timeout_s: 1.5,
            direct_cache_ttl_s: 300.0,
            physical_iface_ip: String::new(),
            traffic_sample_window_sec: 600,
        }
    }
}

impl ProxyConfig {
    /// 用动态分配到的三个端口构造一个新配置（`AppState` 启动期会用）。
    pub fn with_ports(http_port: u16, socks_port: u16, api_port: u16) -> Self {
        Self {
            http_port,
            socks_port,
            api_port,
            ..Self::default()
        }
    }

    /// 校验 peer IP 是否落在 `allowed_cidrs` 任一网段内。
    /// CIDR 解析失败的条目被忽略，与 Python 端 `is_client_allowed` 行为一致。
    pub fn is_client_allowed(&self, peer_ip: &str) -> bool {
        let Ok(addr) = IpAddr::from_str(peer_ip) else {
            return false;
        };
        for cidr in &self.allowed_cidrs {
            let Ok(net) = IpNet::from_str(cidr) else {
                continue;
            };
            if net.contains(&addr) {
                return true;
            }
        }
        false
    }

    /// 校验目标端口是否落在 `allowed_connect_ports` 白名单内。
    pub fn is_connect_port_allowed(&self, port: u16) -> bool {
        self.allowed_connect_ports.contains(&port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_python_layout() {
        let cfg = ProxyConfig::default();
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.socks_port, 1080);
        assert_eq!(cfg.api_port, 8090);
        assert_eq!(cfg.allowed_cidrs.len(), 4);
        assert_eq!(cfg.allowed_connect_ports.len(), 9);
        assert!(cfg.allowed_connect_ports.contains(&443));
        assert!(cfg.direct_first);
    }

    #[test]
    fn allowed_client_lan_ips() {
        let cfg = ProxyConfig::default();
        assert!(cfg.is_client_allowed("192.168.1.10"));
        assert!(cfg.is_client_allowed("10.0.0.5"));
        assert!(cfg.is_client_allowed("172.20.0.1"));
        assert!(cfg.is_client_allowed("127.0.0.1"));
    }

    #[test]
    fn rejects_public_and_invalid_ips() {
        let cfg = ProxyConfig::default();
        assert!(!cfg.is_client_allowed("8.8.8.8"));
        assert!(!cfg.is_client_allowed("172.32.0.1")); // 172.32 不在 12-31 范围
        assert!(!cfg.is_client_allowed("not-an-ip"));
    }

    #[test]
    fn skips_malformed_cidr_entries() {
        let mut cfg = ProxyConfig::default();
        cfg.allowed_cidrs.push("garbage".into());
        cfg.allowed_cidrs.push("10.0.0.0/33".into());
        assert!(cfg.is_client_allowed("192.168.1.1")); // 仍然走有效条目
    }

    #[test]
    fn connect_port_allowlist_round_trip() {
        let cfg = ProxyConfig::default();
        assert!(cfg.is_connect_port_allowed(443));
        assert!(!cfg.is_connect_port_allowed(6379));
    }
}
