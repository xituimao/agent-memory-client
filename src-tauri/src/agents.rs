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
    /// 次级配置目录（可选，兼容改名/发行版差异）
    dir2: Option<&'static str>,
    /// VS Code / Cursor 扩展标识（可选，匹配扩展目录名）
    ext: Option<&'static str>,
}

/// 与官方 `agentmemory connect` 支持列表一致（connect --help 实测 21 个）
const AGENTS: &[AgentDef] = &[
    AgentDef { id: "claude-code", name: "Claude Code", description: "Anthropic 官方 CLI", dir: ".claude", cmd: "claude", app: None, dir2: None, ext: None },
    AgentDef { id: "codex", name: "Codex CLI", description: "OpenAI 终端编码助手", dir: ".codex", cmd: "codex", app: None, dir2: None, ext: None },
    AgentDef { id: "gemini-cli", name: "Gemini CLI", description: "Google 终端 AI 助手", dir: ".gemini", cmd: "gemini", app: None, dir2: None, ext: None },
    AgentDef { id: "qwen", name: "Qwen Code", description: "通义千问终端编码助手", dir: ".qwen", cmd: "qwen", app: None, dir2: None, ext: None },
    AgentDef { id: "copilot-cli", name: "Copilot CLI", description: "GitHub 终端助手", dir: ".config/gh", cmd: "gh", app: None, dir2: None, ext: None },
    AgentDef { id: "cursor", name: "Cursor", description: "AI 代码编辑器", dir: ".cursor", cmd: "cursor", app: Some("/Applications/Cursor.app"), dir2: None, ext: None },
    AgentDef { id: "cline", name: "Cline", description: "VS Code 编码助手", dir: ".cline", cmd: "", app: None, dir2: None, ext: Some("cline") },
    AgentDef { id: "continue", name: "Continue", description: "开源 IDE 编码助手", dir: ".continue", cmd: "", app: None, dir2: None, ext: Some("continue") },
    AgentDef { id: "zed", name: "Zed", description: "高性能代码编辑器", dir: ".zed", cmd: "zed", app: Some("/Applications/Zed.app"), dir2: None, ext: None },
    AgentDef { id: "warp", name: "Warp", description: "AI 终端", dir: ".warp", cmd: "warp", app: Some("/Applications/Warp.app"), dir2: None, ext: None },
    AgentDef { id: "kiro", name: "Kiro", description: "AWS AI IDE", dir: ".kiro", cmd: "kiro", app: None, dir2: None, ext: None },
    AgentDef { id: "antigravity", name: "Antigravity", description: "Google AI IDE", dir: ".antigravity", cmd: "", app: Some("/Applications/Antigravity.app"), dir2: None, ext: None },
    AgentDef { id: "devin", name: "Devin", description: "Cognition AI 工程师", dir: ".devin", cmd: "devin", app: None, dir2: None, ext: None },
    AgentDef { id: "droid", name: "Droid", description: "Factory 终端工程师", dir: ".droid", cmd: "droid", app: None, dir2: None, ext: None },
    AgentDef { id: "dsh", name: "DSH", description: "DeepSeek 终端助手", dir: ".dsh", cmd: "dsh", app: None, dir2: None, ext: None },
    AgentDef { id: "opencode", name: "OpenCode", description: "开源终端 AI 编码工具", dir: ".config/opencode", cmd: "opencode", app: None, dir2: None, ext: None },
    AgentDef { id: "openclaw", name: "OpenClaw", description: "开源 AI 助手框架", dir: ".openclaw", cmd: "openclaw", app: None, dir2: Some(".kimi_openclaw"), ext: None },
    AgentDef { id: "hermes", name: "Hermes", description: "Nous Research 助手", dir: ".hermes", cmd: "hermes", app: None, dir2: None, ext: None },
    AgentDef { id: "pi", name: "Pi", description: "Mario Zecher 极简助手", dir: ".pi", cmd: "pi", app: None, dir2: None, ext: None },
    AgentDef { id: "openhuman", name: "OpenHuman", description: "开源个人 AI", dir: ".openhuman", cmd: "openhuman", app: None, dir2: None, ext: None },
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

fn is_installed(def: &AgentDef, home: &Option<PathBuf>) -> bool {
    if let Some(home) = home {
        if home.join(def.dir).exists() {
            return true;
        }
        if let Some(dir2) = def.dir2 {
            if home.join(dir2).exists() {
                return true;
            }
        }
    }
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
