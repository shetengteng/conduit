//! `ClientConfig` —— client-app 启动配置（监听端口、PAC TTL、代理超时等）。
//!
//! 全部字段使用 snake_case 序列化，方便与 UI 端 / control_api JSON 直接对齐。

use serde::{Deserialize, Serialize};

/// 默认 SOCKS5 listener 监听地址。
pub const DEFAULT_BIND_HOST: &str = "127.0.0.1";

/// 默认 SOCKS5 listener 端口。
pub const DEFAULT_BIND_PORT: u16 = 7890;

/// PAC 默认相对路径（server-app 暴露在 `:http_port/proxy.pac`）。
pub const DEFAULT_PAC_PATH: &str = "/proxy.pac";

/// 客户端运行配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// 本地 SOCKS5 listener bind host。
    pub bind_host: String,
    /// 本地 SOCKS5 listener bind port（0 = 让 OS 分配）。
    pub bind_port: u16,
    /// 控制 API loopback bind port（0 = 让 OS 分配）。
    pub api_port: u16,
    /// 启动时如果指定了上游 server，可以预先 prefill PAC。
    pub server_host: Option<String>,
    /// 上游 server HTTP port（PAC 也走这个端口）。
    pub server_port: u16,
    /// PAC URL 相对路径。
    pub pac_path: String,
    /// 是否触发 macOS `networksetup` 改系统代理。
    pub enable_system_proxy: bool,
}

impl ClientConfig {
    /// 用预先分配好的端口生成默认配置（与 Tauri shell `pick_two_ports` 配合）。
    pub fn with_ports(bind_port: u16, api_port: u16) -> Self {
        Self {
            bind_host: DEFAULT_BIND_HOST.to_string(),
            bind_port,
            api_port,
            server_host: None,
            server_port: 8080,
            pac_path: DEFAULT_PAC_PATH.to_string(),
            enable_system_proxy: true,
        }
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::with_ports(DEFAULT_BIND_PORT, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_legacy_ports() {
        let cfg = ClientConfig::default();
        assert_eq!(cfg.bind_host, "127.0.0.1");
        assert_eq!(cfg.bind_port, 7890);
        assert_eq!(cfg.pac_path, "/proxy.pac");
        assert!(cfg.enable_system_proxy);
    }

    #[test]
    fn with_ports_overrides_correctly() {
        let cfg = ClientConfig::with_ports(12345, 23456);
        assert_eq!(cfg.bind_port, 12345);
        assert_eq!(cfg.api_port, 23456);
    }
}
