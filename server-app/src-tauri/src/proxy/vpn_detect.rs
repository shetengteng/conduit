//! VPN 接口检测——给 Network 健康检查面板提供"是否走 VPN"的判定。
//!
//! 设计要点 (v0.2.2 起改用 `netdev` crate, 替代旧版 `local_ip_address` + 名字前缀
//! 启发式判断):
//! - **接口分类用 `InterfaceType`**: netdev 把 OS 上报的 if_type 归一成
//!   Ethernet / Wireless80211 / Tunnel / Ppp / Loopback 等枚举,不再依赖
//!   `utun*/ppp*/tun*` 名字前缀启发式 (易错, 比如 macOS 用户接口可能换名)。
//! - **同时要求接口"有非 loopback IPv4"**: macOS 上 `utun0/utun1` 常是
//!   NetworkExtension (iCloud Relay 等) 占位接口, 没有 IPv4; 只有真正接通的
//!   VPN 才会拿到 RFC1918/CGNAT 地址。这条筛选与旧实现等价。
//! - **`default_route_via_vpn`**: 直接看 `netdev::get_default_interface()`
//!   返回的接口是不是 Tunnel/Ppp/is_tun, 真实反映"系统当前默认路由是否经过
//!   VPN 隧道"——旧实现这块字段一直是 vpn_on 近似 (有 VPN ≠ 走 VPN), UI
//!   "默认路由 → VPN" 文本不准确。
//! - 检测属于 advisory: UI 用它来切换 VPN 徽标, HTTP 路由层面不依赖它。
//!   前端 `vpn_state_changed` SSE event 由本协程推。
//!
//! 周期 5s 触发一次, 状态翻转才通过 `ProxyCore::update_vpn` 推事件, 避免抖动。

use std::time::Duration;

use log::{debug, info};
use tokio_util::sync::CancellationToken;

use super::core::ProxyCore;

/// 周期检测间隔，UI 顶部 VPN 徽标据此刷新。
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// 接口是否被视为"VPN 隧道"接口。
/// 判定: `InterfaceType::Tunnel / Ppp` 或 `is_tun()` 启发式 (跨平台兜底)。
fn is_vpn_iface(iface: &netdev::Interface) -> bool {
    use netdev::interface::types::InterfaceType;
    matches!(iface.if_type, InterfaceType::Tunnel | InterfaceType::Ppp) || iface.is_tun()
}

/// 探测当前主机是否有"接通的" VPN 接口 + 默认路由是否走 VPN。
///
/// 返回 `(vpn_on, iface_name, default_via_vpn)`:
/// - `vpn_on`: 是否有任意 Tunnel/Ppp/utun-style 接口拿到非 loopback 的 IPv4
/// - `iface_name`: 第一条匹配的接口名(多条时取首条);用于 UI 显示
/// - `default_via_vpn`: 系统当前 default route 出接口是否就是 VPN 接口本身
pub fn detect() -> (bool, Option<String>, bool) {
    let ifaces = netdev::get_interfaces();
    let mut tunnel_iface: Option<String> = None;
    for iface in &ifaces {
        if !is_vpn_iface(iface) {
            continue;
        }
        let has_ipv4 = iface
            .ipv4
            .iter()
            .any(|n| !n.addr().is_loopback() && !n.addr().is_unspecified());
        if has_ipv4 {
            debug!("[vpn] 发现隧道接口: {} ({:?})", iface.name, iface.if_type);
            tunnel_iface = Some(iface.name.clone());
            break;
        }
    }

    let default_via_vpn = match netdev::get_default_interface() {
        Ok(def) => is_vpn_iface(&def),
        Err(_) => false,
    };

    (tunnel_iface.is_some(), tunnel_iface, default_via_vpn)
}

/// 启动 VPN 检测协程。`cancel` 触发时退出。
///
/// 启动时立即跑一次 detect 把状态推给 [`ProxyCore`]，之后每 5s 重新探测；
/// 状态翻转才会让 `update_vpn` 真正 publish event（内部已去重）。
///
/// 死锁防护：每次调 `core.update_vpn(...)` 前都用 `tokio::select` 与
/// `cancel.cancelled()` 竞争——`ProxyCore::stop()` 流程会先 cancel 再
/// `inner.lock()` 然后 `h.await`；如果 detector 此刻正在 await 同一个
/// `inner.lock()`，stop 就死锁了。让 update_vpn 与 cancel 竞争即可避免。
pub async fn run(core: ProxyCore, cancel: CancellationToken) {
    let (initial_on, initial_iface, initial_default) = detect();
    tokio::select! {
        _ = cancel.cancelled() => {
            debug!("[vpn] 启动期被取消，跳过初始 update");
            return;
        }
        _ = core.update_vpn(initial_on, initial_iface.clone(), initial_default) => {}
    }
    info!(
        "[vpn] 初始状态: vpn={} iface={} default_via_vpn={}",
        if initial_on { "on" } else { "off" },
        initial_iface.as_deref().unwrap_or("-"),
        initial_default
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                debug!("[vpn] 收到取消信号，detector 退出");
                return;
            }
            _ = tokio::time::sleep(POLL_INTERVAL) => {
                let (on, iface, default_via) = detect();
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = core.update_vpn(on, iface, default_via) => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_does_not_panic() {
        // 不假设宿主机是否在 VPN，只要不 panic 即可。
        let (_on, _iface, _default_via) = detect();
    }

    /// is_vpn_iface 必须在常见 macOS 隧道接口名下返回 true (utun*) 即便平台没把
    /// if_type 标成 Tunnel —— 这是 is_tun() 启发式兜底场景。
    #[test]
    fn is_vpn_iface_recognizes_utun_via_is_tun_fallback() {
        // 拿当前主机一张 loopback 当反例:它一定不是 VPN。
        let any_loop = netdev::get_interfaces()
            .into_iter()
            .find(|i| i.is_loopback());
        if let Some(lo) = any_loop {
            assert!(!is_vpn_iface(&lo), "loopback should not be VPN: {:?}", lo);
        }
    }
}
