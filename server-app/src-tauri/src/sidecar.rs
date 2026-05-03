use std::path::{Path, PathBuf};

use log::{info, warn};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::{ConduitError, ConduitResult};

pub struct SidecarHandle {
    child: Mutex<Option<Child>>,
    pub api_port: u16,
    pub http_port: u16,
    pub socks5_port: u16,
}

impl SidecarHandle {
    pub fn new(http_port: u16, socks5_port: u16, api_port: u16) -> Self {
        Self {
            child: Mutex::new(None),
            api_port,
            http_port,
            socks5_port,
        }
    }

    /// Spawn the sidecar.
    ///
    /// 两种模式自动适配:
    ///   1. release / 已打包: 找到本地的 `conduit-server-sidecar[-<triple>][.exe]`
    ///      直接 exec 它(PyInstaller 打的单二进制)。
    ///   2. dev: 回退到 `python3 server-app/core/proxy_server.py`。
    pub async fn spawn(&self) -> ConduitResult<u32> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Err(ConduitError::Internal("sidecar already running".into()));
        }

        let parent_pid = std::process::id();
        let common_args = vec![
            "--yes".to_string(),
            "--http-port".to_string(),
            self.http_port.to_string(),
            "--socks-port".to_string(),
            self.socks5_port.to_string(),
            "--api-port".to_string(),
            self.api_port.to_string(),
            "--watchdog-ppid".to_string(),
            parent_pid.to_string(),
        ];

        let mut cmd = if let Some(bin) = locate_sidecar_binary("conduit-server-sidecar") {
            info!(
                "spawning sidecar (bundled binary): {} (http={}, socks={}, api={})",
                bin.display(),
                self.http_port,
                self.socks5_port,
                self.api_port,
            );
            let mut c = Command::new(&bin);
            c.args(&common_args).kill_on_drop(true);
            c
        } else {
            let core_dir = locate_core_dir().ok_or_else(|| {
                ConduitError::SidecarSpawn(
                    "could not locate sidecar binary nor server-app/core (set CONDUIT_CORE_DIR to override)".into(),
                )
            })?;
            let entrypoint = core_dir.join("proxy_server.py");
            if !entrypoint.exists() {
                return Err(ConduitError::SidecarSpawn(format!(
                    "missing {}",
                    entrypoint.display()
                )));
            }
            info!(
                "spawning sidecar (dev: python3): {} (http={}, socks={}, api={})",
                entrypoint.display(),
                self.http_port,
                self.socks5_port,
                self.api_port,
            );
            let mut c = Command::new("python3");
            c.arg(&entrypoint)
                .args(&common_args)
                .current_dir(&core_dir)
                .kill_on_drop(true);
            c
        };

        // M-β.1：mDNS 默认启用 —— client-app 依赖广播来自动发现 server。
        // 用 CONDUIT_NO_MDNS=1 env var 临时关掉（隔离调试用）。
        if std::env::var("CONDUIT_NO_MDNS").as_deref() == Ok("1") {
            cmd.arg("--no-mdns");
        }

        let child = cmd
            .spawn()
            .map_err(|e| ConduitError::SidecarSpawn(format!("python3 spawn: {e}")))?;

        let pid = child.id().unwrap_or(0);
        *guard = Some(child);
        Ok(pid)
    }

    pub async fn kill(&self) {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            match child.start_kill() {
                Ok(_) => {
                    let _ = child.wait().await;
                    info!("sidecar killed");
                }
                Err(e) => warn!("sidecar kill failed: {e}"),
            }
        }
    }

    pub async fn is_alive(&self) -> bool {
        let mut guard = self.child.lock().await;
        match guard.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }
}

/// 在 Tauri externalBin 约定的位置找已打包的 sidecar 二进制。
///
/// 查找顺序:
///   1. $CONDUIT_SIDECAR_BIN 环境变量(直接给绝对路径)
///   2. <exe_dir>/<name>[.exe]               (Tauri externalBin 部署位置)
///   3. <exe_dir>/<name>-<host_triple>[.exe]  (cargo run + 直接放在 binaries 里时的 fallback)
///   4. <repo>/server-app/src-tauri/binaries/<name>-<host_triple>[.exe]  (本地 build-sidecars.sh 产出)
fn locate_sidecar_binary(name: &str) -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("CONDUIT_SIDECAR_BIN") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let triple = host_triple();

    let candidates = [
        exe_dir.join(format!("{name}{exe_suffix}")),
        exe_dir.join(format!("{name}-{triple}{exe_suffix}")),
    ];
    for c in candidates.iter() {
        if c.is_file() {
            return Some(c.clone());
        }
    }

    // Dev fallback: 仓库根 binaries 目录(允许跑 cargo run 时直接挑选 build-sidecars.sh 的产物)
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest
        .join("binaries")
        .join(format!("{name}-{triple}{exe_suffix}"));
    if dev_path.is_file() {
        return Some(dev_path);
    }

    None
}

/// 把当前 host 平台映射到 Tauri externalBin 期望的 rustc target triple 字符串。
fn host_triple() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-gnu"
        } else {
            "x86_64-unknown-linux-gnu"
        }
    } else {
        "unknown"
    }
}

fn locate_core_dir() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("CONDUIT_CORE_DIR") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
    }

    // Walk upward from CARGO_MANIFEST_DIR (server-app/src-tauri) to find
    // ../core/proxy_server.py. This works in `cargo tauri dev`.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest
        .parent()
        .map(|p| p.join("core"))
        .filter(|p| p.join("proxy_server.py").exists());
    if candidate.is_some() {
        return candidate;
    }

    let cwd = std::env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let probe = ancestor.join("server-app").join("core");
        if probe.join("proxy_server.py").exists() {
            return Some(probe);
        }
    }
    None
}
