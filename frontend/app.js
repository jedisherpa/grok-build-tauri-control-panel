// Minimal control-panel frontend (Tauri invoke + event bridge).
// Works in browser without Tauri by showing stub messages.

const $ = (id) => document.getElementById(id);
let selectedSession = null;

function hasTauri() {
  return !!(window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke);
}

async function invoke(cmd, args = {}) {
  if (!hasTauri()) {
    return { offline: true, cmd, args };
  }
  return window.__TAURI__.core.invoke(cmd, args);
}

function log(el, data) {
  el.textContent = typeof data === "string" ? data : JSON.stringify(data, null, 2);
}

function appendEvent(ev) {
  const el = $("events");
  const line = typeof ev === "string" ? ev : JSON.stringify(ev);
  el.textContent = `${new Date().toISOString()} ${line}\n` + el.textContent;
}

// Tabs
document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    $(btn.dataset.tab).classList.add("active");
  });
});

// Sessions
$("btn-mock").onclick = async () => {
  const cwd = $("cwd").value || "/tmp";
  const res = await invoke("start_mock_session", { cwd });
  selectedSession = res.id || null;
  appendEvent({ type: "mock_started", res });
  await refreshSessions();
};

$("btn-refresh").onclick = refreshSessions;

async function refreshSessions() {
  const list = await invoke("list_sessions");
  log($("sessions-list"), list);
  if (Array.isArray(list) && list.length && !selectedSession) {
    selectedSession = list[0].id;
  }
}

$("btn-prompt").onclick = async () => {
  if (!selectedSession) return appendEvent("no session selected");
  const prompt = $("prompt").value;
  const res = await invoke("send_prompt", { id: selectedSession, prompt });
  appendEvent({ type: "prompt_sent", res });
};

$("btn-cancel").onclick = async () => {
  if (!selectedSession) return;
  const res = await invoke("cancel_session", { id: selectedSession });
  appendEvent({ type: "cancelled", res });
  await refreshSessions();
};

// Worktrees
$("btn-wt-list").onclick = async () => {
  const repo = $("repo").value;
  log($("worktrees-list"), await invoke("list_worktrees", { repo }));
};
$("btn-wt-create").onclick = async () => {
  const repo = $("repo").value;
  const name = $("wt-name").value;
  log($("worktrees-list"), await invoke("create_worktree", { repo, name, baseRef: null }));
};

// Extensions
$("btn-ext-list").onclick = async () => {
  log($("extensions-list"), await invoke("list_extensions"));
};
$("btn-ext-doctor").onclick = async () => {
  log($("extensions-list"), await invoke("extensions_doctor"));
};

// Memory
$("btn-mem-add").onclick = async () => {
  await invoke("memory_add", {
    scope: $("mem-scope").value,
    content: $("mem-content").value,
    tags: [],
  });
  log($("memory-list"), await invoke("memory_list", { scope: $("mem-scope").value }));
};
$("btn-mem-list").onclick = async () => {
  log($("memory-list"), await invoke("memory_list", { scope: $("mem-scope").value }));
};
$("btn-mem-flush").onclick = async () => {
  log($("memory-list"), await invoke("memory_flush", { scope: $("mem-scope").value }));
};

// Scheduler
$("btn-job-add").onclick = async () => {
  const res = await invoke("scheduler_add", {
    request: {
      name: $("job-name").value,
      prompt: $("job-prompt").value,
      intervalSecs: Number($("job-interval").value || 3600),
      cron: null,
      onceDelaySecs: null,
      cwd: $("cwd").value || null,
      maxRuns: null,
    },
  });
  log($("scheduler-list"), res);
};
$("btn-job-list").onclick = async () => {
  log($("scheduler-list"), await invoke("scheduler_list"));
};

// System
$("btn-discover").onclick = async () => log($("system-log"), await invoke("discover_environment"));
$("btn-baseline").onclick = async () => log($("system-log"), await invoke("capture_baseline"));
$("btn-config").onclick = async () => log($("system-log"), await invoke("get_config"));
$("btn-checkpoint").onclick = async () => log($("system-log"), await invoke("persistence_checkpoint"));
$("btn-shutdown").onclick = async () => log($("system-log"), await invoke("shutdown_all"));

// Live events
async function bindEvents() {
  if (!hasTauri() || !window.__TAURI__.event) {
    appendEvent("Running without Tauri bridge (UI preview mode).");
    return;
  }
  await window.__TAURI__.event.listen("control-event", (e) => appendEvent(e.payload));
}

bindEvents();
refreshSessions().catch(() => {});
