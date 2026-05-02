//! 系统托盘 —— M-δ。
//!
//! 菜单结构 (从上到下):
//!   1. 当前状态 (disabled item, 文字会被 5 秒一次的轮询更新)
//!   2. ─────
//!   3. 打开主窗口
//!   4. 诊断 / 设置 (走 invoke 命令打开窗口并切换路由)
//!   5. ─────
//!   6. 断开连接 (动态启用/禁用)
//!   7. 退出
//!
//! 状态轮询用 reqwest 直接打 sidecar 控制 API,不引入 SSE client,代码更简单。
//! 节流到 5 秒,Rust 端开销可忽略。

use std::sync::Arc;
use std::time::Duration;

use log::warn;
use serde::Deserialize;
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager};
use tokio::time::interval;

use crate::sidecar::SidecarHandle;

const POLL_SEC: u64 = 5;

/// /api/connection 的精简反序列化 —— 只取我们绘制菜单需要的字段。
#[derive(Debug, Deserialize, Clone)]
struct ConnectionSnapshot {
    state: String,
    server: Option<ConnectedServer>,
    heartbeat: Option<HeartbeatBlock>,
}

#[derive(Debug, Deserialize, Clone)]
struct ConnectedServer {
    name: String,
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize, Clone)]
struct HeartbeatBlock {
    tone: String,
}

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

    // 菜单栏专用 template icon(透明 PNG,只用 alpha,macOS 自动 light/dark 反色)
    // 优先用 @2x(44x44),退回到 1x;如果都加载失败,退回到默认窗口图标(会显示成方块,
    // 但至少能看到托盘存在)。
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
    let Some(sidecar) = app.try_state::<Arc<SidecarHandle>>() else {
        return;
    };
    let port = sidecar.api_port;
    tauri::async_runtime::spawn(async move {
        let url = format!("http://127.0.0.1:{port}/api/disconnect");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .ok();
        let Some(client) = client else { return };
        match client.post(&url).send().await {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => warn!("tray disconnect failed: {}", resp.status()),
            Err(e) => warn!("tray disconnect error: {e}"),
        }
    });
}

fn spawn_status_poller(
    handle: AppHandle,
    item_status: Arc<MenuItem<tauri::Wry>>,
    item_disconnect: Arc<MenuItem<tauri::Wry>>,
) {
    tauri::async_runtime::spawn(async move {
        let port = match handle.try_state::<Arc<SidecarHandle>>() {
            Some(s) => s.api_port,
            None => {
                warn!("tray poller: sidecar handle missing");
                return;
            }
        };
        let url = format!("http://127.0.0.1:{port}/api/connection");
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("tray poller: build client failed: {e}");
                return;
            }
        };

        let mut tick = interval(Duration::from_secs(POLL_SEC));
        // 第一次 tick 立刻触发
        tick.tick().await;

        loop {
            let snapshot = client.get(&url).send().await.ok().and_then(|r| {
                if r.status().is_success() {
                    Some(r.json::<ConnectionSnapshot>())
                } else {
                    None
                }
            });
            let snapshot = match snapshot {
                Some(fut) => fut.await.ok(),
                None => None,
            };

            let (label, can_disconnect) = render_status(snapshot.as_ref());
            let _ = item_status.set_text(&label);
            let _ = item_disconnect.set_enabled(can_disconnect);
            // 顺便把状态广播给 Vue,UI 层可拿来同步 sidebar 高亮
            if let Some(s) = snapshot {
                let _ = handle.emit("tray:connection_state", s.state.clone());
            }

            tick.tick().await;
        }
    });
}

/// 从二进制内嵌的 PNG 字节构造 menu bar template icon。
/// 编译期 `include_bytes!` 嵌入,免去 release 模式下查找资源路径的麻烦。
fn load_tray_icon(_handle: &AppHandle) -> Option<Image<'static>> {
    const TRAY_PNG: &[u8] = include_bytes!("../icons/tray/tray-client@2x.png");
    Image::from_bytes(TRAY_PNG).ok()
}

fn render_status(s: Option<&ConnectionSnapshot>) -> (String, bool) {
    let Some(s) = s else {
        return ("状态: 离线 (sidecar 未就绪)".to_string(), false);
    };
    match s.state.as_str() {
        "connected" => {
            let suffix = match (&s.server, s.heartbeat.as_ref().map(|h| h.tone.as_str())) {
                (Some(srv), Some("yellow")) => format!("{} ({}:{}) · 波动", srv.name, srv.host, srv.port),
                (Some(srv), Some("red")) => format!("{} ({}:{}) · 失联", srv.name, srv.host, srv.port),
                (Some(srv), _) => format!("{} ({}:{})", srv.name, srv.host, srv.port),
                _ => "未知 server".to_string(),
            };
            (format!("已连接: {}", suffix), true)
        }
        "connecting" => ("状态: 正在连接…".to_string(), false),
        "disconnecting" => ("状态: 断开中…".to_string(), false),
        "failed" => ("状态: 上次连接失败".to_string(), false),
        _ => ("状态: 未连接".to_string(), false),
    }
}
