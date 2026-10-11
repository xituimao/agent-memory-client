import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import "./styles.css";

type Status =
  | { state: "notConfigured" }
  | { state: "connecting" }
  | { state: "online"; latencyMs: number }
  | { state: "offline"; detail: string }
  | { state: "disconnected" };

interface AppConfig {
  host: string;
  sshPort: number;
  sshUser: string;
  sshKeyPath: string;
  restPort: number;
  viewerPort: number;
  token: string;
  extraSshArgs: string;
  autoConnect: boolean;
  autoStart: boolean;
  agentName: string;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const els = {
  viewHome: $("viewHome"),
  viewSettings: $("viewSettings"),
  hero: $("hero"),
  ringFg: $("ringFg") as unknown as SVGCircleElement,
  heroStatus: $("heroStatus"),
  heroSub: $("heroSub"),
  btnPower: $("btnPower") as HTMLButtonElement,
  btnOpenSettings: $("btnOpenSettings") as HTMLButtonElement,
  btnOpenGuide: $("btnOpenGuide") as HTMLButtonElement,
  btnGuideBack: $("btnGuideBack") as HTMLButtonElement,
  btnAgents: $("btnAgents") as HTMLButtonElement,
  btnAgentsBack: $("btnAgentsBack") as HTMLButtonElement,
  agentList: $("agentList"),
  viewGuide: $("viewGuide"),
  viewAgents: $("viewAgents"),
  btnBack: $("btnBack") as HTMLButtonElement,
  btnViewer: $("btnViewer") as HTMLButtonElement,
  btnCopy: $("btnCopy") as HTMLButtonElement,
  // settings form
  host: $("host") as HTMLInputElement,
  sshPort: $("sshPort") as HTMLInputElement,
  sshUser: $("sshUser") as HTMLInputElement,
  sshKeyPath: $("sshKeyPath") as HTMLInputElement,
  restPort: $("restPort") as HTMLInputElement,
  viewerPort: $("viewerPort") as HTMLInputElement,
  token: $("token") as HTMLInputElement,
  extraSshArgs: $("extraSshArgs") as HTMLInputElement,
  autoConnect: $("autoConnect") as HTMLInputElement,
  autoStart: $("autoStart") as HTMLInputElement,
  agentName: $("agentName") as HTMLInputElement,
  btnPickKey: $("btnPickKey") as HTMLButtonElement,
  btnTest: $("btnTest") as HTMLButtonElement,
  btnSave: $("btnSave") as HTMLButtonElement,
  toast: $("toast"),
};

const RING_LEN = 2 * Math.PI * 88;

let currentStatus: Status = { state: "connecting" };
let currentConfig: AppConfig | null = null;

/* ---------- 视图切换 ---------- */

type ViewName = "home" | "settings" | "guide" | "agents";

function showView(name: ViewName) {
  els.viewHome.classList.toggle("hidden", name !== "home");
  els.viewSettings.classList.toggle("hidden", name !== "settings");
  els.viewGuide.classList.toggle("hidden", name !== "guide");
  els.viewAgents.classList.toggle("hidden", name !== "agents");
  if (name === "agents") loadAgents();
}

function showSettings(show: boolean) {
  showView(show ? "settings" : "home");
}

/* ---------- 状态渲染 ---------- */

function renderStatus(s: Status) {
  currentStatus = s;
  els.hero.className = "hero " + s.state;

  if (s.state === "online") {
    els.heroStatus.textContent = "已连接";
    els.heroSub.textContent = currentConfig ? `${currentConfig.host} · ${s.latencyMs} ms` : `${s.latencyMs} ms`;
    els.btnPower.disabled = false;
  } else if (s.state === "connecting") {
    els.heroStatus.textContent = "连接中…";
    els.heroSub.textContent = "正在建立加密隧道";
    els.btnPower.disabled = true;
  } else if (s.state === "offline") {
    els.heroStatus.textContent = "连接已断开";
    els.heroSub.textContent = s.detail + "，正在重连";
    els.btnPower.disabled = false;
  } else if (s.state === "disconnected") {
    els.heroStatus.textContent = "未连接";
    els.heroSub.textContent = currentConfig ? currentConfig.host : "";
    els.btnPower.disabled = false;
  } else {
    els.heroStatus.textContent = "需要配置";
    els.heroSub.textContent = "点击右上角齿轮完成设置";
    els.btnPower.disabled = false;
  }

  // 状态环进度：在线满环、连接中旋转、其它低亮
  if (s.state === "online") {
    els.ringFg.style.strokeDashoffset = "0";
  } else if (s.state !== "connecting") {
    els.ringFg.style.strokeDashoffset = String(RING_LEN);
  }
}

els.ringFg.style.strokeDasharray = String(RING_LEN);
els.ringFg.style.strokeDashoffset = String(RING_LEN);

/* ---------- 表单 ---------- */

function readForm(): AppConfig {
  return {
    host: els.host.value.trim(),
    sshPort: Number(els.sshPort.value) || 22,
    sshUser: els.sshUser.value.trim() || "root",
    sshKeyPath: els.sshKeyPath.value.trim(),
    restPort: Number(els.restPort.value) || 3111,
    viewerPort: Number(els.viewerPort.value) || 3113,
    token: els.token.value.trim(),
    extraSshArgs: els.extraSshArgs.value.trim(),
    autoConnect: els.autoConnect.checked,
    autoStart: els.autoStart.checked,
    agentName: els.agentName.value.trim() || "claude-code",
  };
}

function fillForm(c: AppConfig) {
  els.host.value = c.host;
  els.sshPort.value = String(c.sshPort);
  els.sshUser.value = c.sshUser;
  els.sshKeyPath.value = c.sshKeyPath;
  els.restPort.value = String(c.restPort);
  els.viewerPort.value = String(c.viewerPort);
  els.token.value = c.token;
  els.extraSshArgs.value = c.extraSshArgs;
  els.autoConnect.checked = c.autoConnect;
  els.autoStart.checked = c.autoStart;
  els.agentName.value = c.agentName;
}

/* ---------- 提示 ---------- */

let toastTimer: number | undefined;
function toast(msg: string, ok = true) {
  els.toast.textContent = msg;
  els.toast.className = "toast show " + (ok ? "ok" : "err");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => els.toast.classList.remove("show"), 3200);
}

/* ---------- 事件 ---------- */

els.btnOpenSettings.addEventListener("click", () => showView("settings"));
els.btnBack.addEventListener("click", () => showView("home"));
els.btnOpenGuide.addEventListener("click", () => showView("guide"));
els.btnGuideBack.addEventListener("click", () => showView("home"));
els.btnAgents.addEventListener("click", () => showView("agents"));
els.btnAgentsBack.addEventListener("click", () => showView("home"));

els.btnPower.addEventListener("click", async () => {
  const connected = currentStatus.state === "online";
  if (currentStatus.state === "notConfigured") {
    showSettings(true);
    toast("请先完成服务器设置", false);
    return;
  }
  els.btnPower.disabled = true;
  try {
    if (connected) {
      await invoke("disconnect_tunnel");
    } else {
      await invoke("connect_tunnel");
    }
  } catch (e) {
    toast(`操作失败：${e}`, false);
    els.btnPower.disabled = false;
  }
});

els.btnViewer.addEventListener("click", async () => {
  try {
    const c = currentConfig ?? readForm();
    await openUrl(`http://localhost:${c.viewerPort}`);
  } catch (e) {
    toast(`无法打开查看器：${e}`, false);
  }
});

els.btnCopy.addEventListener("click", async () => {
  try {
    const c = currentConfig ?? readForm();
    const cmd = `export AGENTMEMORY_URL=http://localhost:${c.restPort}\nexport AGENTMEMORY_SECRET=${c.token}\nnpx -y @agentmemory/agentmemory@latest connect ${c.agentName}`;
    await writeText(cmd);
    toast("接入命令已复制，粘贴到终端执行");
  } catch (e) {
    toast(`复制失败：${e}`, false);
  }
});

els.btnPickKey.addEventListener("click", async () => {
  try {
    const path = await open({ multiple: false, directory: false });
    if (typeof path === "string") els.sshKeyPath.value = path;
  } catch (e) {
    toast(`选择文件失败：${e}`, false);
  }
});

els.btnTest.addEventListener("click", async () => {
  els.btnTest.disabled = true;
  els.btnTest.textContent = "测试中…";
  try {
    const ms = await invoke<number>("test_connection", { config: readForm() });
    toast(`连接成功 · ${ms} ms`);
  } catch (e) {
    toast(`连接失败：${e}`, false);
  } finally {
    els.btnTest.disabled = false;
    els.btnTest.textContent = "测试连接";
  }
});

els.btnSave.addEventListener("click", async () => {
  els.btnSave.disabled = true;
  try {
    const config = readForm();
    await invoke("save_config", { config });
    currentConfig = config;
    toast("已保存");
    showSettings(false);
  } catch (e) {
    toast(`保存失败：${e}`, false);
  } finally {
    els.btnSave.disabled = false;
  }
});

/* ---------- 接入 AI 助手 ---------- */

interface AgentInfo {
  id: string;
  name: string;
  description: string;
  installed: boolean;
}

async function loadAgents() {
  els.agentList.innerHTML = '<p class="agent-desc" style="padding:8px 4px">正在扫描本机已安装的 AI 助手…</p>';
  try {
    const list = await invoke<AgentInfo[]>("scan_agents");
    els.agentList.innerHTML = "";
    for (const a of list) {
      els.agentList.appendChild(renderAgentRow(a));
    }
  } catch (e) {
    els.agentList.innerHTML = `<p class="agent-msg err">扫描失败：${e}</p>`;
  }
}

function renderAgentRow(a: AgentInfo): HTMLElement {
  const row = document.createElement("div");
  row.className = "agent-row";
  row.innerHTML = `
    <div class="agent-meta">
      <div class="agent-name">
        ${a.name}
        <span class="agent-badge ${a.installed ? "yes" : ""}">${a.installed ? "已安装" : "未检测到"}</span>
      </div>
      <div class="agent-desc">${a.description}</div>
      <div class="agent-msg hidden"></div>
    </div>
    <button class="install-btn" ${a.installed ? "" : "disabled title='未检测到该助手，请先安装它'"}>安装 MCP + SKILL</button>`;
  const btn = row.querySelector(".install-btn") as HTMLButtonElement;
  const msg = row.querySelector(".agent-msg") as HTMLElement;
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "安装中…";
    msg.className = "agent-msg";
    msg.textContent = "";
    try {
      await invoke<string>("install_agent", { id: a.id });
      row.classList.add("done");
      btn.textContent = "已安装 ✓";
      msg.classList.add("ok");
      msg.textContent = "安装成功，该助手已接入记忆服务器。";
    } catch (e) {
      row.classList.add("failed");
      btn.disabled = false;
      btn.textContent = "重试";
      msg.classList.add("err");
      msg.textContent = String(e);
    }
  });
  return row;
}

/* ---------- 初始化 ---------- */

async function init() {
  try {
    currentConfig = await invoke<AppConfig>("get_config");
    fillForm(currentConfig);
    renderStatus(await invoke<Status>("get_status"));
    await listen<Status>("status-changed", (e) => renderStatus(e.payload));
  } catch (e) {
    els.heroStatus.textContent = "载入失败";
    els.heroSub.textContent = String(e);
  }
}

init();
