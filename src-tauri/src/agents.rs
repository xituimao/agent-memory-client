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
    /// 是否已接入 agentmemory（配置文件中检测到接入痕迹）
    pub wired: bool,
    /// 已接入但 env 里是未展开的 "${AGENTMEMORY_*}" 占位符（推送会 401）
    pub needs_repair: bool,
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
    /// 接入痕迹检测：agentmemory 会写入的配置文件（相对用户目录）
    wired_paths: &'static [&'static str],
}

/// 与官方 `agentmemory connect` 支持列表一致（connect --help 实测 21 个）
const AGENTS: &[AgentDef] = &[
    AgentDef { id: "claude-code", name: "Claude Code", description: "Anthropic 官方 CLI", cmd: "claude", app: None, ext: None, wired_paths: &[".claude.json"] },
    AgentDef { id: "codex", name: "Codex CLI", description: "OpenAI 终端编码助手", cmd: "codex", app: None, ext: None, wired_paths: &[".codex/config.toml"] },
    AgentDef { id: "gemini-cli", name: "Gemini CLI", description: "Google 终端 AI 助手", cmd: "gemini", app: None, ext: None, wired_paths: &[".gemini/settings.json"] },
    AgentDef { id: "qwen", name: "Qwen Code", description: "通义千问终端编码助手", cmd: "qwen", app: None, ext: None, wired_paths: &[".qwen/settings.json"] },
    // 注意：不能用 gh 判断——gh 是 GitHub CLI，装了它不等于装了 Copilot CLI
    AgentDef { id: "copilot-cli", name: "Copilot CLI", description: "GitHub 终端助手", cmd: "copilot", app: None, ext: None, wired_paths: &[] },
    AgentDef { id: "cursor", name: "Cursor", description: "AI 代码编辑器", cmd: "cursor", app: Some("/Applications/Cursor.app"), ext: None, wired_paths: &[".cursor/mcp.json"] },
    AgentDef { id: "cline", name: "Cline", description: "VS Code 编码助手", cmd: "", app: None, ext: Some("cline"), wired_paths: &[] },
    AgentDef { id: "continue", name: "Continue", description: "开源 IDE 编码助手", cmd: "", app: None, ext: Some("continue"), wired_paths: &[".continue/config.json"] },
    AgentDef { id: "zed", name: "Zed", description: "高性能代码编辑器", cmd: "zed", app: Some("/Applications/Zed.app"), ext: None, wired_paths: &[".config/zed/settings.json"] },
    AgentDef { id: "warp", name: "Warp", description: "AI 终端", cmd: "warp", app: Some("/Applications/Warp.app"), ext: None, wired_paths: &[] },
    AgentDef { id: "kiro", name: "Kiro", description: "AWS AI IDE", cmd: "kiro", app: None, ext: None, wired_paths: &[".kiro/settings/mcp.json"] },
    AgentDef { id: "antigravity", name: "Antigravity", description: "Google AI IDE", cmd: "", app: Some("/Applications/Antigravity.app"), ext: None, wired_paths: &[] },
    AgentDef { id: "antigravity-cli", name: "Antigravity CLI", description: "Google 终端助手", cmd: "agy", app: None, ext: None, wired_paths: &[] },
    AgentDef { id: "devin", name: "Devin", description: "Cognition AI 工程师", cmd: "devin", app: None, ext: None, wired_paths: &[] },
    AgentDef { id: "droid", name: "Droid", description: "Factory 终端工程师", cmd: "droid", app: None, ext: None, wired_paths: &[".factory/mcp.json"] },
    AgentDef { id: "dsh", name: "DSH", description: "DeepSeek 终端助手", cmd: "dsh", app: None, ext: None, wired_paths: &[".dsh/mcp.json"] },
    AgentDef { id: "opencode", name: "OpenCode", description: "开源终端 AI 编码工具", cmd: "opencode", app: None, ext: None, wired_paths: &[".config/opencode/opencode.json"] },
    AgentDef { id: "openclaw", name: "OpenClaw", description: "开源 AI 助手框架", cmd: "openclaw", app: None, ext: None, wired_paths: &[] },
    AgentDef { id: "hermes", name: "Hermes", description: "Nous Research 助手", cmd: "hermes", app: None, ext: None, wired_paths: &[] },
    AgentDef { id: "pi", name: "Pi", description: "Mario Zecher 极简助手", cmd: "pi", app: None, ext: None, wired_paths: &[] },
    AgentDef { id: "openhuman", name: "OpenHuman", description: "开源个人 AI", cmd: "openhuman", app: None, ext: None, wired_paths: &[] },
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
            wired: def
                .wired_paths
                .iter()
                .any(|rel| has_wire_marker(&home, rel)),
            needs_repair: def
                .wired_paths
                .iter()
                .any(|rel| has_placeholder_env(&home, rel)),
        })
        .collect();
    list.sort_by(|a, b| b.installed.cmp(&a.installed).then(a.name.cmp(&b.name)));
    list
}

/// 配置文件中是否出现 agentmemory 接入痕迹
fn has_wire_marker(home: &Option<PathBuf>, rel: &str) -> bool {
    let Some(home) = home else { return false };
    std::fs::read_to_string(home.join(rel))
        .map(|content| content.to_lowercase().contains("agentmemory"))
        .unwrap_or(false)
}

/// 配置文件中 agentmemory 的 env 是否含未展开的 "${AGENTMEMORY_*}" 占位符
fn has_placeholder_env(home: &Option<PathBuf>, rel: &str) -> bool {
    let Some(home) = home else { return false };
    std::fs::read_to_string(home.join(rel))
        .map(|content| content.contains("${AGENTMEMORY"))
        .unwrap_or(false)
}

/// 本机已接入 agentmemory 的客户端总数（官方 + 通用名单）
pub fn wired_count() -> u64 {
    let home = home_dir();
    let official = AGENTS
        .iter()
        .filter(|d| d.wired_paths.iter().any(|rel| has_wire_marker(&home, rel)))
        .count();
    let generic = GENERIC_AGENTS
        .iter()
        .chain(MANUAL_ONLY.iter())
        .filter(|d| d.wired_paths.iter().any(|rel| has_wire_marker(&home, rel)))
        .count();
    (official + generic) as u64
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

/// PATH 中是否存在该可执行文件。
/// 若命中的是符号链接且解析后的真实文件名是另一个已知客户端的命令
/// （例：Antigravity CLI 会建 gemini -> agy 兼容别名），则视为误报。
fn in_path(cmd: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&paths).any(|dir| {
        let exe = dir.join(cmd);
        (exe.is_file() && !is_alias_of_other(&exe, cmd))
            || exe.with_extension("exe").is_file()
            || exe.with_extension("cmd").is_file()
    })
}

/// 符号链接目标的主名是另一个受支持的命令 → 这是别名而非真实安装
fn is_alias_of_other(exe: &PathBuf, cmd: &str) -> bool {
    let Ok(target) = std::fs::canonicalize(exe) else { return false };
    if target == *exe {
        return false;
    }
    let Some(stem) = target.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
        return false;
    };
    if stem == cmd.to_lowercase() {
        return false;
    }
    known_cmds().iter().any(|c| *c == stem)
}

/// 所有受支持客户端的命令名集合（用于别名识别）
fn known_cmds() -> Vec<String> {
    AGENTS
        .iter()
        .map(|d| d.cmd)
        .chain(GENERIC_AGENTS.iter().map(|d| d.cmd))
        .chain(MANUAL_ONLY.iter().map(|d| d.cmd))
        .filter(|c| !c.is_empty())
        .map(|c| c.to_lowercase())
        .collect()
}

// ============ 通用 MCP 接入（官方 connect 名单之外的 MCP-capable 客户端） ============

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenericAgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    /// 是否已接入 agentmemory（配置文件中检测到接入痕迹）
    pub wired: bool,
    /// 有值 = 暂不支持自动注入，展示提示文案（按钮禁用）
    pub manual_note: Option<String>,
}

struct GenericDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    cmd: &'static str,
    /// macOS 应用候选路径（命中任一即算安装）
    apps: &'static [&'static str],
    /// VS Code / Cursor 扩展标识（可选）
    ext: Option<&'static str>,
    /// 接入痕迹检测：agentmemory 会写入的配置文件（相对用户目录）
    wired_paths: &'static [&'static str],
}

/// 配置文件位置（按客户端区分，支持 Windows 路径差异）
fn mcp_config_path(id: &str, home: &PathBuf) -> Option<PathBuf> {
    match id {
        // Kimi Code CLI：实测 ~/.kimi-code/mcp.json，标准 mcpServers 格式
        "kimi" => Some(home.join(".kimi-code").join("mcp.json")),
        // Claude Desktop
        "claude-desktop" => {
            #[cfg(target_os = "macos")]
            {
                Some(home.join("Library/Application Support/Claude/claude_desktop_config.json"))
            }
            #[cfg(not(target_os = "macos"))]
            {
                Some(home.join("AppData").join("Roaming").join("Claude").join("claude_desktop_config.json"))
            }
        }
        // Windsurf
        "windsurf" => Some(home.join(".codeium").join("windsurf").join("mcp_config.json")),
        _ => None,
    }
}

const GENERIC_AGENTS: &[GenericDef] = &[
    GenericDef { id: "kimi", name: "Kimi CLI", description: "月之暗面终端助手", cmd: "kimi", apps: &["/Applications/Kimi Code.app", "/Applications/Kimi.app"], ext: None, wired_paths: &[".kimi-code/mcp.json"] },
    GenericDef { id: "claude-desktop", name: "Claude Desktop", description: "Anthropic 桌面应用", cmd: "", apps: &["/Applications/Claude.app"], ext: None, wired_paths: &["Library/Application Support/Claude/claude_desktop_config.json"] },
    GenericDef { id: "windsurf", name: "Windsurf", description: "Codeium AI IDE", cmd: "", apps: &["/Applications/Windsurf.app"], ext: None, wired_paths: &[".codeium/windsurf/mcp_config.json"] },
];

/// 已检测到、但 MCP 配置由应用自己管理、只能手动添加的客户端
/// （2026 主流国内外工具，依据网络调研补充）
const MANUAL_ONLY: &[GenericDef] = &[
    GenericDef { id: "grok", name: "Grok CLI", description: "xAI 终端助手", cmd: "grok", apps: &[], ext: None, wired_paths: &[] },
    GenericDef { id: "trae", name: "Trae", description: "字节 AI IDE", cmd: "trae", apps: &["/Applications/Trae.app", "/Applications/Trae CN.app"], ext: None, wired_paths: &[] },
    GenericDef { id: "qoder", name: "Qoder", description: "阿里 AI IDE", cmd: "", apps: &["/Applications/Qoder.app", "/Applications/Qoder CN.app"], ext: None, wired_paths: &[] },
    GenericDef { id: "codebuddy", name: "CodeBuddy", description: "腾讯编程助手", cmd: "codebuddy", apps: &[], ext: None, wired_paths: &[] },
    GenericDef { id: "iflow", name: "iFlow CLI", description: "心流终端助手", cmd: "iflow", apps: &[], ext: None, wired_paths: &[] },
    GenericDef { id: "goose", name: "Goose", description: "Block 开源助手", cmd: "goose", apps: &[], ext: None, wired_paths: &[".config/goose/config.yaml"] },
    GenericDef { id: "roo-code", name: "Roo Code", description: "VS Code 编码助手", cmd: "", apps: &[], ext: Some("roo-cline"), wired_paths: &[] },
    GenericDef { id: "kilo-code", name: "Kilo Code", description: "VS Code 编码助手", cmd: "", apps: &[], ext: Some("kilocode"), wired_paths: &[] },
    GenericDef { id: "workbuddy", name: "WorkBuddy", description: "腾讯 AI 办公助手", cmd: "", apps: &["/Applications/WorkBuddy.app"], ext: None, wired_paths: &[] },
];

const MANUAL_NOTE: &str = "该应用的 MCP 列表由它自己管理，请在应用内的 MCP/插件设置中添加：命令 npx -y @agentmemory/agentmemory@latest mcp，环境变量 AGENTMEMORY_URL=http://localhost:3111、AGENTMEMORY_SECRET=你的密钥";

pub fn scan_generic() -> Vec<GenericAgentInfo> {
    let home = home_dir();
    let mut list: Vec<GenericAgentInfo> = GENERIC_AGENTS
        .iter()
        .map(|def| GenericAgentInfo {
            id: def.id.to_string(),
            name: def.name.to_string(),
            description: def.description.to_string(),
            installed: generic_installed(def),
            wired: def.wired_paths.iter().any(|rel| has_wire_marker(&home, rel)),
            manual_note: None,
        })
        .collect();
    for def in MANUAL_ONLY {
        if generic_installed(def) {
            list.push(GenericAgentInfo {
                id: def.id.to_string(),
                name: def.name.to_string(),
                description: def.description.to_string(),
                installed: true,
                wired: def.wired_paths.iter().any(|rel| has_wire_marker(&home, rel)),
                manual_note: Some(MANUAL_NOTE.to_string()),
            });
        }
    }
    list.sort_by(|a, b| b.installed.cmp(&a.installed).then(a.name.cmp(&b.name)));
    list
}

fn generic_installed(def: &GenericDef) -> bool {
    #[cfg(not(target_os = "macos"))]
    let _ = def.apps;
    #[cfg(target_os = "macos")]
    for app in def.apps {
        if PathBuf::from(app).exists() {
            return true;
        }
    }
    if let Some(ext) = def.ext {
        if has_vscode_ext(ext, &home_dir()) {
            return true;
        }
    }
    !def.cmd.is_empty() && in_path(def.cmd)
}

pub fn is_generic_supported(id: &str) -> bool {
    GENERIC_AGENTS.iter().any(|d| d.id == id)
}

/// 向客户端的 MCP 配置文件注入 agentmemory（幂等：重复执行覆盖同名条目）
pub fn install_generic(id: &str, rest_url: &str, token: &str) -> Result<String, String> {
    if !is_generic_supported(id) {
        return Err("不支持的客户端".to_string());
    }
    let home = home_dir().ok_or("无法确定用户目录")?;
    let path = mcp_config_path(id, &home).ok_or("未知的配置路径")?;

    // 读取现有配置（缺省视为空 mcpServers），合并 agentmemory 条目
    let mut root: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let servers = root
        .as_object_mut()
        .ok_or("配置文件格式不是 JSON 对象")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or("mcpServers 字段格式不正确")?;
    servers.insert(
        "agentmemory".to_string(),
        serde_json::json!({
            "command": "npx",
            "args": ["-y", "@agentmemory/agentmemory@latest", "mcp"],
            "env": { "AGENTMEMORY_URL": rest_url, "AGENTMEMORY_SECRET": token },
        }),
    );

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("无法创建配置目录: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("无法写入配置文件: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(format!("已写入 {}", path.display()))
}

/// 校验 id 合法（防止注入到 npx 命令）
pub fn is_supported(id: &str) -> bool {
    AGENTS.iter().any(|d| d.id == id)
}

/// connect 成功后把客户端 JSON 配置里 agentmemory 的 env 占位符
/// （如 "${AGENTMEMORY_SECRET:-}"，多数客户端不会展开）替换为真实值。
/// 只处理 JSON 配置；返回是否发生了修改。
pub fn inline_secret(id: &str, rest_url: &str, token: &str) -> Result<bool, String> {
    let def = AGENTS.iter().find(|d| d.id == id).ok_or("不支持的助手类型")?;
    let Some(home) = home_dir() else { return Ok(false) };
    let mut changed = false;
    for rel in def.wired_paths {
        if !rel.ends_with(".json") {
            continue;
        }
        let path = home.join(rel);
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(env) = root
            .pointer_mut("/mcpServers/agentmemory/env")
            .and_then(|e| e.as_object_mut())
        else {
            continue;
        };
        env.insert(
            "AGENTMEMORY_URL".to_string(),
            serde_json::Value::String(rest_url.to_string()),
        );
        env.insert(
            "AGENTMEMORY_SECRET".to_string(),
            serde_json::Value::String(token.to_string()),
        );
        // 其余 "${AGENTMEMORY_X:-default}" 占位符取默认值内联（如 TOOLS）
        let keys: Vec<String> = env
            .iter()
            .filter(|(_, v)| v.as_str().map(|s| s.contains("${AGENTMEMORY")).unwrap_or(false))
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            let default = env[key.as_str()]
                .as_str()
                .and_then(|s| s.split(":-").nth(1))
                .and_then(|s| s.strip_suffix('}'))
                .unwrap_or("")
                .to_string();
            env.insert(key, serde_json::Value::String(default));
        }
        let json = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("无法写入 {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        changed = true;
    }
    Ok(changed)
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

    #[test]
    fn symlink_alias_is_not_install_evidence() {
        // gemini -> agy 这类兼容别名不应算 gemini 已安装
        let tmp = std::env::temp_dir().join(format!("amc-alias-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let real = tmp.join("agy");
        std::fs::write(&real, b"x").unwrap();
        #[cfg(unix)]
        {
            let alias = tmp.join("gemini");
            std::os::unix::fs::symlink(&real, &alias).unwrap();
            assert!(is_alias_of_other(&alias, "gemini"));
            assert!(!is_alias_of_other(&real, "agy"));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn placeholder_env_detected() {
        let tmp = std::env::temp_dir().join(format!("amc-placeholder-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let home = Some(tmp.clone());
        let file = tmp.join("settings.json");
        std::fs::write(
            &file,
            r#"{"mcpServers":{"agentmemory":{"env":{"AGENTMEMORY_SECRET":"${AGENTMEMORY_SECRET:-}"}}}}"#,
        )
        .unwrap();
        assert!(has_wire_marker(&home, "settings.json"));
        assert!(has_placeholder_env(&home, "settings.json"));
        std::fs::write(&file, r#"{"mcpServers":{"agentmemory":{"env":{"AGENTMEMORY_SECRET":"real"}}}}"#).unwrap();
        assert!(!has_placeholder_env(&home, "settings.json"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
