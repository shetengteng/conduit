#[cfg(target_os = "macos")]
mod autostart;
mod commands;
mod error;
mod healthz;
mod sidecar;
mod state;
mod tray;

use std::sync::Arc;

use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};

use crate::error::ConduitError;
use crate::sidecar::SidecarHandle;
use crate::state::{AppRuntime, AppState, LifecyclePhase};

const HEALTHZ_TIMEOUT_SEC: u64 = 9;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let (socks_port, api_port) = match pick_two_ports() {
        Some(t) => t,
        None => {
            eprintln!("FATAL: could not allocate two free TCP ports");
            std::process::exit(2);
        }
    };

    let runtime = AppRuntime::booting(socks_port, api_port);
    info!("boot socks={} api={}", socks_port, api_port);

    let sidecar = Arc::new(SidecarHandle::new(socks_port, api_port));

    let app_state = AppState::new(runtime);

    let sidecar_for_setup = sidecar.clone();
    let sidecar_for_runevent = sidecar.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .manage(sidecar.clone())
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
            let sidecar = sidecar_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = boot_sequence(handle, sidecar).await {
                    error!("client boot failed: {e}");
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| {
            if let RunEvent::WindowEvent {
                event: WindowEvent::CloseRequested { .. },
                ..
            } = &event
            {
                let sc = sidecar_for_runevent.clone();
                let h = app.clone();
                tauri::async_runtime::block_on(async move {
                    request_graceful_shutdown(&h, &sc).await;
                });
            }

            if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
                let sc = sidecar_for_runevent.clone();
                tauri::async_runtime::block_on(async move {
                    sc.kill().await;
                });
            }
        });
}

async fn boot_sequence(
    handle: AppHandle,
    sidecar: Arc<SidecarHandle>,
) -> Result<(), ConduitError> {
    let state = handle.state::<AppState>();
    state.set_phase(LifecyclePhase::Booting, None);
    let _ = handle.emit("boot:phase", "booting");

    match sidecar.spawn().await {
        Ok(pid) => {
            state.set_sidecar_pid(Some(pid));
            info!("client sidecar pid={}", pid);
        }
        Err(e) => {
            state.set_phase(LifecyclePhase::Failed, Some(e.to_string()));
            let _ = handle.emit("boot:phase", "failed");
            let _ = handle.emit("boot:error", e.to_string());
            return Err(e);
        }
    }

    match healthz::wait_until_ready(sidecar.api_port, HEALTHZ_TIMEOUT_SEC).await {
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
            warn!("healthz timeout: {e}");
            state.set_phase(LifecyclePhase::Failed, Some(e.to_string()));
            let _ = handle.emit("boot:phase", "failed");
            let _ = handle.emit("boot:error", e.to_string());
            Err(e)
        }
    }
}

/// 优雅退出：M-α 阶段先做"还没接通 control API 就退" —— 直接 kill。
/// M-δ 完成后会改为先 POST /api/disconnect 还原系统代理，再 kill。
async fn request_graceful_shutdown(handle: &AppHandle, sidecar: &SidecarHandle) {
    if let Some(state) = handle.try_state::<AppState>() {
        state.set_phase(LifecyclePhase::Stopped, None);
    }
    sidecar.kill().await;
}

fn pick_two_ports() -> Option<(u16, u16)> {
    let mut taken = vec![];
    let mut next = || {
        for _ in 0..32 {
            if let Some(p) = portpicker::pick_unused_port() {
                if !taken.contains(&p) {
                    taken.push(p);
                    return Some(p);
                }
            }
        }
        None
    };
    Some((next()?, next()?))
}
