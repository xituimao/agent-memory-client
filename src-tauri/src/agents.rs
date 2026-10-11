use serde::Serialize;
use std::path::PathBuf;

/// 受支持的 agent 客户端（与官方 `agentmemory connect` 支持的列表一致）
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 本机是否检测到已安装
    pub installed: bool,
}

struct AgentDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    /// 判断是否已安装：配置目录 or PATH 中的可执行文件
    dir: &'static str,
    cmd: &'static str,
    /// macOS 应用安装路径（可选）
    app: Option<&'static str>,
}

const AGENTS: &[AgentDef] = &[
    AgentDef { id: "claude-code", name: "Claude Code", description: "Anthropic 官方 CLI", dir: ".claude", cmd: "claude", app: None },
    AgentDef { id: "codex", name: "Codex CLI", description: "OpenAI 终端编码助手", dir: ".codex", cmd: "codex", app: None },
    AgentDef { id: "gemini-cli", name: "Gemini CLI", description: "Google 终端 AI 助手", dir: ".gemini", cmd: "gemini", app: None },
    AgentDef { id: "qwen", name: "Qwen Code", description: "通义千问终端编码助手", dir: ".qwen", cmd: "qwen", app: None },
    AgentDef { id: "opencode", name: "OpenCode", description: "开源终端 AI 编码工具", dir: ".config/opencode", cmd: "opencode", app: None },
    AgentDef { id: "cursor", name: "Cursor", description: "AI 代码编辑器", dir: ".cursor", cmd: "cursor", app: Some("/Applications/Cursor.app") },
];

/// 扫描本机已安装的 agent 客户端
pub fn scan() -> Vec<AgentInfo> {
    let home = home_dir();
    AGENTS
        .iter()
        .map(|def| AgentInfo {
            id: def.id.to_string(),
            name: def.name.to_string(),
            description: def.description.to_string(),
            installed: is_installed(def, &home),
        })
        .collect()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn is_installed(def: &AgentDef, home: &Option<PathBuf>) -> bool {
    if let Some(home) = home {
        if home.join(def.dir).exists() {
            return true;
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(app) = def.app {
        if PathBuf::from(app).exists() {
            return true;
        }
    }
    in_path(def.cmd)
}

fn in_path(cmd: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&paths).any(|dir| {
        let exe = dir.join(cmd);
        exe.is_file()
            || exe.with_extension("exe").is_file()
            || exe.with_extension("cmd").is_file()
    })
}

/// 校验 id 合法（防止注入到 npx 命令）
pub fn is_supported(id: &str) -> bool {
    AGENTS.iter().any(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_returns_all_supported_agents() {
        let list = scan();
        assert_eq!(list.len(), AGENTS.len());
        assert!(list.iter().any(|a| a.id == "claude-code"));
        // 本机 macOS 上至少能检测到 PATH 里的内容，字段齐全
        for a in &list {
            assert!(!a.name.is_empty());
            assert!(!a.description.is_empty());
        }
    }

    #[test]
    fn supported_id_validation() {
        assert!(is_supported("claude-code"));
        assert!(is_supported("cursor"));
        assert!(!is_supported("rm -rf"));
        assert!(!is_supported(""));
        assert!(!is_supported("claude-code; evil"));
    }
}
