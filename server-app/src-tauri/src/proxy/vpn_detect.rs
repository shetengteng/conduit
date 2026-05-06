//! VPN 接口检测——给 Network 健康检查面板提供"是否走 VPN"的判定。
//!
//! 设计要点：
//! - 不调用子进程：用 `local_ip_address::list_afinet_netifas()` 列出全部
//!   已分配 IPv4 的接口，名字以 `utun` / `ppp` / `tun` / `tap` / `gpd`
//!   开头视为 VPN 隧道接口。
//! - macOS 上 `utun0`/`utun1` 等通常是系统 NetworkExtension（iCloud Relay
//!   等）创建的占位接口，常常没有 IPv4；只有真正接通 VPN 的那一条会被
//!   分到 RFC1918 / CGNAT 地址。所以仅匹配"有 IPv4 的"接口就足够准确。
//! - 检测属于 advisory：UI 用它来切换"VPN 异常"徽标，HTTP 路由层面
//!   不依赖它。前端 `vpn_state_changed` SSE event 也由这里推。
//!
//! 周期 5s 触发一次，状态翻转才通过 `ProxyCore::update_vpn` 推事件，避免抖动。

use std::time::Duration;

use log::{debug, info, warn};
use tokio_util::sync::CancellationToken;

use super::core::ProxyCore;

/// 视为 VPN 隧道的接口名前缀（macOS 上 `utun*` / `ppp*` 是常见 VPN/Relay 隧道）。
const VPN_PREFIXES: &[&str] = &["utun", "ppp", "tun", "tap", "gpd"];

/// 周期检测间隔，UI 顶部 VPN 徽标据此刷新。
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// 探测当前主机是否有"接通的" VPN 接口。
///
/// 返回 `(vpn_on, iface_name)`：
/// - `vpn_on`：是否有任意 utun/ppp/tun/tap/gpd 接口拿到非 loopback 的 IPv4。
/// - `iface_name`：第一条匹配的接口名（多条时取首条）；用于 UI 显示。
pub fn detect() -> (bool, Option<String>) {
    let ifaces = match local_ip_address::list_afinet_netifas() {
        Ok(v) => v,
        Err(e) => {
            warn!("[vpn] list_afinet_netifas 失败: {e}");
            return (false, None);
        }
    };
    for (name, ip) in &ifaces {
        let is_vpn_name = VPN_PREFIXES.iter().any(|p| name.starts_with(p));
        if is_vpn_name && ip.is_ipv4() && !ip.is_loopback() {
            debug!("[vpn] 发现隧道接口: {name} = {ip}");
            return (true, Some(name.clone()));
        }
    }
    (false, None)
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
    let (initial_on, initial_iface) = detect();
    tokio::select! {
        _ = cancel.cancelled() => {
            debug!("[vpn] 启动期被取消，跳过初始 update");
            return;
        }
        _ = core.update_vpn(initial_on, initial_iface.clone()) => {}
    }
    info!(
        "[vpn] 初始状态: vpn={} iface={}",
        if initial_on { "on" } else { "off" },
        initial_iface.as_deref().unwrap_or("-")
    );

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                debug!("[vpn] 收到取消信号，detector 退出");
                return;
            }
            _ = tokio::time::sleep(POLL_INTERVAL) => {
                let (on, iface) = detect();
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = core.update_vpn(on, iface) => {}
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
        let (_on, _iface) = detect();
    }

    #[test]
    fn vpn_prefixes_cover_known_tunnel_kinds() {
        for p in ["utun", "ppp", "tun", "tap", "gpd"] {
            assert!(VPN_PREFIXES.contains(&p));
        }
    }
}
