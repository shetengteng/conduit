//! `MacSystemProxy` —— macOS `networksetup` 系统 SOCKS 代理 wrapper。
//!
//! 调用 4 个 networksetup 命令：
//! - `-listallnetworkservices`            列出网络服务（只读）
//! - `-getsocksfirewallproxy <svc>`       读当前 SOCKS 配置（只读）
//! - `-setsocksfirewallproxy <svc> ...`   设置 SOCKS 服务器（**需要 admin**）
//! - `-setsocksfirewallproxystate <svc>`  开/关（**需要 admin**）
//!
//! ## 提权策略（2026-05-07 W6 backlog #7,W6 backlog #11 收敛）
//!
//! Tauri sandbox + macOS 13+ 下普通用户跑 `networksetup -setsocksfirewallproxy*`
//! 经常 exit 14（"Operation not permitted"）。最早的策略是任意 service 失败 → 整批
//! 走 `osascript ... with administrator privileges`,但 osascript 提权 token 不跨进程
//! 缓存,导致 **每次 connect 都会弹密码框**,用户体验严重退化。
//!
//! 现在采用 **两层降级 + 已设短路** 策略,与 Python 版本(`client-app/core/system_proxy.py`)
//! 行为对齐:
//!
//! 1. **目标 service 过滤**(`pick_target_services`):
//!    优先 Wi-Fi / Ethernet 类常用网卡,黑名单排除 Thunderbolt Bridge / iPhone USB /
//!    Bluetooth PAN / Bluetooth DUN —— 这些虚拟 service 上 `setsocksfirewallproxy`
//!    通常会失败,且用户不会用它们上网,纯粹拖累成功率。
//!
//! 2. **已设短路**(enable 入口):
//!    如果**全部目标 service 都已经指向 `host:port` + enabled**,直接 noop 返回。
//!    "断开 → 重新 connect 同一个 server"的场景下不会再触发任何写操作 → 永不弹密码。
//!
//! 3. **无提权批量 set + enable**:对剩余目标 service 跑 networksetup,任何一项失败 →
//! 4. **fallback 到 osascript admin 一次性提权**(原有兜底),用户会看到 macOS 原生密码框。
//!
//! 这样:
//! - **绝大多数循环 connect/disconnect 不弹密码**(短路命中)
//! - 首次或换 server 时仍要弹一次,但目标 service 已经被精简,无提权阶段成功率更高
//! - 极端 sandbox 限制下仍能 fallback 提权,功能不会"默默失效"
//!
//! 注:Linux/Windows 平台没有这个能力,`enable` / `disable` 直接 noop。

#[cfg(target_os = "macos")]
use std::process::Command;

const NETWORKSETUP: &str = "/usr/sbin/networksetup";
#[cfg(target_os = "macos")]
const OSASCRIPT: &str = "/usr/bin/osascript";

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
    /// 2. 全部目标 service 都已经指向 `host:port + enabled` → noop(避免循环 connect 弹密码)
    /// 3. 无提权批量 `set` + `enable`,任何一项失败 →
    /// 4. fallback 到 `osascript ... with administrator privileges` 一次性提权
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

        // 已设短路:全部目标 service 都已指向我们 → 跳过整个写流程,免提权弹框。
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

        let mut needs_elevation = false;
        let mut ok_count = 0usize;
        for svc in &svcs {
            match try_set_and_enable_unprivileged(svc, host, port) {
                Ok(()) => ok_count += 1,
                Err(e) => {
                    log::warn!("[system_proxy] unprivileged set/enable on '{svc}' failed: {e}");
                    needs_elevation = true;
                    break; // 一旦有 service 失败就整批走 osascript 提权,避免提权后再单独补
                }
            }
        }

        if needs_elevation {
            log::info!(
                "[system_proxy] falling back to osascript admin elevation for {} services",
                svcs.len()
            );
            run_setup_with_admin(&svcs, host, port)?;
            log::info!(
                "[system_proxy] enabled SOCKS on {} services via admin elevation → {host}:{port}",
                svcs.len()
            );
            return Ok(());
        }

        log::info!(
            "[system_proxy] enabled SOCKS on {ok_count}/{} services → {host}:{port}",
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
    /// service(Thunderbolt Bridge / iPhone USB)发 disable 命令,降低无谓的失败 →
    /// 减少 osascript 触发概率。
    ///
    /// 单 service 部分失败不算 fatal(设计取舍:disable 的可恢复性比 enable 重要)。
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

        let mut ok_count = 0usize;
        for svc in &svcs {
            if disable_socks(svc).is_ok() {
                ok_count += 1;
            }
        }
        if ok_count == 0 {
            log::info!("[system_proxy] disable all-failed unprivileged → trying osascript admin");
            run_disable_with_admin(&svcs)?;
        }
        log::info!("[system_proxy] disabled SOCKS on {} services", svcs.len());
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn disable(&self) -> Result<(), String> {
        Err("system_proxy not supported on this platform".into())
    }
}

#[cfg(target_os = "macos")]
fn try_set_and_enable_unprivileged(svc: &str, host: &str, port: u16) -> Result<(), String> {
    set_socks(svc, host, port)?;
    enable_socks(svc)?;
    Ok(())
}

/// 已知"几乎不会用作上网"的虚拟 service —— 在它们上 set socksfirewallproxy 通常会
/// 失败(且即使成功也没用),纯粹拖累成功率/触发 osascript 提权。
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
        // 极端兜底:全部 service 都被黑名单命中 → 退回原列表,让 osascript 兜底也比直接 fail 强
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

#[cfg(target_os = "macos")]
fn set_socks(svc: &str, host: &str, port: u16) -> Result<(), String> {
    run(&[
        "-setsocksfirewallproxy",
        svc,
        host,
        &port.to_string(),
    ])
    .map(|_| ())
}

#[cfg(target_os = "macos")]
fn enable_socks(svc: &str) -> Result<(), String> {
    run(&["-setsocksfirewallproxystate", svc, "on"]).map(|_| ())
}

#[cfg(target_os = "macos")]
fn disable_socks(svc: &str) -> Result<(), String> {
    run(&["-setsocksfirewallproxystate", svc, "off"]).map(|_| ())
}

/// 把字符串包成单引号 sh literal,内部 `'` 替换成 `'"'"'` (single→double→single 三段)。
///
/// 用于 `do shell script` 内嵌的 sh 脚本拼装,确保 service 名（可能含空格 / 引号 /
/// 反斜杠）不会破坏命令结构。
#[cfg(target_os = "macos")]
fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\"'\"'");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// 把 sh 脚本包进 AppleScript 字符串字面量。AppleScript 的双引号字符串需要转义
/// `\` 与 `"`,其它字符（含空格 / 制表符 / 中文）都是合法字符。
#[cfg(target_os = "macos")]
fn applescript_string_literal(sh: &str) -> String {
    let escaped = sh.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// 用 osascript 走 admin 提权一次性执行整批 set+enable。失败时返回 stderr 摘要。
///
/// 用户取消密码弹框 → osascript 退出码 1 + stderr `User canceled.`,转换成
/// `Err("admin password prompt cancelled by user")` 让上层 fail_connect 走 rollback。
#[cfg(target_os = "macos")]
fn run_setup_with_admin(svcs: &[String], host: &str, port: u16) -> Result<(), String> {
    let mut sh_cmds: Vec<String> = Vec::with_capacity(svcs.len() * 2);
    for svc in svcs {
        sh_cmds.push(format!(
            "{} -setsocksfirewallproxy {} {} {}",
            NETWORKSETUP,
            sh_quote(svc),
            host,
            port,
        ));
        sh_cmds.push(format!(
            "{} -setsocksfirewallproxystate {} on",
            NETWORKSETUP,
            sh_quote(svc),
        ));
    }
    let sh = sh_cmds.join("; ");
    run_osascript_admin(&sh)
}

/// 用 osascript 走 admin 提权一次性关掉所有 service 的 SOCKS。
#[cfg(target_os = "macos")]
fn run_disable_with_admin(svcs: &[String]) -> Result<(), String> {
    let mut sh_cmds: Vec<String> = Vec::with_capacity(svcs.len());
    for svc in svcs {
        sh_cmds.push(format!(
            "{} -setsocksfirewallproxystate {} off",
            NETWORKSETUP,
            sh_quote(svc),
        ));
    }
    let sh = sh_cmds.join("; ");
    run_osascript_admin(&sh)
}

#[cfg(target_os = "macos")]
fn run_osascript_admin(sh_script: &str) -> Result<(), String> {
    let apple = format!(
        "do shell script {} with administrator privileges",
        applescript_string_literal(sh_script)
    );
    let out = Command::new(OSASCRIPT)
        .args(["-e", &apple])
        .output()
        .map_err(|e| format!("spawn {OSASCRIPT} failed: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr_trim = stderr.trim();
    // osascript 用户取消的典型 stderr: "execution error: User canceled. (-128)"
    if stderr_trim.contains("User canceled")
        || stderr_trim.contains("(-128)")
        || stderr_trim.contains("cancel")
    {
        return Err("admin password prompt cancelled by user".into());
    }
    Err(format!(
        "{OSASCRIPT} admin elevation failed: exit {} stderr={stderr_trim}",
        out.status,
    ))
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

    // 提权 fallback 相关 helper 仅 macOS 编译,所以测试也加 cfg。
    #[cfg(target_os = "macos")]
    mod elevation {
        use super::super::{applescript_string_literal, pick_target_services, sh_quote};

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

        #[test]
        fn sh_quote_wraps_plain_value_in_single_quotes() {
            assert_eq!(sh_quote("Wi-Fi"), "'Wi-Fi'");
            assert_eq!(sh_quote(""), "''");
        }

        #[test]
        fn sh_quote_handles_embedded_single_quote() {
            // service 名 "Bob's iPhone" → 'Bob'"'"'s iPhone'
            // 这是把"已经在单引号内"的状态先 break 出去 (以双引号包一个 ')
            // 再回到单引号内继续。
            assert_eq!(sh_quote("Bob's iPhone"), "'Bob'\"'\"'s iPhone'");
        }

        #[test]
        fn sh_quote_passes_through_spaces_and_unicode() {
            assert_eq!(sh_quote("iPhone USB"), "'iPhone USB'");
            assert_eq!(sh_quote("以太网 1"), "'以太网 1'");
        }

        #[test]
        fn applescript_string_escapes_quotes_and_backslashes() {
            assert_eq!(applescript_string_literal("plain"), "\"plain\"");
            assert_eq!(
                applescript_string_literal(r#"a"b\c"#),
                r#""a\"b\\c""#
            );
        }
    }
}
