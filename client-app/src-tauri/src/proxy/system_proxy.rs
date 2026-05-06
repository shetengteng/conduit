//! `MacSystemProxy` —— macOS `networksetup` 系统 SOCKS 代理 wrapper。
//!
//! 调用 4 个 networksetup 命令：
//! - `-listallnetworkservices`            列出网络服务
//! - `-getsocksfirewallproxy <svc>`       读当前 SOCKS 配置
//! - `-setsocksfirewallproxy <svc> ...`   设置 SOCKS 服务器
//! - `-setsocksfirewallproxystate <svc>`  开/关
//!
//! 注：Linux/Windows 平台没有这个能力，`enable` / `disable` 直接 noop。

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

    /// 把所有可见网络服务的 SOCKS 代理改成指向 `host:port` 并开启。
    #[cfg(target_os = "macos")]
    pub fn enable(&self, host: &str, port: u16) -> Result<(), String> {
        let svcs = list_services()?;
        if svcs.is_empty() {
            return Err("no network services visible".into());
        }
        let mut last_err = None;
        let mut ok_count = 0usize;
        for svc in &svcs {
            if let Err(e) = set_socks(svc, host, port) {
                log::warn!("[system_proxy] set on '{svc}' failed: {e}");
                last_err = Some(e);
                continue;
            }
            if let Err(e) = enable_socks(svc) {
                log::warn!("[system_proxy] enable on '{svc}' failed: {e}");
                last_err = Some(e);
                continue;
            }
            ok_count += 1;
        }
        if ok_count == 0 {
            return Err(last_err.unwrap_or_else(|| "all services failed".into()));
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

    /// 关闭所有可见网络服务的 SOCKS 代理。
    #[cfg(target_os = "macos")]
    pub fn disable(&self) -> Result<(), String> {
        let svcs = list_services()?;
        for svc in &svcs {
            let _ = disable_socks(svc);
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
}
