use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "macos")]
use crate::autostart;
use crate::error::ConduitError;
use crate::state::{AppRuntime, AppState};

#[tauri::command]
pub fn get_runtime(state: State<'_, AppState>) -> AppRuntime {
    state.snapshot()
}

#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<(), ConduitError> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| ConduitError::Internal(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn show_main_window(app: AppHandle) -> Result<(), ConduitError> {
    if let Some(w) = app.get_webview_window("main") {
        w.show().map_err(|e| ConduitError::Internal(e.to_string()))?;
        let _ = w.set_focus();
    }
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn restart_app(app: AppHandle) -> Result<(), ConduitError> {
    // dev 模式下 `app.restart()` 会杀掉 binary 但 vite dev server (由父 `pnpm tauri dev`
    // 管理) 不会跟着重生，新 binary 加载 devUrl 时拿到空响应就只剩白屏。
    // production 是 frontendDist 静态文件，没这个问题，所以正常 restart。
    if cfg!(debug_assertions) {
        return Err(ConduitError::DevRestartUnsupported(
            "dev mode: please ctrl+c the terminal and rerun `pnpm tauri dev`".to_string(),
        ));
    }
    app.restart();
}

#[tauri::command]
pub fn autostart_status() -> Result<bool, ConduitError> {
    #[cfg(target_os = "macos")]
    {
        autostart::is_enabled()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(ConduitError::Internal("autostart not supported on this OS".into()))
    }
}

#[tauri::command]
pub fn autostart_enable() -> Result<(), ConduitError> {
    #[cfg(target_os = "macos")]
    {
        autostart::enable()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(ConduitError::Internal("autostart not supported on this OS".into()))
    }
}

#[tauri::command]
pub fn autostart_disable() -> Result<(), ConduitError> {
    #[cfg(target_os = "macos")]
    {
        autostart::disable()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(ConduitError::Internal("autostart not supported on this OS".into()))
    }
}
