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
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};

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

            // 单实例闸门：已有实例在跑时，让用户决定用哪一个。
            // 必须在起第二个托盘图标 / 第二条隧道之前拦下，否则两个实例会互抢转发端口。
            if instance::existing() {
                if start_hidden {
                    log::info!("已有实例在运行，本次自启实例直接退出");
                    app.handle().exit(0);
                    return Ok(());
                }
                ask_which_instance(app.handle(), start_hidden);
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
            install_generic_agent
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

/// 检测到已有实例时：先显示窗口，再弹「用新的 / 用旧的」让用户当场选，按选择接管或让位。
///
/// 两处实测教训（都踩过）：
/// 1. 弹窗必须有可见窗口——Accessory 应用窗口还没显示时，弹窗会「看不见就返回默认结果」，
///    用户等于没得选，所以先 `tray::show_window` 并稍等它成为 key window；
/// 2. 必须用阻塞式弹窗——异步 `show_with_result` 在 macOS 上实测不等用户点击就回 Cancel。
///    官方要求阻塞式弹窗不要在主线程序调用，而接管旧实例本来就要做秒级阻塞 IO，
///    因此整段协商都放在独立线程里，最后再回主线程 `boot`。
fn ask_which_instance(app: &tauri::AppHandle, start_hidden: bool) {
    tray::show_window(app);
    // 激活本 app：非活跃 app 的原生模态框会「看不见就返回默认值」，用户等于没得选
    #[cfg(target_os = "macos")]
    let _ = app.show();
    let handle = app.clone();
    std::thread::spawn(move || {
        // 等窗口真正成为 key window：不聚焦时原生模态框会立刻返回（等于没等用户），
        // 这里轮询到获焦为止（最多 2 秒），再补一次聚焦。
        for _ in 0..20 {
            let focused = handle
                .get_webview_window("main")
                .and_then(|w| w.is_focused().ok())
                .unwrap_or(false);
            if focused {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let focus_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = focus_handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(400));

        let started = std::time::Instant::now();
        let choice = handle
            .dialog()
            .message(
                "检测到本程序已经有一个在运行。\n\n\
                 「用新的」：先关闭正在运行的那个（它会断开连接），再用本次启动。\n\
                 「用旧的」：本次不启动，直接打开正在运行的那个。",
            )
            .title("本程序已在运行")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "用新的".to_string(),
                "用旧的".to_string(),
            ))
            .blocking_show_with_result();
        let elapsed = started.elapsed();

        if should_yield_to_running(&choice, elapsed) {
            // 用旧的：把正在运行的那个调到前台，本次退出
            instance::ask_show();
            handle.exit(0);
            return;
        }

        // 用新的：先礼后兵——请旧实例优雅退出（它会回收 ssh 子进程），无响应再强杀兜底
        let quit_ok = instance::ask_quit();
        let free_after_quit = instance::wait_until_free(instance::RELEASE_TIMEOUT);
        log::info!("请旧实例退出：已响应={quit_ok} 已释放={free_after_quit}");
        if !quit_ok || !free_after_quit {
            match instance::holder_pid() {
                Some(pid) => {
                    log::warn!("旧实例未响应退出请求，强制结束进程 {pid}");
                    instance::force_kill(pid);
                }
                None => log::warn!("旧实例未响应退出请求，且无法定位其进程"),
            }
        }
        if !instance::wait_until_free(std::time::Duration::from_secs(5)) {
            log::warn!("接管失败：实例锁端口仍被占用");
            // 接管不了就本次不启动，绝不允许两个实例同时抢隧道
            handle
                .dialog()
                .message("无法关闭正在运行的实例，请手动退出它后再试。")
                .title("启动失败")
                .kind(MessageDialogKind::Error)
                .blocking_show();
            handle.exit(0);
            return;
        }
        log::info!("已接管，开始启动");
        let boot_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || boot(&boot_handle, start_hidden));
    });
}

/// 用户是否选了「用新的」。
///
/// ⚠️ 用了自定义按钮文案时，弹窗返回的是 `Custom("用新的")` 而不是 `Ok`——
/// 早先按 `Ok` 判断，导致点「用新的」永远被当成「用旧的」，新实例静默退出。
fn chose_new_instance(choice: &MessageDialogResult) -> bool {
    match choice {
        MessageDialogResult::Custom(label) => label == "用新的",
        MessageDialogResult::Ok | MessageDialogResult::Yes => true,
        _ => false,
    }
}

/// 本次是否应当让位给正在运行的实例（本次不启动）。
///
/// 除了用户明确选「用旧的」之外，还兜住一种更危险的偶发情况：弹窗在窗口尚未获焦时
/// 「没等用户点就返回默认按钮」（实测偶发）。人类不可能在几百毫秒内点完，所以把
/// 极短耗时一律当成「没真正弹给用户看」，按最安全的「用旧的」处理——
/// 宁可本次不启动，也绝不静默杀掉用户正在使用的那个实例。
fn should_yield_to_running(choice: &MessageDialogResult, elapsed: std::time::Duration) -> bool {
    const MIN_HUMAN_CHOICE: std::time::Duration = std::time::Duration::from_millis(500);
    if elapsed < MIN_HUMAN_CHOICE {
        log::warn!("弹窗疑似未能正常展示（{elapsed:?}），按「用旧的」处理");
        return true;
    }
    !chose_new_instance(choice)
}

/// 正式启动流程：清理残留隧道 → 持有实例锁 → 建托盘 → 起隧道与探活 → 显示/隐藏窗口。
///
/// 只调用一次，且在单实例闸门之后。
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

    if let Err(e) = instance::acquire(app.clone()) {
        log::warn!("{e}");
    }

    let initial = state::current_status(&shared);
    app.manage(tray::TrayState::new());
    if let Err(e) = tray::build_tray(app, &initial, cfg.auto_start, cfg.auto_connect) {
        log::error!("托盘初始化失败：{e}");
    }

    let handle = app.clone();
    let tunnel_state = shared.clone();
    tauri::async_runtime::spawn(async move {
        tunnel::tunnel_loop(handle, tunnel_state).await;
    });

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 自定义按钮文案时必须按文案判断，不能按 Ok（真实踩过的坑：点「用新的」被当成「用旧的」）
    #[test]
    fn chosen_button_maps_to_intent() {
        assert!(chose_new_instance(&MessageDialogResult::Custom("用新的".into())));
        assert!(!chose_new_instance(&MessageDialogResult::Custom("用旧的".into())));
        assert!(chose_new_instance(&MessageDialogResult::Ok));
        assert!(chose_new_instance(&MessageDialogResult::Yes));
        // 关闭弹窗 / 明确取消 / 文案对不上，一律按「不接管」处理，避免误关正在运行的实例
        assert!(!chose_new_instance(&MessageDialogResult::Cancel));
        assert!(!chose_new_instance(&MessageDialogResult::No));
        assert!(!chose_new_instance(&MessageDialogResult::Custom("别的".into())));
    }

    /// 让位判定：用户选旧的 → 让位；选新的且耗时正常 → 接管；耗时过短（弹窗没展示）→ 让位
    #[test]
    fn yield_decision_prefers_safety() {
        use std::time::Duration;
        let slow = Duration::from_secs(3);
        let instant = Duration::from_millis(20);
        let new = MessageDialogResult::Custom("用新的".into());
        let old = MessageDialogResult::Custom("用旧的".into());

        assert!(should_yield_to_running(&old, slow));
        assert!(!should_yield_to_running(&new, slow));
        // 秒退（无论返回什么）一律让位，绝不静默杀掉正在运行的实例
        assert!(should_yield_to_running(&new, instant));
        assert!(should_yield_to_running(&old, instant));
        assert!(should_yield_to_running(&MessageDialogResult::Ok, instant));
    }
}
