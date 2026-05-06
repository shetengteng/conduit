#![cfg(target_os = "macos")]
//! macOS LaunchAgent 开机自启 —— M-δ。
//!
//! 通过 `~/Library/LaunchAgents/com.conduit.client.plist` 注册一个 user-scope
//! agent。用户登录后由 launchd 自动拉起当前 .app bundle。
//!
//! 注意:
//!   * 我们刻意 user-scope 而非 system-wide,不需要 sudo。
//!   * 卸载时彻底删除 plist + bootout,而不是仅 Disabled,保证用户可见状态一致。
//!   * 在开发模式 (cargo run) 下,bundle 路径会指向 target/release/bundle/...,
//!     此时退出 app 后 launchd 会启动新进程导致冲突;为避免误伤开发,
//!     `enable()` 在不存在 .app bundle 时直接报错。
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{ConduitError, ConduitResult};

const LABEL: &str = "com.conduit.client";

fn plist_path() -> ConduitResult<PathBuf> {
    let home = home_dir()?;
    Ok(home.join("Library/LaunchAgents").join(format!("{}.plist", LABEL)))
}

fn home_dir() -> ConduitResult<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| ConduitError::Internal("$HOME not set".into()))
}

/// 找到当前正在运行的 .app bundle 路径。
///
/// 在 Tauri 打包后,argv[0] 形如:
///   /Applications/Conduit Client.app/Contents/MacOS/conduit-client-app
/// 我们要的是: /Applications/Conduit Client.app
///
/// 如果路径不含 `.app/Contents/MacOS`,说明是 `cargo run` 直跑可执行文件,
/// 这种情况开机自启没有意义,直接报错。
fn current_app_bundle() -> ConduitResult<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| ConduitError::Internal(format!("current_exe: {e}")))?;
    let s = exe.to_string_lossy().to_string();
    if let Some(idx) = s.find(".app/Contents/MacOS/") {
        let bundle = &s[..idx + 4]; // 包含 ".app"
        return Ok(PathBuf::from(bundle));
    }
    Err(ConduitError::Internal(
        "未找到 .app bundle (开发模式不支持开机自启,请先打包再启用)".into(),
    ))
}

fn render_plist(app_bundle: &Path) -> String {
    // open -a 启动 .app,确保走 macOS 标准启动路径(权限 / Sandbox / Dock)。
    let bundle = app_bundle.to_string_lossy();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>-a</string>
        <string>{bundle}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        label = LABEL,
        bundle = bundle,
    )
}

/// 是否已经启用开机自启 —— 简单判断 plist 文件是否存在。
pub fn is_enabled() -> ConduitResult<bool> {
    Ok(plist_path()?.exists())
}

/// 启用 = 写 plist + launchctl bootstrap 到当前 user GUI session。
pub fn enable() -> ConduitResult<()> {
    let bundle = current_app_bundle()?;
    let path = plist_path()?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ConduitError::Internal(format!("mkdir LaunchAgents: {e}")))?;
    }

    fs::write(&path, render_plist(&bundle))
        .map_err(|e| ConduitError::Internal(format!("write plist: {e}")))?;

    // 即时加载,免得用户重启才生效。bootstrap 失败不致命(下次登录会自动加载)。
    let uid = unsafe { libc::getuid() };
    let domain = format!("gui/{}", uid);
    let _ = Command::new("launchctl")
        .args(["bootout", &domain, &path.to_string_lossy()])
        .status();
    let _ = Command::new("launchctl")
        .args(["bootstrap", &domain, &path.to_string_lossy()])
        .status();

    Ok(())
}

/// 关闭 = launchctl bootout + 删 plist。
pub fn disable() -> ConduitResult<()> {
    let path = plist_path()?;
    if path.exists() {
        let uid = unsafe { libc::getuid() };
        let domain = format!("gui/{}", uid);
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, &path.to_string_lossy()])
            .status();
        fs::remove_file(&path).map_err(|e| ConduitError::Internal(format!("remove plist: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_plist_contains_label_and_bundle() {
        let bundle = PathBuf::from("/Applications/Conduit Client.app");
        let xml = render_plist(&bundle);
        assert!(xml.contains("<string>com.conduit.client</string>"));
        assert!(xml.contains("<string>/Applications/Conduit Client.app</string>"));
        assert!(xml.contains("/usr/bin/open"));
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("<true/>"));
    }

    #[test]
    fn current_app_bundle_rejects_non_app_exe() {
        // current_exe() in tests is target/debug/deps/...; should bail out cleanly.
        let r = current_app_bundle();
        assert!(r.is_err(), "expected non-app exe to be rejected");
    }
}
