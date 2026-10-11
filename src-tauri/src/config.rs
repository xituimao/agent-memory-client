use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_HOST: &str = "your-server-ip";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// 服务器地址（SSH HostName）
    pub host: String,
    /// SSH 端口
    pub ssh_port: u16,
    /// SSH 用户
    pub ssh_user: String,
    /// SSH 私钥路径，空字符串表示使用默认密钥（ssh-agent / ~/.ssh/id_*）
    pub ssh_key_path: String,
    /// 本地 REST 转发端口（对应服务器 3111）
    pub rest_port: u16,
    /// 本地 Viewer 转发端口（对应服务器 3113）
    pub viewer_port: u16,
    /// Bearer token（AGENTMEMORY_SECRET）
    pub token: String,
    /// 附加 ssh 参数（空格分隔，原样追加）
    pub extra_ssh_args: String,
    /// 应用启动时自动建立隧道
    pub auto_connect: bool,
    /// 登录系统时自动启动本应用
    pub auto_start: bool,
    /// 目标 agent 名称（用于生成 MCP 接入命令）
    pub agent_name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_string(),
            ssh_port: 22,
            ssh_user: "root".to_string(),
            ssh_key_path: String::new(),
            rest_port: 3111,
            viewer_port: 3113,
            token: String::new(),
            extra_ssh_args: String::new(),
            auto_connect: true,
            auto_start: false,
            agent_name: "claude-code".to_string(),
        }
    }
}

impl AppConfig {
    /// 建立隧道所需的最小配置是否齐全
    pub fn is_valid(&self) -> bool {
        !self.host.trim().is_empty() && !self.token.trim().is_empty()
    }

    pub fn viewer_url(&self) -> String {
        format!("http://localhost:{}", self.viewer_port)
    }

    pub fn rest_url(&self) -> String {
        format!("http://localhost:{}", self.rest_port)
    }

    /// 文档中的 MCP 接入命令（供一键复制）
    pub fn mcp_command(&self) -> String {
        format!(
            "export AGENTMEMORY_URL={}\nexport AGENTMEMORY_SECRET={}\nnpx -y @agentmemory/agentmemory@latest connect {}",
            self.rest_url(),
            self.token,
            self.agent_name
        )
    }

    /// 构造 `ssh -N` 隧道命令参数
    pub fn ssh_args(&self) -> Vec<String> {
        let mut args = vec![
            "-N".to_string(),
            "-T".to_string(),
            "-o".to_string(),
            "ExitOnForwardFailure=yes".to_string(),
            "-o".to_string(),
            "ServerAliveInterval=30".to_string(),
            "-o".to_string(),
            "ServerAliveCountMax=3".to_string(),
            // 禁止交互式密码提示，避免子进程挂死等待输入
            "-o".to_string(),
            "BatchMode=yes".to_string(),
        ];
        if !self.ssh_key_path.trim().is_empty() {
            args.push("-i".to_string());
            args.push(self.ssh_key_path.trim().to_string());
        }
        args.push("-p".to_string());
        args.push(self.ssh_port.to_string());
        args.push("-L".to_string());
        args.push(format!("{}:localhost:{}", self.rest_port, self.rest_port));
        args.push("-L".to_string());
        args.push(format!("{}:localhost:{}", self.viewer_port, self.viewer_port));
        // viewer 的实时数据走 WebSocket，端口默认为 viewer 端口 - 1（即文档中的 3112 streams）
        let streams_port = self.viewer_port.saturating_sub(1);
        args.push("-L".to_string());
        args.push(format!("{}:localhost:{}", streams_port, streams_port));
        if !self.extra_ssh_args.trim().is_empty() {
            args.extend(self.extra_ssh_args.split_whitespace().map(str::to_string));
        }
        args.push(format!(
            "{}@{}",
            self.ssh_user.trim(),
            self.host.trim()
        ));
        args
    }
}

pub fn config_path(app_config_dir: &Path) -> PathBuf {
    app_config_dir.join("config.json")
}

pub fn load(path: &Path) -> AppConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建配置目录: {e}"))?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("无法写入配置文件: {e}"))?;
    // 尽力收紧权限，避免 token 泄露
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_args_include_forwards_and_auth() {
        let cfg = AppConfig {
            host: "your-server-ip".into(),
            ssh_port: 2222,
            ssh_user: "root".into(),
            ssh_key_path: "/tmp/key".into(),
            rest_port: 3111,
            viewer_port: 3113,
            ..Default::default()
        };
        let args = cfg.ssh_args();
        let joined = args.join(" ");
        assert!(joined.contains("-N"));
        assert!(joined.contains("ExitOnForwardFailure=yes"));
        assert!(joined.contains("BatchMode=yes"));
        assert!(args.windows(2).any(|w| w == ["-i", "/tmp/key"]));
        assert!(args.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(args.windows(2).any(|w| w == ["-L", "3111:localhost:3111"]));
        assert!(args.windows(2).any(|w| w == ["-L", "3113:localhost:3113"]));
        assert!(args.windows(2).any(|w| w == ["-L", "3112:localhost:3112"]));
        assert_eq!(args.last().map(String::as_str), Some("root@your-server-ip"));
    }

    #[test]
    fn ssh_args_omit_key_when_empty() {
        let cfg = AppConfig {
            ssh_key_path: String::new(),
            ..Default::default()
        };
        assert!(!cfg.ssh_args().contains(&"-i".to_string()));
    }

    #[test]
    fn ssh_args_append_extra_args() {
        let cfg = AppConfig {
            extra_ssh_args: "-J jump@bastion".into(),
            ..Default::default()
        };
        let args = cfg.ssh_args();
        let pos_j = args.iter().position(|a| a == "-J").unwrap();
        assert_eq!(args[pos_j + 1], "jump@bastion");
        // 附加参数之后紧跟 user@host
        let pos_host = args.iter().position(|a| a == "root@your-server-ip").unwrap();
        assert_eq!(pos_j + 2, pos_host);
    }

    #[test]
    fn validity_requires_host_and_token() {
        assert!(!AppConfig::default().is_valid());
        let mut cfg = AppConfig::default();
        cfg.token = "t".into();
        assert!(cfg.is_valid());
        cfg.host = " ".into();
        assert!(!cfg.is_valid());
    }

    #[test]
    fn mcp_command_contains_url_token_agent() {
        let cfg = AppConfig {
            token: "secret".into(),
            agent_name: "codex".into(),
            ..Default::default()
        };
        let cmd = cfg.mcp_command();
        assert!(cmd.contains("AGENTMEMORY_URL=http://localhost:3111"));
        assert!(cmd.contains("AGENTMEMORY_SECRET=secret"));
        assert!(cmd.contains("connect codex"));
    }

    #[test]
    fn config_roundtrip_preserves_fields() {
        let dir = std::env::temp_dir().join("amc-test-config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let cfg = AppConfig {
            token: "tok".into(),
            ssh_key_path: "/k".into(),
            auto_start: true,
            ..Default::default()
        };
        save(&path, &cfg).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.token, "tok");
        assert_eq!(loaded.ssh_key_path, "/k");
        assert!(loaded.auto_start);
        assert_eq!(loaded.host, DEFAULT_HOST);
    }
}
