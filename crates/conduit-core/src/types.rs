//! 双端共享 wire types —— UI <→ Tauri <→ conduit-core 三方都用同一份 schema。
//!
//! 字段名严格对齐 Python 端的 dataclass / dict payload（snake_case），具体来源：
//! - `client-app/core/discoverer.py::DiscoveredServer`
//! - `server-app/core/active_connections.py::ConnectionInfo`
//! - `server-app/core/healthcheck.py::CheckResult` + `to_dict()` 输出
//! - `client-app/core/connectivity.py::ProbeResult`
//!
//! S1.5 会接入 [`specta`] 自动生成 TS 端 binding；当前先用 `serde` 保证 wire-format。
//!
//! 字段顺序、可选性、默认值任何变化都需要双向同步 Python 端。

use serde::{Deserialize, Serialize};

/// `DiscoveredServer.source` —— 该 server 当前是怎么被记录到的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerSource {
    /// 当前 mDNS 在线广播看到。
    Mdns,
    /// 仅来自历史持久化文件 `known-servers.json`，未必在线。
    History,
    /// 用户手动添加（IP 直填）。
    Manual,
}

/// 单个 Conduit Server 记录。来源可能是 mDNS、历史文件或手动添加。
///
/// 字段顺序与 Python `DiscoveredServer` 完全一致，UI 直接用同名结构反序列化。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredServer {
    /// `name@host:port` 形式的稳定标识（同一服务跨 session 保持一致）。
    pub server_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub socks: u16,
    pub api: u16,
    pub vpn: bool,
    pub version: String,
    /// PAC URL 相对路径（默认 `/proxy.pac`）。
    pub pac: String,
    pub source: ServerSource,
    /// epoch seconds（与 Python `time.time()` 对齐）。
    pub last_seen_at: f64,
    pub healthy: bool,
}

impl DiscoveredServer {
    /// 拼接完整 PAC URL：`http://{host}:{port}{pac}`，与 Python 端 property 等价。
    pub fn pac_url(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.pac)
    }

    /// 生成 `name@host:port` 形式的 server_id（与 Python `_make_server_id` 等价）。
    pub fn make_server_id(name: &str, host: &str, port: u16) -> String {
        format!("{name}@{host}:{port}")
    }
}

/// 单个进行中的代理会话 —— Server 端 `active_connections.ConnectionInfo` 的镜像。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// 形如 `s17` 的单调递增 id。
    pub session_id: String,
    pub peer_ip: String,
    /// `"http"` 或 `"socks5"`。
    pub proto: String,
    /// `host:port` 目标地址。
    pub target: String,
    pub since: f64,
    pub last_seen: f64,
    #[serde(default)]
    pub sent_bytes: u64,
    #[serde(default)]
    pub recv_bytes: u64,
}

/// 单条健康检查项（端口监听 / LAN IP / VPN tunnel）。
///
/// 对齐 Python `healthcheck.CheckResult`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub name: String,
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

/// 聚合健康检查响应（Python `HealthCheck.to_dict()` 的 schema）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthSummary {
    pub ready: bool,
    pub checks: Vec<HealthCheckResult>,
    /// 是否在跑（Tauri 主进程模型下基本恒为 true）。
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub uptime_sec: f64,
}

/// 客户端一次性可达性 probe 的结果，对齐 Python `connectivity.ProbeResult`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub ok: bool,
    pub healthz_ok: bool,
    pub socks_reachable: bool,
    pub http_reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub latency_ms: f64,
    #[serde(default)]
    pub server_vpn: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_discovered() -> DiscoveredServer {
        DiscoveredServer {
            server_id: "host01@192.168.1.10:8080".into(),
            name: "host01".into(),
            host: "192.168.1.10".into(),
            port: 8080,
            socks: 1080,
            api: 8090,
            vpn: true,
            version: "0.1.4".into(),
            pac: "/proxy.pac".into(),
            source: ServerSource::Mdns,
            last_seen_at: 1_780_000_000.0,
            healthy: true,
        }
    }

    #[test]
    fn discovered_server_serde_roundtrip_matches_python_payload() {
        let ds = sample_discovered();
        let json = serde_json::to_string(&ds).unwrap();
        // 关键字段必须在 JSON 中以 Python 端预期的 snake_case 出现
        assert!(json.contains("\"server_id\""));
        assert!(json.contains("\"last_seen_at\""));
        assert!(json.contains("\"source\":\"mdns\""));
        let back: DiscoveredServer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ds);
    }

    #[test]
    fn pac_url_helper_matches_python_property() {
        assert_eq!(
            sample_discovered().pac_url(),
            "http://192.168.1.10:8080/proxy.pac"
        );
    }

    #[test]
    fn make_server_id_matches_python_format() {
        assert_eq!(
            DiscoveredServer::make_server_id("host01", "192.168.1.10", 8080),
            "host01@192.168.1.10:8080"
        );
    }

    #[test]
    fn server_source_serializes_lowercase() {
        for (src, want) in [
            (ServerSource::Mdns, "\"mdns\""),
            (ServerSource::History, "\"history\""),
            (ServerSource::Manual, "\"manual\""),
        ] {
            let got = serde_json::to_string(&src).unwrap();
            assert_eq!(got, want, "source {src:?} should serialize as {want}");
        }
    }

    #[test]
    fn connection_info_default_byte_counters() {
        let json = r#"{
            "session_id": "s1", "peer_ip": "10.0.0.2", "proto": "socks5",
            "target": "example.com:443", "since": 1.0, "last_seen": 1.0
        }"#;
        let ci: ConnectionInfo = serde_json::from_str(json).unwrap();
        assert_eq!(ci.sent_bytes, 0);
        assert_eq!(ci.recv_bytes, 0);
    }

    #[test]
    fn health_summary_optional_uptime() {
        let json =
            r#"{"ready":true,"checks":[{"name":"http_port","ok":true,"detail":"open"}]}"#;
        let hs: HealthSummary = serde_json::from_str(json).unwrap();
        assert!(hs.ready);
        assert_eq!(hs.checks.len(), 1);
        assert_eq!(hs.uptime_sec, 0.0);
        assert!(!hs.running);
    }

    #[test]
    fn probe_result_skips_none_error() {
        let r = ProbeResult {
            ok: true,
            healthz_ok: true,
            socks_reachable: true,
            http_reachable: true,
            error: None,
            latency_ms: 12.3,
            server_vpn: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("\"error\""), "error: None should be skipped, got {json}");
    }
}
