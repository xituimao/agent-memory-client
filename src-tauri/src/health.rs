use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::state::{self, SharedState, Status};

const POLL_INTERVAL: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Online 状态事件的最小发送间隔，避免每 3s 抖动托盘图标与事件
const ONLINE_EMIT_INTERVAL: Duration = Duration::from_secs(10);
/// 今日新增记忆统计的轮询间隔
const STATS_INTERVAL: Duration = Duration::from_secs(60);

/// 周期探活：配置齐全 + ssh 子进程存活 + REST 端口 HTTP 可达 ⇒ Online。
/// 任何 2xx/3xx/404 等响应都算服务可达——端口通则服务在；
/// 401/403 单独识别为「token 无效」，避免错误 token 显示绿灯误导用户。
pub async fn health_loop(app: AppHandle, state: SharedState) {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("reqwest client");
    let mut fail_streak: u32 = 0;
    let mut last_generation = state::current_generation(&state);
    let mut last_online_emit = Instant::now() - ONLINE_EMIT_INTERVAL;
    let mut last_stats_poll = Instant::now() - STATS_INTERVAL;

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        // 隧道每次重建 generation 都会变：借此清零上一个隧道的失败计数
        let generation = state::current_generation(&state);
        if generation != last_generation {
            fail_streak = 0;
            last_generation = generation;
        }

        let config = state::get_config(&state);
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
                // 已在线时按节流间隔刷新延迟，避免事件/托盘图标抖动
                let current = state::current_status(&state);
                if !matches!(current, Status::Online { .. })
                    || last_online_emit.elapsed() >= ONLINE_EMIT_INTERVAL
                {
                    last_online_emit = Instant::now();
                    state::set_status(&app, &state, Status::Online { latency_ms: latency });
                }
                // 每 60s 统计今日新增记忆并推送给前端
                if last_stats_poll.elapsed() >= STATS_INTERVAL {
                    last_stats_poll = Instant::now();
                    if let Ok(stats) = crate::stats::count_today(&client, &config).await {
                        let _ = app.emit("stats-changed", &stats);
                    }
                }
            }
            Err(detail) => {
                fail_streak += 1;
                // 刚拉起隧道时服务可能还没就绪，容忍前 2 次失败（约 6s 宽限）
                let current = state::current_status(&state);
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
    let status = resp.status();
    let _ = resp.text().await;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err("Token 无效（认证失败）".to_string());
    }
    Ok(started.elapsed().as_millis() as u64)
}
