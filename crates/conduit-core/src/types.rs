//! 双端共享 wire types —— UI <→ Tauri <→ conduit-core 三方都用同一份 schema。
//!
//! 字段全部 snake_case 序列化，UI（TypeScript）端可直接以同名字段反序列化。
//!
//! 修改任何字段（重命名 / 改类型 / 改可选性 / 改默认值）都需要：
//! 1. 同步更新 `server-app/ui/src/types/*` 与 `client-app/ui/src/types/*` 中的 TS 类型；
//! 2. 跑 `cargo test -p conduit-core` 验证 serde round-trip。

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
    /// epoch seconds（UTC，UI 端 `Date.now()/1000` 同口径）。
    pub last_seen_at: f64,
    pub healthy: bool,
}

impl DiscoveredServer {
    /// 拼接完整 PAC URL：`http://{host}:{port}{pac}`。
    pub fn pac_url(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.pac)
    }

    /// 生成 `name@host:port` 形式的 server_id（跨 session 稳定）。
    pub fn make_server_id(name: &str, host: &str, port: u16) -> String {
        format!("{name}@{host}:{port}")
    }
}

/// 单个进行中的代理会话 —— Server 端活跃连接面板用。
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub name: String,
    pub ok: bool,
    #[serde(default)]
    pub detail: String,
}

/// 聚合健康检查响应（`/api/healthz` 与 UI Network 面板共用）。
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

/// 客户端一次性可达性 probe 的结果（连接前的可达性检查 + 心跳的单次结果）。
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

/// 路由方向：`direct` 直连，`proxy` 走上游 server。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteDirection {
    Direct,
    Proxy,
}

/// 路由缓存条目（`RouteCache` 内部存储与 `/api/route_cache` 列表项）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteEntry {
    pub host: String,
    pub direction: RouteDirection,
    /// epoch seconds 过期时间。
    pub expires_at: f64,
    /// `"pac"` / `"probe"` / `"manual"` 等。
    pub source: String,
    #[serde(default)]
    pub hit_count: u64,
}

/// 心跳健康度颜色（绿/黄/红），用于顶栏徽标和 UI 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeartbeatTone {
    Green,
    Yellow,
    Red,
}

/// 心跳状态快照（一次心跳轮询的结果 + 连续失败计数）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatState {
    pub tone: HeartbeatTone,
    pub consecutive_failures: u32,
    #[serde(default)]
    pub last_check_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 客户端连接生命周期状态（5 步连接状态机的对外表达）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Failed,
    Disconnecting,
}

impl ConnectionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionState::Idle => "idle",
            ConnectionState::Connecting => "connecting",
            ConnectionState::Connected => "connected",
            ConnectionState::Failed => "failed",
            ConnectionState::Disconnecting => "disconnecting",
        }
    }
}

/// `ConnectionSnapshot.server` 的精简字段集，对齐 UI `ConnectedServerSummary`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectedServerSummary {
    pub server_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub socks: u16,
    pub api: u16,
    pub vpn: bool,
    pub version: String,
}

impl From<&DiscoveredServer> for ConnectedServerSummary {
    fn from(s: &DiscoveredServer) -> Self {
        Self {
            server_id: s.server_id.clone(),
            name: s.name.clone(),
            host: s.host.clone(),
            port: s.port,
            socks: s.socks,
            api: s.api,
            vpn: s.vpn,
            version: s.version.clone(),
        }
    }
}

/// `ConnectionSnapshot.heartbeat`（精简版心跳，UI `ConnectionSnapshot.heartbeat` 形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionHeartbeat {
    pub tone: HeartbeatTone,
    pub consecutive_failures: u32,
    pub last_check_at: f64,
    pub last_error: Option<String>,
}

impl From<&HeartbeatState> for ConnectionHeartbeat {
    fn from(s: &HeartbeatState) -> Self {
        Self {
            tone: s.tone,
            consecutive_failures: s.consecutive_failures,
            last_check_at: s.last_check_at,
            last_error: s.last_error.clone(),
        }
    }
}

/// 客户端 `/api/connection` 响应；对齐 UI `ConnectionSnapshot`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub ok: bool,
    pub state: ConnectionState,
    pub server: Option<ConnectedServerSummary>,
    pub connected_since: Option<f64>,
    pub system_proxy_active: bool,
    /// 我们尝试切换系统代理后回查发现 SOCKS 没真正生效（被外部代理 daemon
    /// 覆盖）。`active=true && overridden=true` 是触发 UI "代理被劫持" 横幅
    /// 的唯一信号；其它组合都视作正常。默认 false 兼容旧 client 反序列化。
    #[serde(default)]
    pub system_proxy_overridden: bool,
    pub heartbeat: Option<ConnectionHeartbeat>,
    pub last_error: Option<String>,
}

/// 5 步连接进度事件 payload。对齐 UI `ConnectProgressPayload`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectProgress {
    pub step: u8,
    pub total: u8,
    pub key: &'static str,
    pub label: &'static str,
    pub status: ConnectStepStatus,
    pub detail: String,
    pub server_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectStepStatus {
    Running,
    Ok,
    Failed,
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
    fn discovered_server_serde_roundtrip_keeps_snake_case_fields() {
        let ds = sample_discovered();
        let json = serde_json::to_string(&ds).unwrap();
        assert!(json.contains("\"server_id\""));
        assert!(json.contains("\"last_seen_at\""));
        assert!(json.contains("\"source\":\"mdns\""));
        let back: DiscoveredServer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ds);
    }

    #[test]
    fn pac_url_helper_concatenates_host_port_path() {
        assert_eq!(
            sample_discovered().pac_url(),
            "http://192.168.1.10:8080/proxy.pac"
        );
    }

    #[test]
    fn make_server_id_uses_name_at_host_colon_port_format() {
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
