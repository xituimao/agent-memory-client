import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./styles.css";

type Status =
  | { state: "notConfigured" }
  | { state: "connecting" }
  | { state: "online"; latencyMs: number }
  | { state: "offline"; detail: string };

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
  statusCard: $("statusCard"),
  statusDot: $("statusDot"),
  statusTitle: $("statusTitle"),
  statusSub: $("statusSub"),
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
  btnViewer: $("btnViewer") as HTMLButtonElement,
  btnTest: $("btnTest") as HTMLButtonElement,
  btnCopy: $("btnCopy") as HTMLButtonElement,
  btnSave: $("btnSave") as HTMLButtonElement,
  toast: $("toast"),
};

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

function renderStatus(s: Status) {
  els.statusCard.className = "status-card " + s.state;
  if (s.state === "online") {
    els.statusTitle.textContent = "已连接";
    els.statusSub.textContent = `延迟 ${s.latencyMs} ms · 记忆服务正常`;
  } else if (s.state === "connecting") {
    els.statusTitle.textContent = "连接中…";
    els.statusSub.textContent = "正在建立 SSH 隧道";
  } else if (s.state === "offline") {
    els.statusTitle.textContent = "未连接";
    els.statusSub.textContent = s.detail + "（自动重试中）";
  } else {
    els.statusTitle.textContent = "未配置";
    els.statusSub.textContent = "请填写服务器地址与 Token 后保存";
  }
}

let toastTimer: number | undefined;
function toast(msg: string, ok = true) {
  els.toast.textContent = msg;
  els.toast.className = "toast show " + (ok ? "ok" : "err");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => els.toast.classList.remove("show"), 3200);
}

async function copyText(text: string) {
  await navigator.clipboard.writeText(text);
}

els.btnPickKey.addEventListener("click", async () => {
  const path = await open({ multiple: false, directory: false });
  if (typeof path === "string") els.sshKeyPath.value = path;
});

els.btnViewer.addEventListener("click", async () => {
  const c = readForm();
  await openUrl(`http://localhost:${c.viewerPort}`);
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

els.btnCopy.addEventListener("click", async () => {
  const c = readForm();
  const cmd = `export AGENTMEMORY_URL=http://localhost:${c.restPort}\nexport AGENTMEMORY_SECRET=${c.token}\nnpx -y @agentmemory/agentmemory@latest connect ${c.agentName}`;
  await copyText(cmd);
  toast("MCP 接入命令已复制到剪贴板");
});

els.btnSave.addEventListener("click", async () => {
  els.btnSave.disabled = true;
  try {
    await invoke("save_config", { config: readForm() });
    toast("已保存，隧道正在重建");
  } catch (e) {
    toast(`保存失败：${e}`, false);
  } finally {
    els.btnSave.disabled = false;
  }
});

async function init() {
  const config = await invoke<AppConfig>("get_config");
  fillForm(config);
  renderStatus(await invoke<Status>("get_status"));
  await listen<Status>("status-changed", (e) => renderStatus(e.payload));
}

init();
