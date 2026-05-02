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

    /// 拉起 client_main.py。开发模式直接 `python3` 跑源码；
    /// 生产模式将切换到 Nuitka 打包出的 sidecar binary（M-δ 完成）。
    pub async fn spawn(&self) -> ConduitResult<u32> {
        let mut guard = self.child.lock().await;
        if guard.is_some() {
            return Err(ConduitError::Internal("sidecar already running".into()));
        }

        let core_dir = locate_core_dir().ok_or_else(|| {
            ConduitError::SidecarSpawn(
                "could not locate client-app/core (set CONDUIT_CLIENT_CORE_DIR to override)"
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
            "spawning client sidecar: python3 {} (socks={}, api={})",
            entrypoint.display(),
            self.socks_port,
            self.api_port,
        );

        let parent_pid = std::process::id();
        let mut cmd = Command::new("python3");
        cmd.arg(&entrypoint)
            .args([
                "--bind-port",
                &self.socks_port.to_string(),
                "--api-port",
                &self.api_port.to_string(),
                "--watchdog-ppid",
                &parent_pid.to_string(),
            ])
            .current_dir(&core_dir)
            .kill_on_drop(true);

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
