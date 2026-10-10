mod config;
mod health;
mod state;
mod tray;
mod tunnel;

use tauri::Manager;

use crate::state::{SharedState, Status};

#[tauri::command]
fn get_config(state: tauri::State<SharedState>) -> config::AppConfig {
    state.get_config()
}

#[tauri::command]
fn get_status(state: tauri::State<SharedState>) -> Status {
    state.current_status()
}

#[tauri::command]
fn save_config(
    app: tauri::AppHandle,
    state: tauri::State<SharedState>,
    config: config::AppConfig,
) -> Result<(), String> {
    if config.host.trim().is_empty() {
        return Err("服务器地址不能为空".to_string());
    }
    if config.token.trim().is_empty() {
        return Err("Bearer Token 不能为空".to_string());
    }
    config::save(&state.config_path, &config)?;
    state.set_config(config.clone());

    // 登录自启开关热生效
    let autostart_result = if config.auto_start {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    if let Err(e) = autostart_result {
        log::warn!("autostart toggle failed: {e}");
    }

    // 重启隧道应用新配置
    state::bump_generation(&state);
    state.tunnel_notify.notify_one();
    tray::sync_checkboxes(&app);
    Ok(())
}

/// 用表单里（尚未保存的）配置立即探活一次，返回延迟毫秒数
#[tauri::command]
async fn test_connection(config: config::AppConfig) -> Result<u64, String> {
    if config.token.trim().is_empty() {
        return Err("请先填写 Bearer Token".to_string());
    }
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|e| e.to_string())?;
    health::probe(&client, &config).await
}

/// 手动重启隧道
#[tauri::command]
fn restart_tunnel(state: tauri::State<SharedState>) {
    state::bump_generation(&state);
    state.tunnel_notify.notify_one();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(vec!["--tray"])
                .build(),
        )
        .setup(|app| {
            let app_config_dir = app.path().app_config_dir()?;
            let shared = state::new(&app_config_dir);
            let initial = shared.current_status();

            app.manage(tray::TrayState::new());
            tray::build_tray(
                app.handle(),
                &initial,
                shared.get_config().auto_start,
                shared.get_config().auto_connect,
            )?;

            let handle = app.handle().clone();
            let tunnel_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                tunnel::tunnel_loop(handle, tunnel_state).await;
            });

            let handle = app.handle().clone();
            let health_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                health::health_loop(handle, health_state).await;
            });

            app.manage(shared);

            // 点关闭按钮时隐藏到托盘，而不是退出
            if let Some(window) = app.get_webview_window("main") {
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_clone.hide();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_status,
            save_config,
            test_connection,
            restart_tunnel
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
