//! `MacSystemProxy` —— macOS 系统 SOCKS 代理切换入口。
//!
//! ## 写路径走 SystemConfiguration framework(2026-05-07 W7 升级到方案 B)
//!
//! 历史:
//! - V1:`networksetup -setsocksfirewallproxy*` CLI。macOS 13+ 普通用户
//!   常 exit 14("Operation not permitted")。
//! - V2:V1 失败时 fallback `osascript ... with administrator privileges`。
//!   osascript token 不跨进程缓存 → **每次 connect 都弹密码框**,严重退化。
//! - V3(P 方案):去掉 V2 fallback,失败仅 UI warning,与 Python 对齐。
//!   **0 弹框**但浏览器要用户手动配 SOCKS5,体验不够"自动"。
//! - V4(B 方案,当前):走 [`super::system_proxy_sc`] 直接调用
//!   `SCPreferencesCreateWithAuthorization` + 进程级 AuthorizationRef
//!   缓存。**首次弹 1 次原生密码框,之后整个进程内复用 token,不再弹框**。
//!
//! ## 当前流程
//!
//! 1. **list / read 走 networksetup**(`list_services` / `read_socks_state`)
//!    —— 这两个是只读命令,不需要 admin,且 networksetup 输出格式稳定易解析,
//!    比 SC API 走 SCPreferencesCreate(无授权版)简单。
//!
//! 2. **目标 service 过滤**(`pick_target_services`):
//!    黑名单排除 Thunderbolt Bridge / iPhone USB / Bluetooth PAN/DUN。
//!
//! 3. **已设短路**:全部目标 service 都已指向 `host:port + enabled` → noop。
//!    避免无谓重写,也避免在已经设好的情况下还触发 SC commit。
//!
//! 4. **写走 SC framework**:`enable_via_sc` / `disable_via_sc`,首次弹 1 次密码。
//!
//! 5. **失败 → 返 Err**,上层 `core.rs::step_switch_endpoint` 发
//!    `system_proxy_warning` event + UI banner 提示手动配置,**不阻断连接**。
//!
//! ## 局限
//!
//! - **每次进程重启会再弹一次密码**:AuthorizationRef 不能跨进程持久化(macOS
//!   安全约束)。彻底消除需要 SMJobBless privileged helper + Apple Developer
//!   ID 签名,目前 ad-hoc 签名做不到。
//!
//! 注:Linux/Windows 平台没有这个能力,`enable` / `disable` 直接 noop。

#[cfg(target_os = "macos")]
use std::process::Command;

const NETWORKSETUP: &str = "/usr/sbin/networksetup";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocksProxyState {
    pub enabled: bool,
    pub server: String,
    pub port: u16,
}

impl SocksProxyState {
    pub fn points_to(&self, host: &str, port: u16) -> bool {
        self.enabled && self.server == host && self.port == port
    }
}

pub struct MacSystemProxy;

impl MacSystemProxy {
    pub fn is_supported() -> bool {
        cfg!(target_os = "macos")
    }

    /// 启动期清理：上次进程崩溃留下的指向我们的 SOCKS 代理。
    /// 返回 true 表示真的清理了。
    pub fn cleanup_if_pointing_to_us(&self, host: &str, port: u16) -> bool {
        #[cfg(target_os = "macos")]
        {
            for svc in list_services().unwrap_or_default() {
                if let Ok(state) = read_socks_state(&svc) {
                    if state.points_to(host, port) {
                        let _ = disable_socks(&svc);
                        log::info!("[system_proxy] cleanup stale proxy on '{svc}'");
                        return true;
                    }
                }
            }
        }
        let _ = (host, port);
        false
    }

    /// 把"目标网卡"的 SOCKS 代理改成指向 `host:port` 并开启。
    ///
    /// 流程(详见模块 doc):
    /// 1. `pick_target_services()` 过滤掉 Thunderbolt Bridge / iPhone USB 等虚拟 service
    /// 2. 全部目标 service 都已经指向 `host:port + enabled` → noop(避免重复写)
    /// 3. 走 SC framework `enable_via_sc`(首次弹 1 次密码,之后进程内 0 次)
    /// 4. 失败 → 返 Err,**不再 fallback osascript**
    #[cfg(target_os = "macos")]
    pub fn enable(&self, host: &str, port: u16) -> Result<(), String> {
        let all = list_services()?;
        if all.is_empty() {
            return Err("no network services visible".into());
        }
        let svcs = pick_target_services(&all);
        if svcs.is_empty() {
            return Err(format!(
                "no usable network service after filtering (raw={all:?})"
            ));
        }

        // 已设短路:全部目标 service 都已指向我们 → 跳过 SC commit + AuthorizationCreate。
        // get 是只读命令,不需要 admin。失败的 service 当作"未设"处理(保守)。
        if svcs.iter().all(|svc| {
            read_socks_state(svc)
                .map(|s| s.points_to(host, port))
                .unwrap_or(false)
        }) {
            log::info!(
                "[system_proxy] all {} target services already → {host}:{port}, skip set",
                svcs.len()
            );
            return Ok(());
        }

        let updated = super::system_proxy_sc::enable_via_sc(&svcs, host, port).map_err(|e| {
            format!("system_proxy_sc::enable_via_sc failed (no fallback by design): {e}")
        })?;
        if updated == 0 {
            return Err(format!(
                "system_proxy_sc updated 0 services (target list = {svcs:?})"
            ));
        }
        log::info!(
            "[system_proxy] enabled SOCKS via SC on {updated}/{} services → {host}:{port}",
            svcs.len()
        );
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn enable(&self, _host: &str, _port: u16) -> Result<(), String> {
        Err("system_proxy not supported on this platform".into())
    }

    /// 关闭"目标网卡"的 SOCKS 代理。
    ///
    /// 与 enable 对称:只对 `pick_target_services` 列表操作,避免对从未被 enable 的虚拟
    /// service(Thunderbolt Bridge / iPhone USB)发 disable 命令,降低无谓的失败。
    ///
    /// 走 SC framework + 进程缓存的 AuthorizationRef:**如果 enable 已经弹过密码,
    /// disable 不再弹**。失败仅 log warning 不返 Err,因为 disconnect 时让用户再
    /// 次输密码体验很差,且代理"开着"比"关不掉"更易恢复(用户可以在系统设置里手动关)。
    #[cfg(target_os = "macos")]
    pub fn disable(&self) -> Result<(), String> {
        let all = list_services()?;
        if all.is_empty() {
            return Ok(());
        }
        let svcs = pick_target_services(&all);
        if svcs.is_empty() {
            return Ok(());
        }

        match super::system_proxy_sc::disable_via_sc(&svcs) {
            Ok(updated) => {
                log::info!(
                    "[system_proxy] disabled SOCKS via SC on {updated}/{} services",
                    svcs.len()
                );
            }
            Err(e) => {
                log::warn!(
                    "[system_proxy] disable_via_sc failed on {} services (no fallback \
                    by design); user may need to turn off SOCKS manually. err={e}",
                    svcs.len()
                );
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn disable(&self) -> Result<(), String> {
        Err("system_proxy not supported on this platform".into())
    }
}

/// 已知"几乎不会用作上网"的虚拟 service —— 在它们上 set socksfirewallproxy 通常会
/// 失败(且即使成功也没用),纯粹拖累成功率,在"无 osascript 兜底"策略下会让整个
/// `enable()` 直接返 Err,触发 UI 降级提示。
///
/// 关键字匹配(case-insensitive),覆盖常见命名:
/// - `Thunderbolt Bridge`(雷电桥,虚拟二层桥接)
/// - `iPhone USB`(iPhone 网络共享)
/// - `Bluetooth PAN` / `Bluetooth DUN`(蓝牙网络共享)
const VIRTUAL_SERVICE_BLOCKLIST: &[&str] = &[
    "thunderbolt bridge",
    "iphone usb",
    "bluetooth pan",
    "bluetooth dun",
];

/// 从 `list_services()` 全集挑出"用户实际用来上网"的 service。
///
/// 策略:
/// 1. 排除 [`VIRTUAL_SERVICE_BLOCKLIST`] 中的虚拟 service
/// 2. 剩下的全部保留(笔记本可能 Wi-Fi + USB 网卡同时插)
/// 3. 如果过滤后空了,返回原列表(保证不会因为命名特殊而完全无法工作)
///
/// 与 Python 版本(`active_service`,只挑一个)略有不同:Python 只对一个 service set,
/// 用户切换网卡时代理就跟丢了。这里保留所有非虚拟 service,**牺牲一点性能换更可靠的体感**:
/// 比如 Wi-Fi 断开切到以太网时不需要重连 server。
#[cfg(target_os = "macos")]
fn pick_target_services(all: &[String]) -> Vec<String> {
    let filtered: Vec<String> = all
        .iter()
        .filter(|svc| {
            let lower = svc.to_ascii_lowercase();
            !VIRTUAL_SERVICE_BLOCKLIST
                .iter()
                .any(|kw| lower.contains(kw))
        })
        .cloned()
        .collect();
    if filtered.is_empty() {
        // 极端兜底:全部 service 都被黑名单命中 → 退回原列表,让 networksetup 至少有
        // 机会 set 成功;失败也比直接"无服务可用"强。
        all.to_vec()
    } else {
        filtered
    }
}

#[cfg(target_os = "macos")]
fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(NETWORKSETUP)
        .args(args)
        .output()
        .map_err(|e| format!("spawn {NETWORKSETUP} failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "{NETWORKSETUP} {args:?} → exit {} stderr={}",
            out.status,
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(target_os = "macos")]
fn list_services() -> Result<Vec<String>, String> {
    let out = run(&["-listallnetworkservices"])?;
    let mut svcs: Vec<String> = Vec::new();
    for (i, line) in out.lines().enumerate() {
        if i == 0 {
            // 第一行是说明：An asterisk (*) denotes that a network service is disabled.
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('*') {
            continue;
        }
        svcs.push(trimmed.to_string());
    }
    Ok(svcs)
}

#[cfg(target_os = "macos")]
fn read_socks_state(svc: &str) -> Result<SocksProxyState, String> {
    let out = run(&["-getsocksfirewallproxy", svc])?;
    let mut enabled = false;
    let mut server = String::new();
    let mut port: u16 = 0;
    for line in out.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("Enabled:") {
            enabled = matches!(v.trim(), "Yes" | "yes" | "YES" | "true" | "True");
        } else if let Some(v) = line.strip_prefix("Server:") {
            server = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Port:") {
            port = v.trim().parse().unwrap_or(0);
        }
    }
    Ok(SocksProxyState {
        enabled,
        server,
        port,
    })
}

/// 启动期 `cleanup_if_pointing_to_us` 用来关闭上次进程崩溃残留的 SOCKS 代理。
/// 这里走 networksetup CLI(无提权,失败被 swallow),不走 SC framework —— 否则
/// 每次启动 client 都会弹一次密码框,体验远差于"残留代理偶尔需要用户手动关"。
#[cfg(target_os = "macos")]
fn disable_socks(svc: &str) -> Result<(), String> {
    run(&["-setsocksfirewallproxystate", svc, "off"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_to_only_when_enabled_and_matching() {
        let state = SocksProxyState {
            enabled: true,
            server: "127.0.0.1".into(),
            port: 7890,
        };
        assert!(state.points_to("127.0.0.1", 7890));
        assert!(!state.points_to("127.0.0.1", 1080));
        assert!(!state.points_to("10.0.0.1", 7890));

        let off = SocksProxyState {
            enabled: false,
            server: "127.0.0.1".into(),
            port: 7890,
        };
        assert!(!off.points_to("127.0.0.1", 7890));
    }

    #[test]
    fn is_supported_matches_target() {
        assert_eq!(MacSystemProxy::is_supported(), cfg!(target_os = "macos"));
    }

    // pick_target_services 仅 macOS 编译,所以测试也加 cfg。
    #[cfg(target_os = "macos")]
    mod service_filter {
        use super::super::pick_target_services;

        #[test]
        fn pick_target_services_drops_known_virtual_services() {
            // 真机 networksetup -listallnetworkservices 返回的混合列表
            let raw = vec![
                "AX88179A".to_string(),
                "Belkin USB-C LAN".to_string(),
                "USB 10/100 LAN".to_string(),
                "Thunderbolt Bridge".to_string(),
                "Wi-Fi".to_string(),
                "iPhone USB".to_string(),
            ];
            let picked = pick_target_services(&raw);
            assert!(picked.contains(&"Wi-Fi".to_string()));
            assert!(picked.contains(&"AX88179A".to_string()));
            assert!(picked.contains(&"Belkin USB-C LAN".to_string()));
            assert!(picked.contains(&"USB 10/100 LAN".to_string()));
            assert!(!picked.contains(&"Thunderbolt Bridge".to_string()));
            assert!(!picked.contains(&"iPhone USB".to_string()));
        }

        #[test]
        fn pick_target_services_is_case_insensitive() {
            let raw = vec![
                "THUNDERBOLT BRIDGE".to_string(),
                "iphone usb".to_string(),
                "Wi-Fi".to_string(),
                "Bluetooth PAN".to_string(),
                "Bluetooth DUN".to_string(),
            ];
            let picked = pick_target_services(&raw);
            assert_eq!(picked, vec!["Wi-Fi".to_string()]);
        }

        #[test]
        fn pick_target_services_falls_back_to_full_list_when_all_blocked() {
            // 极端:全部都被黑名单命中 → 退回原列表,而非完全无法工作
            let raw = vec![
                "Thunderbolt Bridge".to_string(),
                "iPhone USB".to_string(),
            ];
            let picked = pick_target_services(&raw);
            assert_eq!(picked, raw);
        }

        #[test]
        fn pick_target_services_keeps_unrelated_naming() {
            // 用户机器上可能有非标准命名的网卡(USB-C 拓展坞自带 LAN 模块、虚拟网卡等)
            // 只要不在黑名单关键字里就保留
            let raw = vec![
                "Some Random USB Adapter".to_string(),
                "Wi-Fi".to_string(),
            ];
            let picked = pick_target_services(&raw);
            assert_eq!(picked.len(), 2);
        }
    }
}
