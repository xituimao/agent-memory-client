//! 测试辅助：极简本地 HTTP 服务器，供探活/统计的集成测试使用。
//! 仅 #[cfg(test)] 编译。

use std::io::{Read, Write};
use std::net::TcpListener;

/// 启动一个极简 HTTP 服务，返回端口。
/// handler 收到请求路径（含 query string），返回 (status_code, body)。
/// 每连接响应一次即关闭（connection: close），可服务任意多次请求。
pub fn spawn_server(handler: impl Fn(&str) -> (u16, String) + Send + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 8192];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let (code, body) = handler(&path);
            let reason = match code {
                200 => "OK",
                401 => "Unauthorized",
                403 => "Forbidden",
                _ => "Error",
            };
            let resp = format!(
                "HTTP/1.1 {code} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    port
}

/// 找到一个当前未被占用的端口（bind 后立即释放）
pub fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
