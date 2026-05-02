use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::ShellExt;

use crate::error::ConduitError;
use crate::state::{AppRuntime, AppState};

#[tauri::command]
pub fn get_runtime(state: State<'_, AppState>) -> AppRuntime {
    state.snapshot()
}

#[tauri::command]
pub async fn open_external(app: AppHandle, url: String) -> Result<(), ConduitError> {
    app.shell()
        .open(&url, None)
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
pub fn restart_app(app: AppHandle) {
    app.restart();
}
