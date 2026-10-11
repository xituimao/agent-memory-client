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
    /// PATH 中的可执行文件（安装的决定性证据）
    cmd: &'static str,
    /// macOS 应用安装路径（可选）
    app: Option<&'static str>,
    /// VS Code / Cursor 扩展标识（可选，匹配扩展目录名）
    ext: Option<&'static str>,
}

/// 与官方 `agentmemory connect` 支持列表一致（connect --help 实测 21 个）
const AGENTS: &[AgentDef] = &[
    AgentDef { id: "claude-code", name: "Claude Code", description: "Anthropic 官方 CLI", cmd: "claude", app: None, ext: None },
    AgentDef { id: "codex", name: "Codex CLI", description: "OpenAI 终端编码助手", cmd: "codex", app: None, ext: None },
    AgentDef { id: "gemini-cli", name: "Gemini CLI", description: "Google 终端 AI 助手", cmd: "gemini", app: None, ext: None },
    AgentDef { id: "qwen", name: "Qwen Code", description: "通义千问终端编码助手", cmd: "qwen", app: None, ext: None },
    AgentDef { id: "copilot-cli", name: "Copilot CLI", description: "GitHub 终端助手", cmd: "gh", app: None, ext: None },
    AgentDef { id: "cursor", name: "Cursor", description: "AI 代码编辑器", cmd: "cursor", app: Some("/Applications/Cursor.app"), ext: None },
    AgentDef { id: "cline", name: "Cline", description: "VS Code 编码助手", cmd: "", app: None, ext: Some("cline") },
    AgentDef { id: "continue", name: "Continue", description: "开源 IDE 编码助手", cmd: "", app: None, ext: Some("continue") },
    AgentDef { id: "zed", name: "Zed", description: "高性能代码编辑器", cmd: "zed", app: Some("/Applications/Zed.app"), ext: None },
    AgentDef { id: "warp", name: "Warp", description: "AI 终端", cmd: "warp", app: Some("/Applications/Warp.app"), ext: None },
    AgentDef { id: "kiro", name: "Kiro", description: "AWS AI IDE", cmd: "kiro", app: None, ext: None },
    AgentDef { id: "antigravity", name: "Antigravity", description: "Google AI IDE", cmd: "", app: Some("/Applications/Antigravity.app"), ext: None },
    AgentDef { id: "devin", name: "Devin", description: "Cognition AI 工程师", cmd: "devin", app: None, ext: None },
    AgentDef { id: "droid", name: "Droid", description: "Factory 终端工程师", cmd: "droid", app: None, ext: None },
    AgentDef { id: "dsh", name: "DSH", description: "DeepSeek 终端助手", cmd: "dsh", app: None, ext: None },
    AgentDef { id: "opencode", name: "OpenCode", description: "开源终端 AI 编码工具", cmd: "opencode", app: None, ext: None },
    AgentDef { id: "openclaw", name: "OpenClaw", description: "开源 AI 助手框架", cmd: "openclaw", app: None, ext: None },
    AgentDef { id: "hermes", name: "Hermes", description: "Nous Research 助手", cmd: "hermes", app: None, ext: None },
    AgentDef { id: "pi", name: "Pi", description: "Mario Zecher 极简助手", cmd: "pi", app: None, ext: None },
    AgentDef { id: "openhuman", name: "OpenHuman", description: "开源个人 AI", cmd: "openhuman", app: None, ext: None },
];

/// 扫描本机已安装的 agent 客户端；已安装的排前面，未安装的按名称排序垫底
pub fn scan() -> Vec<AgentInfo> {
    let home = home_dir();
    let mut list: Vec<AgentInfo> = AGENTS
        .iter()
        .map(|def| AgentInfo {
            id: def.id.to_string(),
            name: def.name.to_string(),
            description: def.description.to_string(),
            installed: is_installed(def, &home),
        })
        .collect();
    list.sort_by(|a, b| b.installed.cmp(&a.installed).then(a.name.cmp(&b.name)));
    list
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 已安装 = 可执行文件在 PATH / 应用 bundle 存在 / 编辑器扩展存在。
///  deliberately 不用配置目录判断——目录可能是残留数据或被其他软件
/// 捆绑运行时分产生（如 Kimi CLI 内置的 openclaw 运行时），会误报。
fn is_installed(def: &AgentDef, home: &Option<PathBuf>) -> bool {
    #[cfg(target_os = "macos")]
    if let Some(app) = def.app {
        if PathBuf::from(app).exists() {
            return true;
        }
    }
    if let Some(ext) = def.ext {
        if has_vscode_ext(ext, home) {
            return true;
        }
    }
    !def.cmd.is_empty() && in_path(def.cmd)
}

/// VS Code / Cursor 扩展目录中是否存在指定扩展
fn has_vscode_ext(name: &str, home: &Option<PathBuf>) -> bool {
    let Some(home) = home else { return false };
    let roots = [
        home.join(".vscode/extensions"),
        home.join(".cursor/extensions"),
    ];
    roots.iter().any(|root| {
        std::fs::read_dir(root)
            .map(|entries| {
                entries.filter_map(Result::ok).any(|e| {
                    e.file_name().to_string_lossy().to_lowercase().contains(name)
                })
            })
            .unwrap_or(false)
    })
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
