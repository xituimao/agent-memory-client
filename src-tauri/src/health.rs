use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::state::{self, SharedState, Status};

const POLL_INTERVAL: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 周期探活：配置齐全 + ssh 子进程存活 + REST 端口 HTTP 可达 ⇒ Online。
/// 任何 HTTP 响应（含 401/404）都算服务可达——端口通则服务在。
pub async fn health_loop(app: AppHandle, state: SharedState) {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("reqwest client");
    let mut fail_streak: u32 = 0;

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        let config = state.get_config();
        if !config.is_valid() {
            fail_streak = 0;
            state::set_status(&app, &state, Status::NotConfigured);
            continue;
        }
        if !state.ssh_alive.load(Ordering::SeqCst) {
            // 隧道循环负责 Connecting / Offline 标记，这里不覆盖
            fail_streak = 0;
            continue;
        }

        match probe(&client, &config).await {
            Ok(latency) => {
                fail_streak = 0;
                state::set_status(&app, &state, Status::Online { latency_ms: latency });
            }
            Err(detail) => {
                fail_streak += 1;
                // 刚拉起隧道时服务可能还没就绪，容忍前 2 次失败（约 6s 宽限）
                let current = state.current_status();
                if fail_streak > 2 || matches!(current, Status::Online { .. }) {
                    state::set_status(&app, &state, Status::Offline { detail });
                }
            }
        }
    }
}

/// 对本地转发的 REST 端口发起一次带 Bearer token 的 GET，返回耗时
pub async fn probe(client: &reqwest::Client, config: &crate::config::AppConfig) -> Result<u64, String> {
    let started = Instant::now();
    let resp = client
        .get(config.rest_url())
        .bearer_auth(config.token.trim())
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                "端口无响应".to_string()
            } else if e.is_timeout() {
                "请求超时".to_string()
            } else {
                format!("探活失败: {e}")
            }
        })?;
    let _ = resp.text().await;
    Ok(started.elapsed().as_millis() as u64)
}
