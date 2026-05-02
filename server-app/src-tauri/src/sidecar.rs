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

    /// Spawn the python core. In dev mode we run `python3 proxy_server.py`
    /// directly; in production we'd swap to the bundled sidecar binary
    /// (`conduit-server-sidecar-<triple>`) — left as a follow-up for S4.
    pub async fn spawn(&self) -> ConduitResult<u32> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Err(ConduitError::Internal("sidecar already running".into()));
        }

        let core_dir = locate_core_dir().ok_or_else(|| {
            ConduitError::SidecarSpawn(
                "could not locate server-app/core (set CONDUIT_CORE_DIR to override)".into(),
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
            "spawning sidecar: python3 {} (http={}, socks={}, api={})",
            entrypoint.display(),
            self.http_port,
            self.socks5_port,
            self.api_port,
        );

        let parent_pid = std::process::id();
        let mut cmd = Command::new("python3");
        cmd.arg(&entrypoint)
            .arg("--yes")
            .args([
                "--http-port",
                &self.http_port.to_string(),
                "--socks-port",
                &self.socks5_port.to_string(),
                "--api-port",
                &self.api_port.to_string(),
                "--watchdog-ppid",
                &parent_pid.to_string(),
            ])
            .current_dir(&core_dir)
            .kill_on_drop(true);

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
