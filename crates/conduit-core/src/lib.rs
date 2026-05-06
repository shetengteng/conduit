//! `conduit-core` —— server-app 与 client-app 共享的协议、类型与基础组件。
//!
//! ## 模块路线图
//!
//! - [`error`] —— 通用错误模型 [`ConduitError`] / [`ConduitResult`]（已实装）
//! - [`pac`] —— PAC 引擎（regex 平移 Python 端 `pac_engine.py`，已实装）
//! - [`events`] —— 基于 `tokio::sync::broadcast` 的 EventBus（已实装）
//! - [`relay`] —— 双向流量转发（已实装）
//! - [`mdns`] —— mDNS service-type / TXT 字段约定（已实装）
//! - [`types`] —— 双端共享 wire types（serde；specta 绑定在 S1.5 接入）
//!
//! 详见 `design/2026-05-06-2-Conduit-Rust-重写设计文档.md`。

pub mod error;
pub mod events;
pub mod mdns;
pub mod pac;
pub mod relay;
pub mod types;

pub use error::{ConduitError, ConduitResult};
pub use events::EventBus;
pub use pac::{PacDecision, PacRules};
pub use relay::{bidirectional_relay, ProgressSink, CHUNK};
pub use types::{
    ConnectionInfo, DiscoveredServer, HealthCheckResult, HealthSummary, ProbeResult, ServerSource,
};
