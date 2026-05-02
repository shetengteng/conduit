use log::warn;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use tauri::{App, AppHandle, Manager};

use crate::state::AppState;

pub fn setup(app: &App) -> tauri::Result<()> {
    let handle = app.handle();
    let item_show = MenuItemBuilder::with_id("show", "打开主窗口").build(handle)?;
    let item_copy_pac = MenuItemBuilder::with_id("copy_pac", "复制 PAC URL").build(handle)?;
    let item_quit = MenuItemBuilder::with_id("quit", "退出 Conduit Server").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&item_show)
        .item(&item_copy_pac)
        .separator()
        .item(&item_quit)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon(handle.default_window_icon().unwrap().clone())
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_window(app),
            "copy_pac" => copy_pac(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                show_window(app);
            }
        })
        .build(handle)?;

    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn copy_pac(app: &AppHandle) {
    let state = app.state::<AppState>();
    let rt = state.snapshot();
    let url = format!("http://127.0.0.1:{}/proxy.pac", rt.http_port);
    if let Err(e) = write_clipboard(app, &url) {
        warn!("clipboard write failed: {e}");
    }
}

fn write_clipboard(_app: &AppHandle, _text: &str) -> Result<(), String> {
    // tauri-plugin-clipboard-manager is not yet wired in S2; landing in S3.
    // For now we just log so the menu item is observable.
    log::info!("(stub) PAC URL: {}", _text);
    Ok(())
}
