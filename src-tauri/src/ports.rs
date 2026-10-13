//! 残留隧道清理：上次异常退出（崩溃、被强杀、休眠后被杀）可能留下 `ssh -N` 子进程
//! 继续监听着转发端口，新实例的三条转发会全部报 `Address already in use`，
//! 用户看到的现象就是「一直连不上」。启动时先把这类自己的残留进程收掉。
//!
//! 判定刻意从严，两条都必须成立：
//! 1. **是自己人**：命令行同时具备本项目 `ssh_args()` 的三个特征
//!    （`-N` 参数、`ExitOnForwardFailure=yes`、三条 `<端口>:localhost:<端口>` 转发），
//!    否则用户自己手工建的 ssh 隧道会被误杀；
//! 2. **是孤儿**：父进程已消失（被 launchd 收养，ppid ≤ 1）。
//!    父进程还活着的隧道属于某个正在运行的实例——那是单实例闸门该管的事，这里绝不碰。

use crate::config::AppConfig;

/// 本程序遗留的孤儿 ssh 转发进程（PID + 完整命令行）。
/// 只查不杀，便于单测与人工核对。
pub fn find_stale_forwards(cfg: &AppConfig) -> Vec<(u32, String)> {
    #[cfg(unix)]
    {
        let Ok(out) = std::process::Command::new("ps")
            .args(["-ax", "-o", "pid=,command="])
            .output()
        else {
            return Vec::new();
        };
        let me = std::process::id();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, cmd) = line.trim_start().split_once(' ')?;
                let pid: u32 = pid.parse().ok()?;
                if pid == me || !is_our_forward(cmd, cfg) || !is_orphan(pid) {
                    return None;
                }
                Some((pid, cmd.trim().to_string()))
            })
            .collect()
    }
    #[cfg(not(unix))]
    {
        let _ = cfg;
        Vec::new()
    }
}

/// 父进程已消失（被 launchd 收养）的进程才算「残留」
#[cfg(unix)]
fn is_orphan(pid: u32) -> bool {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u32>()
        .map(|ppid| ppid <= 1)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_orphan(_pid: u32) -> bool {
    false
}

/// 判定一条命令行是否为本项目生成的 ssh 转发：
/// `-N` + `ExitOnForwardFailure=yes` + 三条 `<端口>:localhost:<端口>` 必须同时成立。
fn is_our_forward(cmd: &str, cfg: &AppConfig) -> bool {
    let ports = [cfg.rest_port, cfg.viewer_port, cfg.streams_port()];
    cmd.split_whitespace().any(|t| t == "-N")
        && cmd.split_whitespace().next().is_some_and(|exe| {
            std::path::Path::new(exe)
                .file_name()
                .is_some_and(|n| n == "ssh")
        })
        && cmd.contains("ExitOnForwardFailure=yes")
        && ports
            .iter()
            .all(|p| cmd.contains(&format!("{p}:localhost:{p}")))
}

/// 清理本项目遗留的 ssh 转发进程，返回清理数量。
///
/// ⚠️ 只应在 `tunnel_loop` 启动之前调用一次：隧道起来之后再调会把本进程的 ssh 当成残留杀掉。
pub fn cleanup_stale_forwards(cfg: &AppConfig) -> usize {
    let stale = find_stale_forwards(cfg);
    if stale.is_empty() {
        return 0;
    }
    for (pid, cmd) in &stale {
        log::warn!("清理残留隧道进程 {pid}：{cmd}");
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }
    // 给 SIGTERM 一点时间，仍存活说明它没响应（卡死），再强杀
    std::thread::sleep(std::time::Duration::from_millis(500));
    for (pid, _) in &stale {
        if still_alive(*pid) {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
        }
    }
    stale.len()
}

/// 三个转发端口里当前无法绑定的（即被占用的），用于提示用户。
/// 清理过自家残留之后仍有占用 = 被别的程序占了。
pub fn occupied_ports(cfg: &AppConfig) -> Vec<u16> {
    [cfg.rest_port, cfg.viewer_port, cfg.streams_port()]
        .into_iter()
        .filter(|p| !bindable(*p))
        .collect()
}

/// 本地回环端口当前是否可绑定
fn bindable(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

#[cfg(unix)]
fn still_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn still_alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AppConfig {
        AppConfig {
            host: "example.com".into(),
            ssh_user: "root".into(),
            rest_port: 3111,
            viewer_port: 3113,
            ..Default::default()
        }
    }

    /// 本项目自己生成的命令行长什么样（与 config::ssh_args 保持一致）
    fn our_cmd() -> String {
        let c = cfg();
        format!(
            "/usr/bin/ssh {} root@example.com",
            c.ssh_args().join(" ")
        )
    }

    #[test]
    fn recognizes_our_own_tunnel() {
        assert!(is_our_forward(&our_cmd(), &cfg()));
    }

    #[test]
    fn does_not_touch_user_handmade_tunnels() {
        // 用户手工建的隧道：只有一条转发，且没有 ExitOnForwardFailure → 不能杀
        assert!(!is_our_forward(
            "ssh -N -L 3111:localhost:3111 root@example.com",
            &cfg()
        ));
        // 只差一条转发
        assert!(!is_our_forward(
            "ssh -N -o ExitOnForwardFailure=yes -L 3111:localhost:3111 -L 3113:localhost:3113 root@example.com",
            &cfg()
        ));
        // 三条转发都对，但可执行文件不是 ssh
        assert!(!is_our_forward(
            "autossh -N -o ExitOnForwardFailure=yes -L 3111:localhost:3111 -L 3112:localhost:3112 -L 3113:localhost:3113 root@example.com",
            &cfg()
        ));
        // 无关进程
        assert!(!is_our_forward("/usr/bin/python3 server.py", &cfg()));
    }

    #[test]
    fn stream_port_is_viewer_minus_one() {
        // 3112 = viewer 3113 - 1，命令行里必须出现这一条才判定为自己人
        assert!(our_cmd().contains("3112:localhost:3112"));
        assert_eq!(cfg().streams_port(), 3112);
    }

    /// 真实环境只读核对：把本机被认作「自家残留」的命令行列出来（不杀）
    #[test]
    fn find_is_readonly_and_returns_pairs() {
        for (pid, cmd) in find_stale_forwards(&cfg()) {
            assert!(pid > 0);
            assert!(cmd.contains("ssh"));
        }
    }

    /// 活着的进程（父进程健在）不能被当成残留孤儿
    #[test]
    fn live_process_is_not_orphan() {
        assert!(!is_orphan(std::process::id()));
        // 不存在的 PID 也不该被判成孤儿（ps 无输出）
        assert!(!is_orphan(999_999));
    }

    /// 临时验证（跑完即删）：真实环境里能否认出刚造出的孤儿隧道
    #[test]
    fn tmp_find_real_orphan() {
        let found = find_stale_forwards(&cfg());
        println!("找到 {} 个候选", found.len());
        for (pid, cmd) in found {
            println!("  {pid} {}", &cmd[..cmd.len().min(90)]);
        }
    }

    /// 端口占用判定：占住即不可绑，放开即可绑
    #[test]
    fn bindable_follows_occupancy() {
        let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = holder.local_addr().unwrap().port();
        assert!(!bindable(port));
        drop(holder);
        assert!(bindable(port));
    }
}
