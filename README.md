# Agent Memory Client

agentmemory 服务器的极简桌面客户端：自动建立并维护 SSH 隧道，系统栏实时显示连接状态，一键打开记忆查看器。

## 功能

- **VPN 式主界面**：大状态环 + 一个圆形连接/断开按钮，状态一目了然；服务器、Token 等全部收在设置页
- **常驻系统栏**：macOS 菜单栏 / Windows 任务栏，绿点 = 已连接，红点 = 未连接（自动重试），灰点 = 连接中
- **自动隧道**：启动后自动拉起 `ssh -N`，断线按 1s→2s→…→60s 指数退避自动重连
- **手动断开**：点按钮断开即停，不会偷偷重连；「启动时自动连接」开关控制开机行为
- **自动探活**：每 3 秒带 Bearer token 探测 REST 端口，401 会明确提示 Token 无效
- **一键查看器**：托盘菜单或主界面链接直接打开 `http://localhost:3113` 记忆查看器
- **一键复制 MCP 命令**：`AGENTMEMORY_URL` / `AGENTMEMORY_SECRET` / `npx ... connect` 三行命令，粘贴即用
- **登录自启**：设置里开关，托盘菜单也可切换

## 安装

- **macOS**：双击 `Agent Memory_0.1.0_aarch64.dmg`，把 App 拖进 Applications
- **Windows**：双击 `Agent Memory_0.1.0_x64-setup.exe` 安装（需要 Windows 10+，自带 OpenSSH 客户端）

## 首次配置

首次打开会自动弹出设置窗口：

| 配置项 | 说明 | 默认值 |
|---|---|---|
| 服务器地址 | SSH HostName | 你自己的服务器 |
| SSH 端口 / 用户 | 登录服务器的凭据 | 22 / root |
| SSH 私钥路径 | 留空则使用默认密钥（ssh-agent / ~/.ssh/id_*） | 空 |
| REST / Viewer 端口 | 本地转发端口，对应服务器 3111 / 3113 | 3111 / 3113 |
| Bearer Token | 服务器上 `cat ~/.agentmemory/secret` 获取 | 必填 |

保存后隧道自动建立，托盘变绿即完成。

## 开发

```bash
npm install
npm run tauri dev      # 开发模式（热更新前端）
npm run tauri build    # 产出 macOS .dmg
```

Windows 安装包：推 tag `v*` 到 GitHub，Actions 自动产出（见 `.github/workflows/build.yml`）。

## 架构

- `src-tauri/src/config.rs` — 配置读写（`~/.config/agent-memory-client/config.json`，0600 权限，含 Bearer token）
- `src-tauri/src/tunnel.rs` — `ssh -N` 子进程生命周期 + 指数退避重连
- `src-tauri/src/health.rs` — REST 端口定时探活（任何 HTTP 响应视为在线）
- `src-tauri/src/tray.rs` — 托盘图标/菜单/自启
- `src-tauri/src/state.rs` — 状态机：`notConfigured → connecting → online / offline`

依赖系统 OpenSSH 客户端（macOS/Linux 自带，Windows 10+ 自带）。

## 注意事项

- 关闭窗口不会退出应用，它在系统栏继续跑；退出请用托盘菜单「退出」
- 私钥建议使用无密码密钥，或先 `ssh-add` 进 ssh-agent（客户端以 `BatchMode` 运行，不会弹出密码输入）
