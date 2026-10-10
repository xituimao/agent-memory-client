mod config;
mod health;
mod state;
mod tray;
mod tunnel;

use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;

use crate::state::{SharedState, Status};

#[tauri::command]
fn get_config(state: tauri::State<SharedState>) -> config::AppConfig {
    state::get_config(&state)
}

#[tauri::command]
fn get_status(state: tauri::State<SharedState>) -> Status {
    state::current_status(&state)
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
    if config.ssh_port == 0 || config.rest_port == 0 || config.viewer_port == 0 {
        return Err("端口不能为 0".to_string());
    }
    if config.rest_port == config.viewer_port {
        return Err("REST 端口与 Viewer 端口不能相同".to_string());
    }
    config::save(&state.config_path, &config)?;
    state::set_config(&state, config.clone());

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

/// VPN 式「连接」按钮：立即建隧道（无论启动自连开关）
#[tauri::command]
fn connect_tunnel(state: tauri::State<SharedState>) {
    state.manual_disconnect.store(false, std::sync::atomic::Ordering::SeqCst);
    state::bump_generation(&state);
    state.tunnel_notify.notify_one();
}

/// VPN 式「断开」按钮：手动断开，不再自动重连
#[tauri::command]
fn disconnect_tunnel(state: tauri::State<SharedState>) {
    state.manual_disconnect.store(true, std::sync::atomic::Ordering::SeqCst);
    state::bump_generation(&state);
    state.tunnel_notify.notify_one();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(vec!["--tray"])
                .build(),
        )
        .setup(|app| {
            // SIGINT/SIGTERM 转为正常退出，保证 RunEvent::Exit 的子进程回收一定执行
            let exit_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};
                    if let Ok(mut term) = signal(SignalKind::terminate()) {
                        tokio::select! {
                            _ = term.recv() => {}
                            _ = tokio::signal::ctrl_c() => {}
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = tokio::signal::ctrl_c().await;
                }
                exit_handle.exit(0);
            });

            // 纯托盘应用：不在 Dock 常驻图标，窗口按需显示
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_config_dir = app.path().app_config_dir()?;
            let shared = state::new(&app_config_dir);
            let initial = state::current_status(&shared);

            app.manage(tray::TrayState::new());
            tray::build_tray(
                app.handle(),
                &initial,
                state::get_config(&shared).auto_start,
                state::get_config(&shared).auto_connect,
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

            // 登录自启/托盘启动带 --tray 参数：不弹窗口，只驻留托盘
            let start_hidden = std::env::args().any(|a| a == "--tray");
            if let Some(window) = app.get_webview_window("main") {
                if start_hidden {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_clone.hide();
                        #[cfg(target_os = "macos")]
                        let _ = window_clone
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
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
            connect_tunnel,
            disconnect_tunnel
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // 显式回收 ssh 子进程；kill_on_drop 作为兜底
                if let Ok(mut slot) = app_handle.state::<SharedState>().child.lock() {
                    if let Some(mut child) = slot.take() {
                        let _ = child.start_kill();
                    }
                }
            }
        });
}
