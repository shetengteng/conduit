use std::path::{Path, PathBuf};

use log::{info, warn};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::{ConduitError, ConduitResult};

pub struct SidecarHandle {
    child: Mutex<Option<Child>>,
    pub api_port: u16,
    pub socks_port: u16,
}

impl SidecarHandle {
    pub fn new(socks_port: u16, api_port: u16) -> Self {
        Self {
            child: Mutex::new(None),
            api_port,
            socks_port,
        }
    }

    /// 拉起 client sidecar。
    ///
    /// 两种模式自动适配:
    ///   1. 已打包: 找到本地的 `conduit-client-sidecar[-<triple>][.exe]` 直接 exec。
    ///   2. dev: 回退到 `python3 client-app/core/client_main.py`。
    pub async fn spawn(&self) -> ConduitResult<u32> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Err(ConduitError::Internal("sidecar already running".into()));
        }

        let parent_pid = std::process::id();
        let common_args = vec![
            "--bind-port".to_string(),
            self.socks_port.to_string(),
            "--api-port".to_string(),
            self.api_port.to_string(),
            "--watchdog-ppid".to_string(),
            parent_pid.to_string(),
        ];

        let mut cmd = if let Some(bin) = locate_sidecar_binary("conduit-client-sidecar") {
            info!(
                "spawning client sidecar (bundled binary): {} (socks={}, api={})",
                bin.display(),
                self.socks_port,
                self.api_port,
            );
            let mut c = Command::new(&bin);
            c.args(&common_args).kill_on_drop(true);
            c
        } else {
            let core_dir = locate_core_dir().ok_or_else(|| {
                ConduitError::SidecarSpawn(
                    "could not locate sidecar binary nor client-app/core (set CONDUIT_CLIENT_CORE_DIR to override)"
                        .into(),
                )
            })?;
            let entrypoint = core_dir.join("client_main.py");
            if !entrypoint.exists() {
                return Err(ConduitError::SidecarSpawn(format!(
                    "missing {}",
                    entrypoint.display()
                )));
            }
            info!(
                "spawning client sidecar (dev: python3): {} (socks={}, api={})",
                entrypoint.display(),
                self.socks_port,
                self.api_port,
            );
            let mut c = Command::new("python3");
            c.arg(&entrypoint)
                .args(&common_args)
                .current_dir(&core_dir)
                .kill_on_drop(true);
            c
        };

        // M-β.2：默认开启系统代理切换 —— connect_to() 会在握手成功后启用，
        // disconnect()/_connect_failed 会自动 rollback。
        // 用 CONDUIT_NO_SYSTEM_PROXY=1 临时关掉（隔离调试用，需用户手动配 SOCKS5）。
        if std::env::var("CONDUIT_NO_SYSTEM_PROXY").as_deref() == Ok("1") {
            cmd.arg("--no-system-proxy");
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
                    info!("client sidecar killed");
                }
                Err(e) => warn!("client sidecar kill failed: {e}"),
            }
        }
    }

    #[allow(dead_code)]
    pub async fn is_alive(&self) -> bool {
        let mut guard = self.child.lock().await;
        match guard.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }
}

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
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest
        .join("binaries")
        .join(format!("{name}-{triple}{exe_suffix}"));
    if dev_path.is_file() {
        return Some(dev_path);
    }
    None
}

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
    if let Ok(custom) = std::env::var("CONDUIT_CLIENT_CORE_DIR") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
    }

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest
        .parent()
        .map(|p| p.join("core"))
        .filter(|p| p.join("client_main.py").exists());
    if candidate.is_some() {
        return candidate;
    }

    let cwd = std::env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let probe = ancestor.join("client-app").join("core");
        if probe.join("client_main.py").exists() {
            return Some(probe);
        }
    }
    None
}
