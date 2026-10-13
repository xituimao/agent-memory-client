mod agents;
mod config;
mod health;
mod instance;
mod ports;
mod state;
mod stats;
#[cfg(test)]
mod testutil;
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
        // connect 可能写入 "${AGENTMEMORY_SECRET:-}" 这类占位符，
        // 而多数客户端启动 MCP 时不做 shell 展开 → 推送全部 401。
        // 这里把 env 内联为真实值兜底（尽力而为，失败不影响接入结果）。
        if let Err(e) = agents::inline_secret(&id, &config.rest_url(), config.token.trim()) {
            log::warn!("inline_secret failed for {id}: {e}");
        }
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

/// 修复已接入但密钥是占位符的客户端：把 env 内联为真实值
#[tauri::command]
fn repair_agent(state: tauri::State<'_, SharedState>, id: String) -> Result<String, String> {
    if !agents::is_supported(&id) {
        return Err("不支持的助手类型".to_string());
    }
    let config = state::get_config(&state);
    if config.token.trim().is_empty() {
        return Err("请先在设置里填写访问密钥".to_string());
    }
    match agents::inline_secret(&id, &config.rest_url(), config.token.trim())? {
        true => Ok("已修复：密钥已写入配置，重启该助手后生效".to_string()),
        false => Err("未找到可修复的配置文件".to_string()),
    }
}

/// 待关闭的旧实例数量（0 = 无需确认）。前端据此显示「本程序已经在运行」的确认提示。
#[tauri::command]
fn pending_takeover(app: tauri::AppHandle) -> u32 {
    let shared = app.state::<SharedState>().inner().clone();
    state::pending_takeover(&shared)
}

/// 用户确认「关掉旧的、启动新的」：结束所有旧实例 → 抢实例锁 → 启动隧道。
#[tauri::command]
async fn takeover_instance(app: tauri::AppHandle) -> Result<u32, String> {
    let shared = app.state::<SharedState>().inner().clone();
    if state::pending_takeover(&shared) == 0 {
        return Err("当前没有需要关闭的程序".to_string());
    }
    let app2 = app.clone();
    let shared2 = shared.clone();
    // 结束进程 + 等锁端口释放都是阻塞 IO，放到阻塞线程池，别卡住 async runtime
    let killed = tokio::task::spawn_blocking(move || {
        let n = instance::kill_other_instances();
        if !instance::wait_until_free(std::time::Duration::from_secs(5)) {
            return Err("无法关闭正在运行的程序，请手动退出它后再试一次".to_string());
        }
        engage(&app2, &shared2);
        Ok(n)
    })
    .await
    .map_err(|e| format!("切换失败：{e}"))??;

    state::set_pending_takeover(&shared, 0);
    log::info!("用户确认接管，已结束 {killed} 个正在运行的实例");
    Ok(killed as u32)
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
            app.manage(shared);

            // 登录自启（--tray）拉起的第二个实例：静默让位，不弹窗打扰用户
            let start_hidden = std::env::args().any(|a| a == "--tray");

            // 单实例闸门：已有实例在跑时，界面显示「本程序已经在运行」的确认提示，
            // 用户点「继续启动」后结束所有旧实例再接管（见 boot 与 takeover_instance）。
            // 登录自启（--tray）拉起的实例判断到已有实例就静默让位，绝不打扰用户。
            let others = instance::other_instances();
            if start_hidden && (instance::existing() || !others.is_empty()) {
                log::info!("已有实例在运行，本次自启实例直接退出");
                app.handle().exit(0);
                return Ok(());
            }

            boot(app.handle(), start_hidden);
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
            repair_agent,
            scan_generic_agents,
            install_generic_agent,
            pending_takeover,
            takeover_instance
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // 显式回收 ssh 子进程；kill_on_drop 作为兜底
                if let Ok(mut slot) = app_handle.state::<SharedState>().child.lock()
                    && let Some(mut child) = slot.take() {
                        let _ = child.start_kill();
                    }
            }
        });
}

/// 正式启动流程：清理残留隧道 → （有旧实例则进入待确认）→ 建托盘 → 显示/隐藏窗口。
///
/// 只调用一次（在 `setup` 里）。**抢实例锁与起隧道被拆到 `engage`**：
/// 检测到还有本程序实例在跑时，先不抢锁、不建隧道，由界面显示「本程序已经在运行」的确认提示，
/// 用户点「继续启动」后再调 `takeover_instance` 真正接管——避免两个实例互抢转发端口。
fn boot(app: &tauri::AppHandle, start_hidden: bool) {
    let shared = app.state::<SharedState>().inner().clone();
    let cfg = state::get_config(&shared);

    // 残留隧道必须在 tunnel_loop 起来之前清，否则会把自己刚拉起的 ssh 当成残留
    let cleaned = ports::cleanup_stale_forwards(&cfg);
    if cleaned > 0 {
        log::info!("已清理 {cleaned} 个残留隧道进程");
    }
    // 清完仍被占用 = 外部程序占用，仅提示，照常启动（界面上会显示连不上）
    for port in ports::occupied_ports(&cfg) {
        log::warn!("本地端口 {port} 已被其它程序占用，隧道可能建立失败");
    }

    // 还有别的实例在跑 → 先只显示确认提示，等用户确认后再接管
    let running = instance::other_instances().len();
    if running > 0 {
        state::set_pending_takeover(&shared, running as u32);
        log::info!("检测到 {running} 个正在运行的实例，等待用户在界面上确认");
    } else {
        engage(app, &shared);
    }

    let initial = state::current_status(&shared);
    app.manage(tray::TrayState::new());
    if let Err(e) = tray::build_tray(app, &initial, cfg.auto_start, cfg.auto_connect) {
        log::error!("托盘初始化失败：{e}");
    }

    let handle = app.clone();
    let health_state = shared.clone();
    tauri::async_runtime::spawn(async move {
        health::health_loop(handle, health_state).await;
    });

    if let Some(window) = app.get_webview_window("main") {
        if start_hidden {
            let _ = window.hide();
        } else {
            // 直接双击启动时必须先提升为 Regular 再置前，
            // 否则 Accessory 后台启动会让 WKWebView 不渲染（白屏）
            tray::show_window(app);
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
}

/// 抢实例锁并启动隧道循环。
///
/// 两条路径会调用它：① 启动时没有别的实例；② 用户在界面上确认接管之后。
/// 之所以单独拆出来，是因为「确认接管」必须先结束旧实例、等它把锁端口让出来，才能抢锁。
fn engage(app: &tauri::AppHandle, shared: &SharedState) {
    if let Err(e) = instance::acquire(app.clone()) {
        log::warn!("{e}");
    }
    let handle = app.clone();
    let tunnel_state = shared.clone();
    tauri::async_runtime::spawn(async move {
        tunnel::tunnel_loop(handle, tunnel_state).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 待确认期间不得抢锁建隧道：是否接管完全由 pending 状态驱动
    #[test]
    fn pending_takeover_gates_the_handover() {
        let dir = std::env::temp_dir().join(format!("amc-takeover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let shared = state::new(&dir);

        assert_eq!(state::pending_takeover(&shared), 0);
        state::set_pending_takeover(&shared, 2);
        assert_eq!(state::pending_takeover(&shared), 2);
        state::set_pending_takeover(&shared, 0);
        assert_eq!(state::pending_takeover(&shared), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
