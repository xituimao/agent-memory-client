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
  heroStats: $("heroStats"),
  btnPower: $("btnPower") as HTMLButtonElement,
  btnOpenSettings: $("btnOpenSettings") as HTMLButtonElement,
  btnOpenGuide: $("btnOpenGuide") as HTMLButtonElement,
  btnGuideBack: $("btnGuideBack") as HTMLButtonElement,
  btnAgents: $("btnAgents") as HTMLButtonElement,
  btnAgentsBack: $("btnAgentsBack") as HTMLButtonElement,
  agentList: $("agentList"),
  genericList: $("genericList"),
  agentHidden: $("agentHidden") as HTMLDetailsElement,
  agentHiddenSummary: $("agentHiddenSummary"),
  agentHiddenList: $("agentHiddenList"),
  btnRescan: $("btnRescan") as HTMLButtonElement,
  viewGuide: $("viewGuide"),
  viewAgents: $("viewAgents"),
  btnBack: $("btnBack") as HTMLButtonElement,
  btnViewer: $("btnViewer") as HTMLButtonElement,
  btnCopyAgentCmd: $("btnCopyAgentCmd") as HTMLButtonElement,
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

let lastLatency: number | null = null;

function renderStatus(s: Status) {
  currentStatus = s;
  els.hero.className = "hero " + s.state;
  // 状态已刷新：丢弃 hover 暂存的旧状态词，避免 mouseleave 写回过期文案
  delete els.btnPower.dataset.statusText;

  if (s.state === "online") {
    els.heroStatus.textContent = "已连接";
    // 延迟跳变阈值化：变化 >15% 才刷新，避免盘心文字闪烁
    if (lastLatency === null || Math.abs(s.latencyMs - lastLatency) / Math.max(lastLatency, 1) > 0.15) {
      lastLatency = s.latencyMs;
      els.heroSub.textContent = currentConfig ? `${currentConfig.host} · ${s.latencyMs} ms` : `${s.latencyMs} ms`;
    }
    els.btnPower.disabled = false;
  } else if (s.state === "connecting") {
    lastLatency = null;
    els.heroStatus.textContent = "连接中…";
    els.heroSub.textContent = "正在建立安全连接";
    els.heroStats.textContent = "";
    els.btnPower.disabled = true;
  } else if (s.state === "offline") {
    lastLatency = null;
    els.heroStatus.textContent = "连接已断开";
    els.heroSub.textContent = s.detail + "，正在自动重连";
    els.heroStats.textContent = "";
    els.btnPower.disabled = false;
  } else if (s.state === "disconnected") {
    lastLatency = null;
    els.heroStatus.textContent = "未连接";
    els.heroSub.textContent = currentConfig ? currentConfig.host : "";
    els.heroStats.textContent = "";
    els.btnPower.disabled = false;
  } else {
    lastLatency = null;
    els.heroStatus.textContent = "需要配置";
    els.heroSub.textContent = "点右上角齿轮完成设置";
    els.heroStats.textContent = "";
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

/* ---------- hover 时状态词 ↔ 动作词：让圆盘的可点性可发现 ---------- */

const HOVER_ACTION: Record<string, string> = {
  online: "断开连接",
  offline: "立即重连",
  disconnected: "连接",
  notConfigured: "打开设置",
};

els.btnPower.addEventListener("mouseenter", () => {
  if (els.btnPower.disabled) return;
  const action = HOVER_ACTION[currentStatus.state];
  if (action && els.btnPower.dataset.statusText == null) {
    els.btnPower.dataset.statusText = els.heroStatus.textContent ?? "";
    els.heroStatus.textContent = action;
  }
});
els.btnPower.addEventListener("mouseleave", () => {
  const t = els.btnPower.dataset.statusText;
  if (t != null) {
    delete els.btnPower.dataset.statusText;
    els.heroStatus.textContent = t;
  }
});

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

els.btnCopyAgentCmd.addEventListener("click", async () => {
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

interface GenericAgentInfo extends AgentInfo {
  manualNote?: string | null;
}

interface Stats {
  todayMemories: number;
  wiredClients: number;
}

async function loadAgents() {
  els.agentList.innerHTML = '<p class="agent-desc" style="padding:8px 2px">正在扫描…</p>';
  els.genericList.innerHTML = "";
  els.agentHiddenList.innerHTML = "";
  els.agentHidden.style.display = "none";
  let hidden = 0;
  try {
    // 后端已排序：本机已装的在前
    const list = await invoke<AgentInfo[]>("scan_agents");
    els.agentList.innerHTML = "";
    for (const a of list) {
      if (a.installed) {
        els.agentList.appendChild(renderAgentRow(a));
      } else {
        hidden += 1;
        els.agentHiddenList.appendChild(renderAgentRow(a, true));
      }
    }
    if (!list.some((a) => a.installed)) {
      els.agentList.innerHTML = '<p class="agent-desc" style="padding:8px 2px">没有在本机检测到官方支持的 AI 助手</p>';
    }
  } catch (e) {
    els.agentList.innerHTML = `<p class="agent-msg err">扫描失败：${e}</p>`;
  }
  // 通用 MCP 客户端（官方 connect 名单之外）
  try {
    const generics = await invoke<GenericAgentInfo[]>("scan_generic_agents");
    els.genericList.innerHTML = "";
    for (const g of generics) {
      if (g.installed) {
        els.genericList.appendChild(renderGenericRow(g));
      } else {
        hidden += 1;
        els.agentHiddenList.appendChild(renderGenericRow(g, true));
      }
    }
    if (!generics.some((g) => g.installed)) {
      els.genericList.innerHTML = '<p class="agent-desc" style="padding:4px 2px">没有在本机检测到支持的 MCP 客户端</p>';
    }
  } catch (e) {
    els.genericList.innerHTML = `<p class="agent-msg err">扫描失败：${e}</p>`;
  }
  if (hidden > 0) {
    els.agentHiddenSummary.textContent = `未在本机检测到（${hidden}）`;
    els.agentHidden.style.display = "";
  }
}

function renderGenericRow(g: GenericAgentInfo, hidden = false): HTMLElement {
  const row = document.createElement("div");
  row.className = "agent-row";
  const manual = g.manualNote != null;
  row.innerHTML = `
    <div class="agent-meta">
      <div class="agent-name">${g.name}</div>
      <div class="agent-desc">${hidden ? g.description + " · 未在本机检测到" : manual ? g.description + " · 需在应用内手动添加" : g.description}</div>
      <div class="agent-msg hidden"></div>
    </div>
    ${manual && g.installed ? '<button class="ghost-btn">复制命令</button>' : hidden ? "" : '<button class="install-btn">接入记忆</button>'}`;
  const btn = row.querySelector(".install-btn") as HTMLButtonElement | null;
  const copyBtn = row.querySelector(".ghost-btn") as HTMLButtonElement | null;
  const msg = row.querySelector(".agent-msg") as HTMLElement;
  if (manual && g.manualNote) {
    copyBtn?.addEventListener("click", async () => {
      try {
        await writeText(g.manualNote!);
        toast("命令与说明已复制，到应用内粘贴配置");
      } catch (e) {
        toast(`复制失败：${e}`, false);
      }
    });
    return row;
  }
  if (!btn) return row;
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "接入中…";
    try {
      const output = await invoke<string>("install_generic_agent", { id: g.id });
      row.classList.add("done");
      btn.textContent = "已接入 ✓";
      msg.classList.add("ok");
      msg.textContent = output;
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

els.btnRescan.addEventListener("click", () => loadAgents());

function renderAgentRow(a: AgentInfo, hidden = false): HTMLElement {
  const row = document.createElement("div");
  row.className = "agent-row";
  row.innerHTML = `
    <div class="agent-meta">
      <div class="agent-name">${a.name}</div>
      <div class="agent-desc">${hidden ? a.description + " · 未在本机检测到" : a.description}</div>
      <div class="agent-msg hidden"></div>
    </div>
    ${hidden ? "" : '<button class="install-btn">接入记忆</button>'}`;
  const btn = row.querySelector(".install-btn") as HTMLButtonElement | null;
  const msg = row.querySelector(".agent-msg") as HTMLElement;
  if (!btn) return row;
  btn.addEventListener("click", async () => {
    btn.disabled = true;
    btn.textContent = "接入中…";
    msg.className = "agent-msg";
    msg.textContent = "";
    try {
      await invoke<string>("install_agent", { id: a.id });
      row.classList.add("done");
      btn.textContent = "已接入 ✓";
      msg.classList.add("ok");
      msg.textContent = "接入成功，该助手的记忆将自动同步到你的服务器。";
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
    await listen<Stats>("stats-changed", (e) => {
      if (currentStatus.state !== "online") return;
      els.heroStats.textContent = `今日 ${e.payload.todayMemories} 条 · 已接入 ${e.payload.wiredClients} 个助手`;
    });
  } catch (e) {
    els.heroStatus.textContent = "载入失败";
    els.heroSub.textContent = String(e);
  }
}

init();
