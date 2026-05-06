//! `proxy` 模块 —— W2 Sprint 2 引入的内嵌 server-side 代理实现。
//!
//! 子模块路线图：
//! - [`config`] —— `ProxyConfig`（已实装 S2.1）
//! - [`core`] —— `ProxyCore` 总入口、ServerEvent、ServerStatus（骨架 S2.1，
//!   已挂 HTTP listener S2.2）
//! - [`http`] —— hyper-free forward proxy + PAC serving + heartbeat
//!   （已实装 S2.2 第一轮）
//! - [`session`] —— ConnectionInfo / PassiveClient 注册表 + ProgressSink
//!   （已实装 S2.2）
//! - [`socks5`] —— RFC1928 手写 SOCKS5 server（已实装 S2.3）
//! - [`mdns`] —— `mdns-sd` 服务广播（已实装 S2.4）
//! - `outbound` —— DIRECT-first 路由 + system-proxy 控制（待 S2.5 实装）
//!
//! 设计参考：`design/2026-05-06-2-Conduit-Rust-重写设计文档.md` §5.1 ~ §5.6。

pub mod config;
pub mod control_api;
pub mod core;
pub mod http;
pub mod mdns;
pub mod session;
pub mod socks5;

pub use config::ProxyConfig;
pub use core::ProxyCore;
