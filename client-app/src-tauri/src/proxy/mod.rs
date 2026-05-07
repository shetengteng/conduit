//! `proxy` —— client-app 内嵌的智能本地代理（W3 Sprint 3 起进程内承载，无 sidecar）。
//!
//! 模块组成：
//! - [`config`] —— `ClientConfig` 启动参数
//! - [`core`] —— `ClientCore` 生命周期 + EventBus + 5 步连接状态机
//! - [`discoverer`] —— mDNS 发现 + `known-servers.json` 持久化
//! - [`route_cache`] —— 路由缓存（`dashmap` + TTL）
//! - [`route_resolver`] —— PAC + cache + probe + private-IP 决策树
//! - [`local_proxy`] —— 本地 SOCKS5 listener + relay
//! - [`system_proxy`] —— macOS 系统 SOCKS 代理切换入口(走 SystemConfiguration framework)
//! - [`system_proxy_sc`] —— SC framework + AuthorizationRef session 缓存,首次弹 1 次密码
//! - [`connectivity`] —— 一次性 probe + 心跳
//! - [`traffic_meter`] —— 流量统计
//! - [`control_api`] —— 兼容 UI 现有 REST/SSE 的 127.0.0.1 控制接口

// 迁移阶段：以下模块包含一组预留的 public API（如 RouteResolver::set_global_mode、
// RouteCache::len、TrafficMeter::reset 等），将在后续 S3.x 迭代逐步接到 control_api /
// 心跳降级 / 调试面板上。此处统一允许 dead_code，避免污染构建输出。
#![allow(dead_code)]

pub mod config;
pub mod connectivity;
pub mod control_api;
pub mod core;
pub mod discoverer;
pub mod local_proxy;
pub mod route_cache;
pub mod route_resolver;
pub mod system_proxy;
#[cfg(target_os = "macos")]
pub mod system_proxy_sc;
pub mod traffic_meter;

pub use config::ClientConfig;
pub use core::ClientCore;
