use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::AppHandle;
use tokio::process::Command;

use crate::state::{self, SharedState, Status};

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// 隧道管理主循环：按当前配置反复拉起 `ssh -N`，断线指数退避重连。
/// 配置保存后调用方 bump generation 并 notify，本循环随即杀掉旧进程、用新配置重启。
pub async fn tunnel_loop(app: AppHandle, state: SharedState) {
    let mut backoff = MIN_BACKOFF;
    loop {
        let config = state.get_config();

        if !config.is_valid() || !config.auto_connect {
            state.ssh_alive.store(false, Ordering::SeqCst);
            state::set_status(&app, &state, Status::NotConfigured);
            // 等待配置变更或重连开关打开
            state.tunnel_notify.notified().await;
            backoff = MIN_BACKOFF;
            continue;
        }

        let generation = state::current_generation(&state);
        state.ssh_alive.store(false, Ordering::SeqCst);
        state::set_status(&app, &state, Status::Connecting);

        let mut child = match Command::new("ssh")
            .args(config.ssh_args())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                state::set_status(
                    &app,
                    &state,
                    Status::Offline {
                        detail: format!("无法启动 ssh: {e}"),
                    },
                );
                wait_or_notify(&state, backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };

        let pid = child.id();
        {
            let mut slot = state.child.lock().expect("child lock poisoned");
            *slot = Some(child);
        }
        state.ssh_alive.store(true, Ordering::SeqCst);
        log::info!("ssh tunnel started (pid {pid})");

        // 等待三种情况：子进程退出、配置变更（generation 变化）、显式 kill
        wait_child_or_signal(&state, generation).await;

        state.ssh_alive.store(false, Ordering::SeqCst);
        {
            let mut slot = state.child.lock().expect("child lock poisoned");
            if let Some(mut c) = slot.take() {
                let _ = c.kill().await;
            }
        }
        log::info!("ssh tunnel stopped (pid {pid})");

        if state::current_generation(&state) == generation {
            // 自然断开：标记离线并退避重试
            state::set_status(
                &app,
                &state,
                Status::Offline {
                    detail: "隧道已断开，重试中".to_string(),
                },
            );
            wait_or_notify(&state, backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        } else {
            // 配置变更触发的重启：立即用新配置重来，并消费掉唤醒信号
            backoff = MIN_BACKOFF;
            wait_or_notify(&state, Duration::ZERO).await;
        }
    }
}

/// 等待子进程结束或 generation 变化（配置保存触发的重启）
async fn wait_child_or_signal(state: &SharedState, generation: u64) {
    loop {
        // 每 200ms 轮询一次，兼顾响应速度与健康检查竞态
        tokio::time::sleep(Duration::from_millis(200)).await;
        if state::current_generation(state) != generation {
            return;
        }
        let exited = {
            let mut slot = state.child.lock().expect("child lock poisoned");
            match slot.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(Some(_))),
                None => true,
            }
        };
        if exited {
            return;
        }
    }
}

async fn wait_or_notify(state: &SharedState, duration: Duration) {
    tokio::select! {
        _ = state.tunnel_notify.notified() => {}
        _ = tokio::time::sleep(duration) => {}
    }
}
