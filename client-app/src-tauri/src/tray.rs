use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager};

/// M-α 阶段：单态托盘 + 最简菜单（打开主窗口 / 退出）。
/// 4 态切换（🟢/🔵/🟡/⚫）推到 M-δ 与 connectivity composable 一同实现。
pub fn setup(app: &App) -> tauri::Result<()> {
    let handle = app.handle();
    let item_show = MenuItemBuilder::with_id("show", "打开主窗口").build(handle)?;
    let item_quit = MenuItemBuilder::with_id("quit", "退出 Conduit Client").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&item_show)
        .separator()
        .item(&item_quit)
        .build()?;

    let _tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon(handle.default_window_icon().unwrap().clone())
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_window(app),
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
