mod commands;
mod error;
mod proxy;
mod state;
mod tray;

use conduit_core::{pick_unused_ports, wait_until_ready};
use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};

use crate::error::ConduitError;
use crate::proxy::{ProxyConfig, ProxyCore};
use crate::state::{AppRuntime, AppState, LifecyclePhase};

const HEALTHZ_TIMEOUT_SEC: u64 = 30;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    let (http_port, socks5_port, api_port) = match pick_unused_ports(3) {
        Some(v) if v.len() == 3 => (v[0], v[1], v[2]),
        _ => {
            eprintln!("FATAL: could not allocate three free TCP ports");
            std::process::exit(2);
        }
    };

    let runtime = AppRuntime::booting(http_port, socks5_port, api_port);
    info!(
        "boot http={} socks5={} api={}",
        http_port, socks5_port, api_port
    );

    // ProxyCore 进程内承载 HTTP / SOCKS5 / mDNS / 控制 API（无外部 sidecar 进程）。
    let proxy_cfg = ProxyConfig::with_ports(http_port, socks5_port, api_port);
    let proxy = ProxyCore::new(proxy_cfg);

    let app_state = AppState::new(runtime);
    let proxy_for_setup = proxy.clone();
    let proxy_for_runevent = proxy.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(app_state)
        .manage(proxy)
        .invoke_handler(tauri::generate_handler![
            commands::get_runtime,
            commands::open_external,
            commands::show_main_window,
            commands::quit_app,
            commands::restart_app,
        ])
        .setup(move |app| {
            tray::setup(app)?;

            let handle = app.handle().clone();
            let proxy = proxy_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = boot_sequence(handle, proxy).await {
                    error!("boot failed: {e}");
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app, event| {
            // 关闭按钮 → 隐藏到托盘，保持 ProxyCore 后台运行
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

            // 真正退出（cmd-Q / tray quit）：触发 ProxyCore 优雅关闭
            if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
                let proxy = proxy_for_runevent.clone();
                let h = app.clone();
                tauri::async_runtime::block_on(async move {
                    request_graceful_shutdown(&h, &proxy).await;
                });
            }
        });
}

async fn boot_sequence(handle: AppHandle, proxy: ProxyCore) -> Result<(), ConduitError> {
    let state = handle.state::<AppState>();
    state.set_phase(LifecyclePhase::Booting, None);
    let _ = handle.emit("boot:phase", "booting");

    if let Err(e) = proxy.start().await {
        state.set_phase(LifecyclePhase::Failed, Some(e.clone()));
        let _ = handle.emit("boot:phase", "failed");
        let _ = handle.emit("boot:error", e.clone());
        return Err(ConduitError::Internal(e));
    }
    info!("ProxyCore started in-process");
    state.set_sidecar_pid(Some(std::process::id()));

    match wait_until_ready(proxy.config().api_port, HEALTHZ_TIMEOUT_SEC).await {
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

async fn request_graceful_shutdown(handle: &AppHandle, proxy: &ProxyCore) {
    if let Some(state) = handle.try_state::<AppState>() {
        state.set_phase(LifecyclePhase::Stopped, None);
    }
    proxy.stop().await;
    info!("ProxyCore stopped");
}
