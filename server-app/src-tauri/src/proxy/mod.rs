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
//! - [`vpn_detect`] —— utun/ppp/tun 接口周期检测，驱动 UI VPN 徽标
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
pub mod traffic_emitter;
pub mod vpn_detect;

pub use config::ProxyConfig;
pub use core::ProxyCore;

/// 推断"对外可达"的 host —— PAC URL / 接入信息 / PAC body 内 PROXY_HOST 共用。
///
/// 优先级（与 `mdns::run` 中的 `host_ip` 推断一致，确保 mDNS 广播 IP 与
/// 用户在 UI 上看到的接入信息地址同源）：
///
/// 1. `--pac-host` (`cfg.pac_advertised_host`) 显式指定
/// 2. 非 `0.0.0.0` 的 `cfg.bind`（用户绑死了具体网卡）
/// 3. `mdns::detect_lan_ip()` 自动探测的 LAN IP
/// 4. 最终兜底 `127.0.0.1`
///
/// 之前 `chosen_host` 在 1/2 都不满足时直接回退 `127.0.0.1`，导致默认
/// `bind=0.0.0.0` 时 UI 接入卡只能复制 loopback 给同事 —— 解决该问题。
pub fn effective_advertised_host(cfg: &ProxyConfig) -> String {
    if !cfg.pac_advertised_host.is_empty() {
        return cfg.pac_advertised_host.clone();
    }
    if !cfg.bind.is_empty() && cfg.bind != "0.0.0.0" {
        return cfg.bind.clone();
    }
    mdns::detect_lan_ip()
}
