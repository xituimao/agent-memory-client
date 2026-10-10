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
  powerLabel: $("powerLabel"),
  btnOpenSettings: $("btnOpenSettings") as HTMLButtonElement,
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

function showSettings(show: boolean) {
  els.viewHome.classList.toggle("hidden", show);
  els.viewSettings.classList.toggle("hidden", !show);
}

/* ---------- 状态渲染 ---------- */

function renderStatus(s: Status) {
  currentStatus = s;
  els.hero.className = "hero " + s.state;

  if (s.state === "online") {
    els.heroStatus.textContent = "已连接";
    els.heroSub.textContent = currentConfig ? `${currentConfig.host} · ${s.latencyMs} ms` : `${s.latencyMs} ms`;
    els.btnPower.disabled = false;
    els.powerLabel.textContent = "断开";
  } else if (s.state === "connecting") {
    els.heroStatus.textContent = "连接中…";
    els.heroSub.textContent = "正在建立加密隧道";
    els.btnPower.disabled = true;
    els.powerLabel.textContent = "连接中";
  } else if (s.state === "offline") {
    els.heroStatus.textContent = "连接已断开";
    els.heroSub.textContent = s.detail + "，正在重连";
    els.btnPower.disabled = false;
    els.powerLabel.textContent = "连接";
  } else if (s.state === "disconnected") {
    els.heroStatus.textContent = "未连接";
    els.heroSub.textContent = currentConfig ? currentConfig.host : "";
    els.btnPower.disabled = false;
    els.powerLabel.textContent = "连接";
  } else {
    els.heroStatus.textContent = "需要配置";
    els.heroSub.textContent = "点击右上角齿轮完成设置";
    els.btnPower.disabled = false;
    els.powerLabel.textContent = "连接";
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

els.btnOpenSettings.addEventListener("click", () => showSettings(true));
els.btnBack.addEventListener("click", () => showSettings(false));

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
    toast("MCP 接入命令已复制");
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
