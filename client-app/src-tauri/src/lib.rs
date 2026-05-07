#[cfg(target_os = "macos")]
mod autostart;
mod commands;
mod error;
mod proxy;
mod state;
mod tray;

use std::sync::Arc;

use conduit_core::{pick_unused_ports, wait_until_ready};
use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};

use crate::error::ConduitError;
use crate::proxy::{control_api, ClientConfig, ClientCore};
use crate::state::{AppRuntime, AppState, LifecyclePhase};

const HEALTHZ_TIMEOUT_SEC: u64 = 30;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let (socks_port, api_port) = match pick_unused_ports(2) {
        Some(v) if v.len() == 2 => (v[0], v[1]),
        _ => {
            eprintln!("FATAL: could not allocate two free TCP ports");
            std::process::exit(2);
        }
    };

    let runtime = AppRuntime::booting(socks_port, api_port);
    info!("boot socks={} api={}", socks_port, api_port);

    // ClientCore 进程内承载 SOCKS5 入口 / 路由决策 / mDNS 发现 / 控制 API。
    let cfg = ClientConfig::with_ports(socks_port, api_port);
    let core = Arc::new(ClientCore::new(cfg));
    let core_for_setup = core.clone();
    let core_for_runevent = core.clone();
    let core_for_manage = core.clone();

    let app_state = AppState::new(runtime);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .manage(core_for_manage)
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime,
            commands::open_external,
            commands::show_main_window,
            commands::quit_app,
            commands::restart_app,
            commands::autostart_status,
            commands::autostart_enable,
            commands::autostart_disable,
        ])
        .setup(move |app| {
            tray::setup(app)?;

            let handle = app.handle().clone();
            let core = core_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = boot_sequence(handle, core).await {
                    error!("client boot failed: {e}");
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| {
            // 用户点窗口左上角红色"关闭"按钮：拦截 close 改成隐藏到 tray，
            // 保持后台 SOCKS5 / 系统代理设置不被中断。
            // 真正退出走 tray 菜单的"退出"或 cmd-Q。
            if let RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } = &event
            {
                if label == "main" {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.hide();
                    }
                    api.prevent_close();
                    return;
                }
            }

            if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
                let core = core_for_runevent.clone();
                let h = app.clone();
                tauri::async_runtime::block_on(async move {
                    request_graceful_shutdown(&h, core).await;
                });
            }
        });
}

async fn boot_sequence(handle: AppHandle, core: Arc<ClientCore>) -> Result<(), ConduitError> {
    let state = handle.state::<AppState>();
    state.set_phase(LifecyclePhase::Booting, None);
    let _ = handle.emit("boot:phase", "booting");

    if let Err(e) = core.start().await {
        warn!("client_core start failed: {e}");
        state.set_phase(LifecyclePhase::Failed, Some(e.clone()));
        let _ = handle.emit("boot:phase", "failed");
        let _ = handle.emit("boot:error", e.clone());
        return Err(ConduitError::Internal(e));
    }
    info!("[boot] client_core started");

    let api_port = core.config().api_port;
    if let Err(e) = control_api::start(core.clone(), api_port).await {
        warn!("control_api start failed: {e}");
        state.set_phase(LifecyclePhase::Failed, Some(e.clone()));
        let _ = handle.emit("boot:phase", "failed");
        let _ = handle.emit("boot:error", e.clone());
        return Err(ConduitError::Internal(e));
    }
    state.set_sidecar_pid(Some(std::process::id()));

    match wait_until_ready(api_port, HEALTHZ_TIMEOUT_SEC).await {
        Ok(_) => {
            state.set_phase(LifecyclePhase::Ready, None);
            let _ = handle.emit("boot:phase", "ready");
            if let Some(w) = handle.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            warn!("healthz timeout: {msg}");
            state.set_phase(LifecyclePhase::Failed, Some(msg.clone()));
            let _ = handle.emit("boot:phase", "failed");
            let _ = handle.emit("boot:error", msg);
            Err(ConduitError::HealthzTimeout(HEALTHZ_TIMEOUT_SEC))
        }
    }
}

async fn request_graceful_shutdown(handle: &AppHandle, core: Arc<ClientCore>) {
    if let Some(state) = handle.try_state::<AppState>() {
        state.set_phase(LifecyclePhase::Stopped, None);
    }
    core.stop().await;
    info!("client_core stopped");
}
