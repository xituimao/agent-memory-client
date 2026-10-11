use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::state as st;
use crate::state::Status;

pub struct TrayState {
    pub tray: Mutex<Option<TrayIcon>>,
    pub title_item: Mutex<Option<MenuItem<Wry>>>,
    pub autostart_item: Mutex<Option<CheckMenuItem<Wry>>>,
    pub autoconnect_item: Mutex<Option<CheckMenuItem<Wry>>>,
}

impl TrayState {
    pub fn new() -> Self {
        Self {
            tray: Mutex::new(None),
            title_item: Mutex::new(None),
            autostart_item: Mutex::new(None),
            autoconnect_item: Mutex::new(None),
        }
    }
}

pub fn build_tray(app: &AppHandle, status: &Status, auto_start: bool, auto_connect: bool) -> Result<(), tauri::Error> {
    let title = MenuItem::with_id(app, "status", status.label(), false, None::<&str>)?;
    let open_viewer = MenuItem::with_id(app, "open-viewer", "浏览记忆", true, None::<&str>)?;
    let open_settings = MenuItem::with_id(app, "open-settings", "打开设置…", true, None::<&str>)?;
    let copy_mcp = MenuItem::with_id(app, "copy-mcp", "复制接入命令", true, None::<&str>)?;
    let autostart_item = CheckMenuItem::with_id(app, "toggle-autostart", "登录时启动", true, auto_start, None::<&str>)?;
    let autoconnect_item = CheckMenuItem::with_id(app, "toggle-autoconnect", "启动时自动连接", true, auto_connect, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = MenuBuilder::new(app)
        .item(&title)
        .separator()
        .item(&open_viewer)
        .item(&open_settings)
        .item(&copy_mcp)
        .separator()
        .item(&autostart_item)
        .item(&autoconnect_item)
        .separator()
        .item(&quit)
        .build()?;

    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Agent Memory")
        .icon(load_icon(app, status))
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                toggle_window(app);
            }
        })
        .on_menu_event(|app, event| {
            let state = app.state::<crate::SharedState>();
            match event.id().as_ref() {
                "open-viewer" => {
                    let url = st::get_config(&state).viewer_url();
                    let _ = app.opener().open_url(&url, None::<&str>);
                }
                "open-settings" => show_window(app),
                "copy-mcp" => {
                    let cmd = st::get_config(&state).mcp_command();
                    let _ = app.clipboard().write_text(cmd);
                }
                "toggle-autostart" => {
                    let enable = !st::get_config(&state).auto_start;
                    set_autostart(app, enable);
                }
                "toggle-autoconnect" => {
                    // 只影响下次启动是否自动连接，不打断当前会话
                    let mut config = st::get_config(&state);
                    config.auto_connect = !config.auto_connect;
                    st::set_config(&state, config.clone());
                    let _ = crate::config::save(&state.config_path, &config);
                    sync_checkboxes(app);
                }
                "quit" => app.exit(0),
                _ => {}
            }
        })
        .build(app)?;

    let tray_state = app.state::<TrayState>();
    if let Ok(mut g) = tray_state.tray.lock() { *g = Some(tray); }
    if let Ok(mut g) = tray_state.title_item.lock() { *g = Some(title); }
    if let Ok(mut g) = tray_state.autostart_item.lock() { *g = Some(autostart_item); }
    if let Ok(mut g) = tray_state.autoconnect_item.lock() { *g = Some(autoconnect_item); }
    Ok(())
}

/// 状态变化时更新托盘图标、tooltip 与菜单标题
pub fn refresh_tray(app: &AppHandle, status: &Status) {
    let tray_state = app.state::<TrayState>();
    let Ok(mut guard) = tray_state.tray.lock() else { return };
    if let Some(tray) = guard.as_mut() {
        let _ = tray.set_tooltip(Some(format!("Agent Memory — {}", status.label())));
        let _ = tray.set_icon(Some(load_icon(app, status)));
    }
    drop(guard);
    if let Ok(mut guard) = tray_state.title_item.lock() {
        if let Some(item) = guard.as_mut() {
            let _ = item.set_text(status.label());
        }
    }
}

pub fn sync_checkboxes(app: &AppHandle) {
    let state = app.state::<crate::SharedState>();
    let config = st::get_config(&state);
    let tray_state = app.state::<TrayState>();
    if let Ok(mut guard) = tray_state.autoconnect_item.lock() {
        if let Some(item) = guard.as_mut() {
            let _ = item.set_checked(config.auto_connect);
        }
    }
    if let Ok(mut guard) = tray_state.autostart_item.lock() {
        if let Some(item) = guard.as_mut() {
            let _ = item.set_checked(config.auto_start);
        }
    }
}

pub fn set_autostart(app: &AppHandle, enable: bool) {
    let state = app.state::<crate::SharedState>();
    let mut config = st::get_config(&state);
    let result = if enable {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    if result.is_ok() {
        config.auto_start = enable;
        st::set_config(&state, config.clone());
        let _ = crate::config::save(&state.config_path, &config);
    }
    sync_checkboxes(app);
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // Accessory 应用窗口可能无法前置/获焦，显示期间临时提升为 Regular
        #[cfg(target_os = "macos")]
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            #[cfg(target_os = "macos")]
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        } else {
            show_window(app);
        }
    }
}

fn load_icon(_app: &AppHandle, status: &Status) -> tauri::image::Image<'static> {
    // 图标内嵌进二进制，避免运行时路径问题
    const GREEN: &[u8] = include_bytes!("../icons/tray-green-44.png");
    const GRAY: &[u8] = include_bytes!("../icons/tray-gray-44.png");
    const RED: &[u8] = include_bytes!("../icons/tray-red-44.png");
    let bytes: &[u8] = match status {
        Status::Online { .. } => GREEN,
        Status::Connecting => GRAY,
        Status::Offline { .. } | Status::NotConfigured | Status::Disconnected => RED,
    };
    tauri::image::Image::from_bytes(bytes).expect("embedded tray icon must be valid PNG")
}
