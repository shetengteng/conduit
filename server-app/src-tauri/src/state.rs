use std::sync::Mutex;

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub enum LifecyclePhase {
    Booting,
    Ready,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppRuntime {
    pub api_port: u16,
    pub http_port: u16,
    pub socks5_port: u16,
    pub phase: LifecyclePhase,
    pub failure_reason: Option<String>,
    pub sidecar_pid: Option<u32>,
}

impl AppRuntime {
    pub fn booting(http_port: u16, socks5_port: u16, api_port: u16) -> Self {
        Self {
            api_port,
            http_port,
            socks5_port,
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
