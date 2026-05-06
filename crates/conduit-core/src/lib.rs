//! `conduit-core` —— server-app 与 client-app 共享的协议、类型与基础组件。
//!
//! ## 模块路线图
//!
//! - [`error`] —— 通用错误模型 [`ConduitError`] / [`ConduitResult`]（已实装）
//! - [`pac`] —— PAC 引擎（regex 解析 5 段 numbered section，已实装）
//! - [`events`] —— 基于 `tokio::sync::broadcast` 的 EventBus（已实装）
//! - [`relay`] —— 双向流量转发（已实装）
//! - [`mdns`] —— mDNS service-type / TXT 字段约定（已实装）
//! - [`types`] —— 双端共享 wire types（serde；specta 绑定在 S1.5 接入）
//!
//! 详见 `design/2026-05-06-2-Conduit-Rust-重写设计文档.md`。

pub mod error;
pub mod events;
pub mod healthz;
pub mod mdns;
pub mod pac;
pub mod ports;
pub mod relay;
pub mod types;

pub use error::{ConduitError, ConduitResult};
pub use events::EventBus;
pub use healthz::{wait_until_ready, HealthzError};
pub use pac::{PacDecision, PacRules};
pub use ports::pick_unused_ports;
pub use relay::{bidirectional_relay, ProgressSink, CHUNK};
pub use types::{
    ConnectStepStatus, ConnectProgress, ConnectedServerSummary, ConnectionHeartbeat,
    ConnectionInfo, ConnectionSnapshot, ConnectionState, DiscoveredServer, HealthCheckResult,
    HealthSummary, HeartbeatState, HeartbeatTone, ProbeResult, RouteDirection, RouteEntry,
    ServerSource,
};

/// 项目唯一 PAC 模板（embed 自 `crates/conduit-core/assets/proxy.pac`）。
///
/// `__PROXY_HOST__` / `__PROXY_PORT__` 占位符由 server-app HTTP 处理器在
/// 响应 `GET /proxy.pac` / `GET /wpad.dat` 时替换为本机 LAN IP + HTTP 监听端口。
/// PAC 解析（`PacRules::parse`）允许带占位符直接编译，因为占位符不出现在正则可达分支。
pub const PAC_TEMPLATE: &str = include_str!("../assets/proxy.pac");
