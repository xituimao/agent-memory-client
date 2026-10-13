use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
use std::sync::Mutex;

use tauri::Emitter;

use crate::config::{config_path, AppConfig};

/// 连接状态机
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Status {
    /// 配置不全（缺 host 或 token），等待用户在设置窗口补全
    NotConfigured,
    /// 隧道进程已拉起，等待服务探活通过
    Connecting,
    /// 隧道 + HTTP 探活均正常
    Online { latency_ms: u64 },
    /// 隧道断开或探活失败，自动重试中
    Offline { detail: String },
    /// 用户手动断开（VPN 式按钮），不会自动重连，直到用户再次连接
    Disconnected,
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::NotConfigured => "未配置".to_string(),
            Status::Connecting => "连接中…".to_string(),
            Status::Online { latency_ms } => format!("已连接 · {} ms", latency_ms),
            Status::Offline { detail } => format!("未连接 · {}", detail),
            Status::Disconnected => "已断开".to_string(),
        }
    }
}

pub struct Inner {
    pub config: Mutex<AppConfig>,
    pub config_path: PathBuf,
    pub status: Mutex<Status>,
    /// 每次重启隧道自增；旧隧道循环发现 generation 过期即退出
    pub tunnel_generation: AtomicU64,
    /// 隧道 ssh 子进程是否存活（由 tunnel 循环维护）
    pub ssh_alive: AtomicBool,
    /// 手动断开标志：置位后不自动建隧道，直到用户点「连接」
    pub manual_disconnect: AtomicBool,
    /// 隧道循环的唤醒信号（配置保存后触发重启）
    pub tunnel_notify: tokio::sync::Notify,
    /// 当前 ssh 子进程（用于强制终止）
    pub child: Mutex<Option<tokio::process::Child>>,
    /// 待用户确认接管的旧实例数量（0 = 无需确认）。>0 时不抢隧道锁、不建隧道，
    /// 界面显示「本程序已经在运行」的确认提示，用户确认后再接管。
    pub pending_takeover: AtomicU32,
}

pub type SharedState = Arc<Inner>;

use std::sync::Arc;

pub fn new(app_config_dir: &std::path::Path) -> SharedState {
    let path = config_path(app_config_dir);
    let config = crate::config::load(&path);
    let auto_connect = config.auto_connect;
    Arc::new(Inner {
        config: Mutex::new(config),
        config_path: path,
        status: Mutex::new(Status::NotConfigured),
        tunnel_generation: AtomicU64::new(0),
        ssh_alive: AtomicBool::new(false),
        manual_disconnect: AtomicBool::new(!auto_connect),
        tunnel_notify: tokio::sync::Notify::new(),
        child: Mutex::new(None),
        pending_takeover: AtomicU32::new(0),
    })
}

pub fn get_config(s: &SharedState) -> AppConfig {
    s.config.lock().map(|c| c.clone()).unwrap_or_default()
}

pub fn set_config(s: &SharedState, config: AppConfig) {
    if let Ok(mut guard) = s.config.lock() {
        *guard = config;
    }
}

pub fn current_status(s: &SharedState) -> Status {
    s.status
        .lock()
        .map(|s| s.clone())
        .unwrap_or(Status::NotConfigured)
}

/// 更新状态；仅在变化时向窗口发事件并刷新托盘
pub fn set_status(app: &tauri::AppHandle, state: &SharedState, status: Status) {
    let changed = {
        let mut guard = match state.status.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let changed = *guard != status;
        if changed {
            *guard = status.clone();
        }
        changed
    };
    if changed {
        let _ = app.emit("status-changed", &status);
        crate::tray::refresh_tray(app, &status);
    }
}

pub fn bump_generation(state: &SharedState) -> u64 {
    state
        .tunnel_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1
}

pub fn current_generation(state: &SharedState) -> u64 {
    state.tunnel_generation.load(std::sync::atomic::Ordering::SeqCst)
}


/// 待确认接管的旧实例数量（0 = 无需确认）
pub fn pending_takeover(s: &SharedState) -> u32 {
    s.pending_takeover.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn set_pending_takeover(s: &SharedState, running: u32) {
    s.pending_takeover
        .store(running, std::sync::atomic::Ordering::SeqCst);
}
