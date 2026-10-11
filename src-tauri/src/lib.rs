mod agents;
mod config;
mod health;
mod state;
mod stats;
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

/// 扫描本机已安装的 AI 助手
#[tauri::command]
fn scan_agents() -> Vec<agents::AgentInfo> {
    agents::scan()
}

/// 扫描通用 MCP 客户端（官方 connect 名单之外的）
#[tauri::command]
fn scan_generic_agents() -> Vec<agents::GenericAgentInfo> {
    agents::scan_generic()
}

/// 向通用客户端注入 agentmemory MCP 配置
#[tauri::command]
fn install_generic_agent(
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<String, String> {
    let config = state::get_config(&state);
    if config.token.trim().is_empty() {
        return Err("请先在设置里填写访问密钥".to_string());
    }
    agents::install_generic(&id, &config.rest_url(), config.token.trim())
}

/// 给指定助手安装 MCP + SKILL（官方 connect 命令，逐个手动触发）
#[tauri::command]
async fn install_agent(
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<String, String> {
    if !agents::is_supported(&id) {
        return Err("不支持的助手类型".to_string());
    }
    let config = state::get_config(&state);
    if config.token.trim().is_empty() {
        return Err("请先在设置里填写访问密钥".to_string());
    }
    // 需要隧道已建立，connect 要访问 localhost:3111
    if !state.ssh_alive.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("请先连接服务器（圆盘点到绿色）".to_string());
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(180), async {
        tokio::process::Command::new("npx")
            .args(["-y", "@agentmemory/agentmemory@latest", "connect", &id])
            .env("AGENTMEMORY_URL", config.rest_url())
            .env("AGENTMEMORY_SECRET", config.token.trim())
            .output()
            .await
    })
    .await
    .map_err(|_| "安装超时（180 秒）".to_string())?
    .map_err(|e| format!("无法启动 npx：{e}（需要安装 Node.js）"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail = format!("{stdout}\n{stderr}")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let tail: String = tail.chars().rev().take(600).collect::<String>().chars().rev().collect();

    if output.status.success() {
        Ok(tail)
    } else {
        Err(format!("安装失败（退出码 {:?}）：\n{tail}", output.status.code()))
    }
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
            disconnect_tunnel,
            scan_agents,
            install_agent,
            scan_generic_agents,
            install_generic_agent
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
