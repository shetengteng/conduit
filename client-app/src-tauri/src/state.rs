use std::sync::Mutex;

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub enum LifecyclePhase {
    Booting,
    Ready,
    Failed,
    Stopped,
}

/// 客户端运行时句柄。
///
/// 与 server-app 不同，client 只暴露两个端口：
///   - socks_port：本地 SOCKS5 listener（local_proxy）
///   - api_port  ：control HTTP API（aiohttp）
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppRuntime {
    pub api_port: u16,
    pub socks_port: u16,
    pub phase: LifecyclePhase,
    pub failure_reason: Option<String>,
    pub sidecar_pid: Option<u32>,
}

impl AppRuntime {
    pub fn booting(socks_port: u16, api_port: u16) -> Self {
        Self {
            api_port,
            socks_port,
            phase: LifecyclePhase::Booting,
            failure_reason: None,
            sidecar_pid: None,
        }
    }
}

pub struct AppState {
    pub runtime: Mutex<AppRuntime>,
}

impl AppState {
    pub fn new(rt: AppRuntime) -> Self {
        Self {
            runtime: Mutex::new(rt),
        }
    }

    pub fn snapshot(&self) -> AppRuntime {
        self.runtime.lock().expect("AppState poisoned").clone()
    }

    pub fn set_phase(&self, phase: LifecyclePhase, reason: Option<String>) {
        let mut rt = self.runtime.lock().expect("AppState poisoned");
        rt.phase = phase;
        rt.failure_reason = reason;
    }

    pub fn set_sidecar_pid(&self, pid: Option<u32>) {
        self.runtime.lock().expect("AppState poisoned").sidecar_pid = pid;
    }
}
