//! 单实例协商：用一个固定的本地回环端口同时充当「实例锁」与「进程间通道」。
//!
//! 为什么不用 `tauri-plugin-single-instance`：本项目要的不只是「拦住第二个实例」，
//! 还要能对旧实例下达两条指令——「把窗口调到前台」「优雅退出（先回收 ssh 子进程）」，
//! 并且新实例要在旧实例退出后干净接管隧道。一个监听回环端口的极简文本协议就够，
//! 不必为此新增依赖。
//!
//! 协议：客户端连上后发一行命令，服务端回一行结果。
//! - `ping` → `pong`：确认「这个端口后面确实是本程序」，避免把占用同端口的别的软件误判成自己的实例
//! - `show` → `ok`：旧实例把窗口调到前台
//! - `quit` → `ok`：旧实例优雅退出（走 `RunEvent::Exit` 回收 ssh 子进程）

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use tauri::AppHandle;

/// 充当实例锁的回环端口。改动它会导致新旧版本互不识别（旧版不认识新版 → 会并行运行），非必要不要改。
const LOCK_PORT: u16 = 39731;
/// 单次请求超时：够本地回环往返，又不至于让启动卡住
const PROTO_TIMEOUT: Duration = Duration::from_millis(800);
/// 旧实例退出后，等它回收 ssh 子进程并释放锁端口的上限
pub const RELEASE_TIMEOUT: Duration = Duration::from_secs(10);

fn lock_addr() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, LOCK_PORT))
}

/// 客户端：连接旧实例、发一条命令、读一行响应（任何一步失败都返回 None）
fn request(cmd: &str) -> Option<String> {
    let mut stream = TcpStream::connect_timeout(&lock_addr(), PROTO_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(PROTO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(PROTO_TIMEOUT)).ok()?;
    stream.write_all(format!("{cmd}\n").as_bytes()).ok()?;
    stream.flush().ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let line = line.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// 是否已有本程序实例在运行。只有收到 `pong` 才算——
/// 端口被别的软件占用时不算，否则会误判成「已有实例」。
pub fn existing() -> bool {
    request("ping").as_deref() == Some("pong")
}

/// 请旧实例优雅退出（接管前先礼后兵）。返回它是否作了响应。
pub fn ask_quit() -> bool {
    request("quit").is_some()
}

/// 等锁端口重新可绑 = 旧实例已把隧道和锁都放开。返回是否等到。
pub fn wait_until_free(timeout: Duration) -> bool {
    wait_port_free(lock_addr(), timeout)
}

/// 轮询等待指定地址可绑定（绑定成功后立即释放）
fn wait_port_free(addr: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if TcpListener::bind(addr).is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// 本程序的可执行文件名——`.app` 与开发用的 `target/debug` 两种形态都叫这个，
/// 按文件名精确匹配，绝不会误伤别的软件。
const APP_BIN: &str = "agent-memory-client";

/// 本机正在运行的**其它**本程序实例（PID + 可执行路径），不含当前进程。
/// 非 unix 平台返回空（届时只靠锁端口检测，见 `existing`）。
pub fn other_instances() -> Vec<(u32, String)> {
    #[cfg(unix)]
    {
        // 用 comm 列（纯可执行路径，不含参数）：命令行拼接里 .app 路径带空格，
        // 按空格切分取首字段会把正式 .app 实例整个漏掉。
        let Ok(out) = std::process::Command::new("ps")
            .args(["-ax", "-o", "pid=,comm="])
            .output()
        else {
            return Vec::new();
        };
        let me = std::process::id();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, exe) = line.trim_start().split_once(' ')?;
                let pid: u32 = pid.parse().ok()?;
                if pid == me || !is_our_app(exe.trim()) {
                    return None;
                }
                Some((pid, exe.trim().to_string()))
            })
            .collect()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// 可执行路径是否为本程序（文件名精确等于 `agent-memory-client`）
fn is_our_app(exe: &str) -> bool {
    std::path::Path::new(exe.trim())
        .file_name()
        .is_some_and(|name| name == APP_BIN)
}

/// 结束**所有**其它本程序实例：先请锁持有者走正常退出链路（它会回收 ssh 子进程），
/// 再逐个终止剩下的（旧版本、无锁的、卡死的）。返回终止的实例数。
pub fn kill_other_instances() -> usize {
    let targets = other_instances();
    if targets.is_empty() {
        return 0;
    }
    // 1) 锁持有者优先优雅退出：走 app.exit(0) → RunEvent::Exit → 回收 ssh 子进程
    if let Some(pid) = holder_pid()
        && targets.iter().any(|(p, _)| *p == pid)
    {
        ask_quit();
        let _ = wait_until_free(RELEASE_TIMEOUT);
    }
    // 2) 其余实例（含仍没退干净的）逐个终止；重新枚举以跳过已退出的
    let mut killed = 0;
    for (pid, cmd) in other_instances() {
        log::warn!("结束正在运行的实例 {pid}：{cmd}");
        force_kill(pid);
        killed += 1;
    }
    killed
}

/// 占用锁端口的进程 PID，供强杀兜底使用（旧实例卡死、不响应协议时）。
/// 仅 unix 实现；其它平台返回 None，即不强杀。
#[cfg(unix)]
pub fn holder_pid() -> Option<u32> {
    let out = std::process::Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{LOCK_PORT}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok()
}

#[cfg(not(unix))]
pub fn holder_pid() -> Option<u32> {
    None
}

/// 强杀指定进程：先 SIGTERM，400ms 后仍在则 SIGKILL。
/// 返回是否成功发出信号（不代表进程已退出）。
#[cfg(unix)]
pub fn force_kill(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    let sent = std::process::Command::new("kill")
        .arg(&pid_arg)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !sent {
        return false;
    }
    std::thread::sleep(Duration::from_millis(400));
    if alive(pid) {
        let _ = std::process::Command::new("kill").args(["-9", &pid_arg]).status();
    }
    true
}

#[cfg(not(unix))]
pub fn force_kill(_pid: u32) -> bool {
    false
}

/// 进程是否仍存在（`kill -0` 探测）
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 成为主实例：绑定锁端口并起服务线程。
///
/// 只在确认没有别的实例之后调用。端口被非本程序占用时返回 Err——
/// 调用方记录日志后照常启动（不阻断），此时不保证单实例。
pub fn acquire(app: AppHandle) -> Result<(), String> {
    let listener = TcpListener::bind(lock_addr()).map_err(|e| {
        format!("实例锁端口 {LOCK_PORT} 绑定失败：{e}（可能被其它程序占用，本次无法保证单实例）")
    })?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(stream, &app);
        }
    });
    Ok(())
}

/// 处理一条来自新实例的请求：先回包，再执行副作用
fn handle(stream: TcpStream, app: &AppHandle) {
    let Ok(read_half) = stream.try_clone() else { return };
    let mut cmd = String::new();
    let _ = BufReader::new(read_half).read_line(&mut cmd);
    let (reply, action) = dispatch(cmd.trim());

    let mut out = stream;
    let _ = out.write_all(format!("{reply}\n").as_bytes());
    let _ = out.flush();

    match action {
        Action::Show => {
            crate::tray::show_window(app);
        }
        Action::Quit => app.exit(0),
        Action::None => {}
    }
}

/// 命令的副作用类型
#[derive(Debug, PartialEq)]
enum Action {
    None,
    Show,
    Quit,
}

/// 命令 → (响应文本, 副作用)。纯函数，便于测试。
fn dispatch(cmd: &str) -> (&'static str, Action) {
    match cmd {
        "ping" => ("pong", Action::None),
        "show" => ("ok", Action::Show),
        "quit" => ("ok", Action::Quit),
        _ => ("unknown", Action::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_maps_known_commands() {
        assert_eq!(dispatch("ping"), ("pong", Action::None));
        assert_eq!(dispatch("show"), ("ok", Action::Show));
        assert_eq!(dispatch("quit"), ("ok", Action::Quit));
        // 未知命令必须回 unknown 且不产生任何副作用
        assert_eq!(dispatch("rm -rf /"), ("unknown", Action::None));
        assert_eq!(dispatch(""), ("unknown", Action::None));
        // 命令要整行精确匹配，前缀相同不算
        assert_eq!(dispatch("quit now"), ("unknown", Action::None));
    }

    /// 与真实服务端线程对打：起一个用 dispatch 驱动的监听端，验证客户端收发
    #[test]
    fn client_server_roundtrip_over_loopback() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                let mut cmd = String::new();
                let Ok(read_half) = stream.try_clone() else { continue };
                let _ = BufReader::new(read_half).read_line(&mut cmd);
                let (reply, _) = dispatch(cmd.trim());
                let mut out = stream;
                let _ = out.write_all(format!("{reply}\n").as_bytes());
            }
        });

        let mut client = TcpStream::connect_timeout(&addr, PROTO_TIMEOUT).unwrap();
        client.write_all(b"ping\n").unwrap();
        let mut line = String::new();
        BufReader::new(client.try_clone().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "pong");

        let mut client = TcpStream::connect_timeout(&addr, PROTO_TIMEOUT).unwrap();
        client.write_all(b"quit\n").unwrap();
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ok");
        server.join().unwrap();
    }

    /// 等待释放：被占用时必须超时返回 false，释放后立刻返回 true
    #[test]
    fn wait_port_free_follows_occupancy() {
        let holder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = holder.local_addr().unwrap();
        assert!(!wait_port_free(addr, Duration::from_millis(300)));
        drop(holder);
        assert!(wait_port_free(addr, Duration::from_millis(300)));
    }

    /// 「所有进程都干掉」的第一步是认对人：只认可执行文件名精确等于 agent-memory-client
    #[test]
    fn recognizes_our_own_processes_only() {
        // 正式 .app 的路径带空格，必须能认出来（早期按空格取首字段时这里漏掉了）
        assert!(is_our_app(
            "/Applications/Agent Memory.app/Contents/MacOS/agent-memory-client"
        ));
        assert!(is_our_app(
            "/Users/x/Codes/agent-memory-client/src-tauri/target/debug/agent-memory-client"
        ));
        assert!(is_our_app("agent-memory-client"));

        // 名字相近的、别的软件、shell 包装，一律不能杀
        assert!(!is_our_app("agent-memory-client-helper"));
        assert!(!is_our_app("/usr/bin/ssh"));
        assert!(!is_our_app("/bin/zsh"));
        assert!(!is_our_app("node"));
        assert!(!is_our_app(""));
    }

    /// 当前进程不能被算作「其它实例」（否则自己杀自己）
    #[test]
    fn other_instances_excludes_self() {
        let me = std::process::id();
        assert!(other_instances().iter().all(|(pid, _)| *pid != me));
    }
}
