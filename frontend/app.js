// Production control-panel frontend (Tauri 2 withGlobalTauri).

const $ = (id) => document.getElementById(id);
let selectedSession = null;
let sessionsCache = [];
let runtimeReady = false;

function hasTauri() {
  return !!(window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke);
}

async function invoke(cmd, args = {}) {
  if (!hasTauri()) {
    throw new Error("Tauri bridge unavailable — open via the desktop app, not a browser.");
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

function setStatus(kind, text) {
  const bar = $("status-bar");
  bar.className = `status-bar status-${kind}`;
  $("status-text").textContent = text;
}

function toastError(e) {
  const msg = e?.message || String(e);
  appendEvent({ type: "error", message: msg });
  setStatus(runtimeReady ? "ready" : "error", msg);
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

async function refreshStatus() {
  try {
    const s = await invoke("get_runtime_status");
    runtimeReady = !!s.ready;
    setStatus(s.ready ? "ready" : "error", s.message);
    if (s.defaultCwd && !$("cwd").value) {
      $("cwd").value = s.defaultCwd;
      $("repo").value = s.defaultCwd;
    }
    log($("system-log"), s);
    return s;
  } catch (e) {
    setStatus("error", String(e.message || e));
    throw e;
  }
}

function renderSessions(list) {
  sessionsCache = Array.isArray(list) ? list : [];
  $("session-count").textContent = String(sessionsCache.length);
  const root = $("sessions-list");
  if (!sessionsCache.length) {
    root.textContent = "No sessions yet. Start an ACP session to code.";
    return;
  }
  root.innerHTML = "";
  // newest first
  const sorted = [...sessionsCache].sort(
    (a, b) => String(b.created_at || "").localeCompare(String(a.created_at || ""))
  );
  for (const s of sorted) {
    const id = s.id;
    const div = document.createElement("div");
    div.className = "session-item" + (selectedSession === id ? " selected" : "");
    div.innerHTML = `<div><strong>${(s.mode || "?").toUpperCase()}</strong> · ${s.status || "?"} · ${
      s.model || ""
    }</div>
      <div class="meta">${id}<br/>${s.cwd || ""}${
      s.mcp_servers?.length ? `<br/>mcp: ${s.mcp_servers.join(", ")}` : ""
    }</div>`;
    div.onclick = () => {
      selectedSession = id;
      $("selected-label").textContent = id.slice(0, 8) + "…";
      renderSessions(sessionsCache);
    };
    root.appendChild(div);
  }
}

async function refreshSessions() {
  try {
    const list = await invoke("list_sessions");
    renderSessions(list);
    if (Array.isArray(list) && list.length && !selectedSession) {
      selectedSession = list[0].id;
      $("selected-label").textContent = selectedSession.slice(0, 8) + "…";
      renderSessions(list);
    }
  } catch (e) {
    toastError(e);
  }
}

function parseCsv(s) {
  return (s || "")
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);
}

// Sessions
$("btn-start-acp").onclick = async () => {
  try {
    const cwd = $("cwd").value.trim();
    if (!cwd) throw new Error("Set an absolute project directory first");
    const mcpNames = parseCsv($("mcp-attach-session").value);
    // High-risk names need explicit approval when attached
    const highRisk = mcpNames.filter((n) =>
      /playwright|browser|grok-build|custom|^x$/i.test(n)
    );
    const opts = {
      mode: "acp",
      model: $("model").value.trim() || null,
      planMode: $("plan-mode").checked,
      alwaysApprove: $("always-approve").checked,
      mcpServerNames: mcpNames,
      approvedHighRiskMcp: highRisk,
      includeAutoMcp: true,
      mcpServers: [],
      rules: [],
      permissionAllow: [],
      permissionDeny: [],
      trustRepo: false,
      worktree: null,
      prompt: null,
      sandboxProfile: "workspace",
    };
    const res = await invoke("start_session", { cwd, opts });
    selectedSession = res.id;
    $("selected-label").textContent = selectedSession.slice(0, 8) + "…";
    appendEvent({ type: "session_started", res });
    await refreshSessions();
    await refreshStatus();
  } catch (e) {
    toastError(e);
  }
};

$("btn-mock").onclick = async () => {
  try {
    const cwd = $("cwd").value || "/tmp";
    const res = await invoke("start_mock_session", { cwd });
    selectedSession = res.id;
    appendEvent({ type: "mock_started", res });
    await refreshSessions();
  } catch (e) {
    toastError(e);
  }
};

$("btn-refresh").onclick = () => refreshSessions();

$("btn-prompt").onclick = async () => {
  try {
    if (!selectedSession) throw new Error("Select a session first");
    const prompt = $("prompt").value;
    if (!prompt.trim()) throw new Error("Prompt is empty");
    const res = await invoke("send_prompt", { id: selectedSession, prompt });
    appendEvent({ type: "prompt_sent", res });
  } catch (e) {
    toastError(e);
  }
};

$("btn-cancel").onclick = async () => {
  try {
    if (!selectedSession) throw new Error("No session selected");
    await invoke("cancel_session", { id: selectedSession });
    appendEvent({ type: "cancelled", id: selectedSession });
    await refreshSessions();
  } catch (e) {
    toastError(e);
  }
};

$("btn-plan-on").onclick = async () => {
  if (!selectedSession) return toastError(new Error("No session selected"));
  try {
    await invoke("set_plan_mode", { id: selectedSession, enabled: true });
    appendEvent({ type: "plan_mode", enabled: true });
  } catch (e) {
    toastError(e);
  }
};
$("btn-plan-off").onclick = async () => {
  if (!selectedSession) return toastError(new Error("No session selected"));
  try {
    await invoke("set_plan_mode", { id: selectedSession, enabled: false });
    appendEvent({ type: "plan_mode", enabled: false });
  } catch (e) {
    toastError(e);
  }
};

// Worktrees
$("btn-wt-list").onclick = async () => {
  try {
    log($("worktrees-list"), await invoke("list_worktrees", { repo: $("repo").value }));
  } catch (e) {
    toastError(e);
  }
};
$("btn-wt-create").onclick = async () => {
  try {
    log(
      $("worktrees-list"),
      await invoke("create_worktree", {
        repo: $("repo").value,
        name: $("wt-name").value,
        baseRef: null,
      })
    );
  } catch (e) {
    toastError(e);
  }
};

// MCP
$("btn-mcp-catalog").onclick = async () => {
  try {
    log($("mcp-list"), await invoke("list_mcp_catalog"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-list").onclick = async () => {
  try {
    log($("mcp-list"), await invoke("list_mcp_servers"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-doctor").onclick = async () => {
  try {
    log($("mcp-list"), await invoke("doctor_mcp_server", { name: null }));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-tools").onclick = async () => {
  try {
    log($("mcp-list"), await invoke("list_mcp_tools", { name: null }));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-creds").onclick = async () => {
  try {
    log($("mcp-list"), await invoke("list_mcp_credentials"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-add").onclick = async () => {
  try {
    const fromCatalog = $("mcp-catalog").value;
    const name = $("mcp-name").value || fromCatalog;
    const paths = parseCsv($("mcp-paths").value);
    const request = {
      name,
      fromCatalog,
      allowedPaths: paths.length ? paths : null,
      enabled: true,
      readOnly: fromCatalog === "filesystem",
      kind: null,
      transport: null,
      command: null,
      args: null,
      url: null,
      env: null,
      scope: fromCatalog === "filesystem" ? "project" : null,
      description: null,
      autoAttach: fromCatalog === "github",
      requiresApproval: ["browser", "grok_build", "custom", "x_twitter"].includes(fromCatalog),
      headers: null,
      startupTimeoutSec: null,
      toolTimeoutSec: null,
      rateLimitPerMin: fromCatalog === "grok_build" ? 10 : null,
      credentialKeys: null,
    };
    log($("mcp-list"), await invoke("add_mcp_server", { request }));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-remove").onclick = async () => {
  try {
    await invoke("remove_mcp_server", { name: $("mcp-name").value });
    log($("mcp-list"), await invoke("list_mcp_servers"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-cred-set").onclick = async () => {
  try {
    const key = $("cred-key").value.trim();
    const value = $("cred-value").value;
    if (!key || !value) throw new Error("credential key and value required");
    await invoke("set_mcp_credential", { key, value });
    $("cred-value").value = "";
    log($("mcp-list"), await invoke("list_mcp_credentials"));
  } catch (e) {
    toastError(e);
  }
};

// Memory
$("btn-mem-add").onclick = async () => {
  try {
    await invoke("memory_add", {
      scope: $("mem-scope").value,
      content: $("mem-content").value,
      tags: [],
    });
    log($("memory-list"), await invoke("memory_list", { scope: $("mem-scope").value }));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mem-list").onclick = async () => {
  try {
    log($("memory-list"), await invoke("memory_list", { scope: $("mem-scope").value }));
  } catch (e) {
    toastError(e);
  }
};
$("btn-mem-flush").onclick = async () => {
  try {
    log($("memory-list"), await invoke("memory_flush", { scope: $("mem-scope").value }));
  } catch (e) {
    toastError(e);
  }
};

// Scheduler
$("btn-job-add").onclick = async () => {
  try {
    log(
      $("scheduler-list"),
      await invoke("scheduler_add", {
        request: {
          name: $("job-name").value,
          prompt: $("job-prompt").value,
          intervalSecs: Number($("job-interval").value || 3600),
          cron: null,
          onceDelaySecs: null,
          cwd: $("cwd").value || null,
          maxRuns: null,
        },
      })
    );
  } catch (e) {
    toastError(e);
  }
};
$("btn-job-list").onclick = async () => {
  try {
    log($("scheduler-list"), await invoke("scheduler_list"));
  } catch (e) {
    toastError(e);
  }
};

// System
$("btn-status").onclick = () => refreshStatus().catch(toastError);
$("btn-discover").onclick = async () => {
  try {
    log($("system-log"), await invoke("discover_environment"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-baseline").onclick = async () => {
  try {
    log($("system-log"), await invoke("capture_baseline"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-config").onclick = async () => {
  try {
    log($("system-log"), await invoke("get_config"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-checkpoint").onclick = async () => {
  try {
    log($("system-log"), await invoke("persistence_checkpoint"));
  } catch (e) {
    toastError(e);
  }
};
$("btn-shutdown").onclick = async () => {
  try {
    await invoke("shutdown_all");
    selectedSession = null;
    await refreshSessions();
    appendEvent({ type: "shutdown_all" });
  } catch (e) {
    toastError(e);
  }
};

async function bindEvents() {
  if (!hasTauri() || !window.__TAURI__.event) {
    setStatus("error", "Not running inside Tauri — use ./scripts/run.sh or the .app");
    return;
  }
  await window.__TAURI__.event.listen("control-event", (e) => appendEvent(e.payload));
}

async function boot() {
  await bindEvents();
  try {
    await refreshStatus();
    await refreshSessions();
    appendEvent("Control panel ready.");
  } catch (e) {
    toastError(e);
  }
}

boot();
