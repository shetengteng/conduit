//! 系统托盘 —— M-δ。
//!
//! 菜单结构 (从上到下):
//!   1. 当前状态 (disabled item, 文字会被周期性轮询更新)
//!   2. ─────
//!   3. 打开主窗口
//!   4. 诊断 / 设置 (走 invoke 命令打开窗口并切换路由)
//!   5. ─────
//!   6. 断开连接 (动态启用/禁用)
//!   7. 退出
//!
//! W3 Sprint 3：从原来的 reqwest 拉 `/api/connection` 改成直接 `ClientCore`
//! 进程内方法调用 + EventBus 订阅，去掉 sidecar 依赖。

use std::sync::Arc;
use std::time::Duration;

use log::warn;
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager};
use tokio::time::interval;

use conduit_core::{ConnectionSnapshot, ConnectionState, HeartbeatTone};

use crate::proxy::ClientCore;

const POLL_SEC: u64 = 5;

pub fn setup(app: &App) -> tauri::Result<()> {
    let handle = app.handle();

    let item_status = MenuItemBuilder::with_id("status", "状态: 启动中…")
        .enabled(false)
        .build(handle)?;
    let item_show = MenuItemBuilder::with_id("show", "打开主窗口").build(handle)?;
    let item_diagnose = MenuItemBuilder::with_id("diagnose", "打开诊断页").build(handle)?;
    let item_settings = MenuItemBuilder::with_id("settings", "打开设置页").build(handle)?;
    let item_disconnect = MenuItemBuilder::with_id("disconnect", "断开连接")
        .enabled(false)
        .build(handle)?;
    let item_quit = MenuItemBuilder::with_id("quit", "退出 Conduit Client").build(handle)?;

    let menu = MenuBuilder::new(handle)
        .item(&item_status)
        .separator()
        .item(&item_show)
        .item(&item_diagnose)
        .item(&item_settings)
        .separator()
        .item(&item_disconnect)
        .separator()
        .item(&item_quit)
        .build()?;

    let tray_icon = load_tray_icon(handle).unwrap_or_else(|| {
        warn!("tray icon assets missing, falling back to default window icon");
        handle.default_window_icon().unwrap().clone()
    });

    let _tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon(tray_icon)
        .icon_as_template(true)
        .tooltip("Conduit Client")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_window(app),
            "diagnose" => navigate_to(app, "diagnose"),
            "settings" => navigate_to(app, "settings"),
            "disconnect" => spawn_disconnect(app.clone()),
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

    spawn_status_poller(
        handle.clone(),
        Arc::new(item_status),
        Arc::new(item_disconnect),
    );

    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn navigate_to(app: &AppHandle, key: &str) {
    show_window(app);
    let _ = app.emit("tray:navigate", key.to_string());
}

fn spawn_disconnect(app: AppHandle) {
    let Some(core) = app.try_state::<Arc<ClientCore>>() else {
        return;
    };
    let core = (*core).clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = core.disconnect().await {
            warn!("tray disconnect error: {e}");
        }
    });
}

fn spawn_status_poller(
    handle: AppHandle,
    item_status: Arc<MenuItem<tauri::Wry>>,
    item_disconnect: Arc<MenuItem<tauri::Wry>>,
) {
    tauri::async_runtime::spawn(async move {
        let core = match handle.try_state::<Arc<ClientCore>>() {
            Some(c) => (*c).clone(),
            None => {
                warn!("tray poller: ClientCore handle missing");
                return;
            }
        };

        let mut tick = interval(Duration::from_secs(POLL_SEC));
        tick.tick().await;
        loop {
            let snapshot = core.connection_snapshot().await;
            let (label, can_disconnect) = render_status(&snapshot);
            let _ = item_status.set_text(&label);
            let _ = item_disconnect.set_enabled(can_disconnect);

            let _ = handle.emit("tray:connection_state", snapshot.state.as_str().to_string());

            tick.tick().await;
        }
    });
}

fn load_tray_icon(_handle: &AppHandle) -> Option<Image<'static>> {
    const TRAY_PNG: &[u8] = include_bytes!("../icons/tray/tray-client@2x.png");
    Image::from_bytes(TRAY_PNG).ok()
}

fn render_status(snapshot: &ConnectionSnapshot) -> (String, bool) {
    match snapshot.state {
        ConnectionState::Connected => {
            let label = snapshot
                .server
                .as_ref()
                .map(|s| format!("{}:{}", s.host, s.socks))
                .unwrap_or_else(|| "?".to_string());
            let suffix = match snapshot.heartbeat.as_ref().map(|h| h.tone) {
                Some(HeartbeatTone::Yellow) => format!("{label} · 波动"),
                Some(HeartbeatTone::Red) => format!("{label} · 失联"),
                _ => label,
            };
            (format!("已连接: {suffix}"), true)
        }
        ConnectionState::Connecting => ("状态: 正在连接…".to_string(), false),
        ConnectionState::Disconnecting => ("状态: 断开中…".to_string(), false),
        ConnectionState::Failed => ("状态: 上次连接失败".to_string(), false),
        ConnectionState::Idle => ("状态: 未连接".to_string(), false),
    }
}
