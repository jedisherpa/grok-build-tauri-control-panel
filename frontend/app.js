// Bomb Code — three-column Grok Build control panel.

const $ = (id) => document.getElementById(id);

const state = {
  selectedSession: null,
  sessions: [],
  tools: [], // recent tool calls
  ready: false,
  transcriptBySession: new Map(), // id -> [{role, body, at}]
};

function hasTauri() {
  return !!(window.__TAURI__?.core?.invoke);
}

async function invoke(cmd, args = {}) {
  if (!hasTauri()) {
    throw new Error("Open via the desktop app (Tauri bridge missing).");
  }
  return window.__TAURI__.core.invoke(cmd, args);
}

function nowIso() {
  return new Date().toISOString();
}

function shortId(id) {
  if (!id) return "—";
  return String(id).slice(0, 8);
}

function setStatus(kind, text) {
  const pill = $("status-pill");
  pill.className = `status-pill status-${kind}`;
  $("status-text").textContent = text;
}

function pushEvent(text, cls = "") {
  const feed = $("event-feed");
  const line = document.createElement("div");
  line.className = `event-line ${cls}`;
  const ts = new Date().toLocaleTimeString();
  line.innerHTML = `<span class="ts">${ts}</span>${escapeHtml(text)}`;
  feed.prepend(line);
  while (feed.children.length > 200) feed.lastChild.remove();
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function toastError(e) {
  const msg = e?.message || String(e);
  pushEvent(msg, "err");
  setStatus(state.ready ? "ready" : "error", msg);
}

// ── Navigation ──────────────────────────────────────────────────────────
document.querySelectorAll(".nav-item").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".nav-item").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
    btn.classList.add("active");
    const view = $(`view-${btn.dataset.view}`);
    if (view) view.classList.add("active");
  });
});

// ── Transcript (center) ─────────────────────────────────────────────────
function getTranscript(sessionId) {
  if (!state.transcriptBySession.has(sessionId)) {
    state.transcriptBySession.set(sessionId, []);
  }
  return state.transcriptBySession.get(sessionId);
}

function appendTranscript(sessionId, role, body, at = nowIso()) {
  if (!sessionId) return;
  const list = getTranscript(sessionId);
  list.push({ role, body, at });
  if (sessionId === state.selectedSession) {
    renderTranscript();
  }
}

function renderTranscript() {
  const root = $("transcript");
  const sid = state.selectedSession;
  if (!sid) {
    root.innerHTML = `<div class="welcome">
<pre class="banner">  ╔══════════════════════════════════════╗
  ║              bomb code               ║
  ╚══════════════════════════════════════╝</pre>
<p>Select a thread or start a new ACP session.</p>
<p class="muted">Center stream mirrors Grok Build in the terminal.</p>
</div>`;
    $("composer-session").textContent = "no session";
    $("composer-model").textContent = "";
    return;
  }

  const sess = state.sessions.find((s) => s.id === sid);
  $("composer-session").textContent = `${shortId(sid)} · ${sess?.status || "?"}`;
  $("composer-model").textContent = sess?.model || "";

  const entries = getTranscript(sid);
  if (!entries.length) {
    root.innerHTML = `<div class="welcome">
<pre class="banner">session ${escapeHtml(shortId(sid))}</pre>
<p class="muted">${escapeHtml(sess?.cwd || "")}</p>
<p>Connected. Type a prompt below.</p>
</div>`;
    return;
  }

  root.innerHTML = entries
    .map((e) => {
      const role = e.role || "system";
      const label =
        role === "user"
          ? "you"
          : role === "agent"
            ? "grok"
            : role === "tool"
              ? "tool"
              : role === "plan"
                ? "plan"
                : role === "error"
                  ? "error"
                  : "system";
      return `<div class="t-block ${escapeHtml(role)}">
  <div class="t-role">${label}</div>
  <div class="t-body">${escapeHtml(e.body)}</div>
  <div class="t-time">${escapeHtml(e.at || "")}</div>
</div>`;
    })
    .join("");
  root.scrollTop = root.scrollHeight;
}

// ── Threads / agents ────────────────────────────────────────────────────
function renderThreads() {
  const root = $("thread-list");
  if (!state.sessions.length) {
    root.innerHTML = `<div class="empty-hint">No sessions yet</div>`;
    return;
  }
  const sorted = [...state.sessions].sort((a, b) =>
    String(b.createdAt || b.created_at || "").localeCompare(String(a.createdAt || a.created_at || ""))
  );
  root.innerHTML = sorted
    .map((s) => {
      const id = s.id;
      const status = String(s.status || "unknown").toLowerCase();
      const mode = String(s.mode || "acp").toLowerCase();
      const model = s.model || "";
      const isMock = model === "mock";
      const selected = id === state.selectedSession ? "selected" : "";
      const badgeCls = isMock
        ? "mock"
        : status.includes("run")
          ? "running"
          : status.includes("fail") || status.includes("cancel")
            ? "failed"
            : "idle";
      const cwd = s.cwd || "";
      const shortCwd = cwd.length > 28 ? "…" + cwd.slice(-27) : cwd;
      return `<div class="thread-item ${selected}" data-id="${escapeHtml(id)}">
  <div class="name">${escapeHtml(mode)} · ${escapeHtml(shortId(id))}</div>
  <div class="meta"><span class="badge ${badgeCls}">${escapeHtml(status)}</span>
  <span>${escapeHtml(isMock ? "mock" : model || "—")}</span></div>
  <div class="meta">${escapeHtml(shortCwd)}</div>
</div>`;
    })
    .join("");

  root.querySelectorAll(".thread-item").forEach((el) => {
    el.onclick = () => selectSession(el.dataset.id);
  });
}

function renderAgents() {
  const root = $("agent-list");
  if (!state.sessions.length) {
    root.innerHTML = `<div class="empty-hint">No active agents</div>`;
    return;
  }
  root.innerHTML = state.sessions
    .map((s) => {
      const status = String(s.status || "?").toLowerCase();
      const badgeCls = status.includes("run")
        ? "running"
        : status.includes("fail") || status.includes("cancel")
          ? "failed"
          : "idle";
      return `<div class="agent-card">
  <div class="name">${escapeHtml(String(s.mode || "acp").toUpperCase())} · ${escapeHtml(shortId(s.id))}</div>
  <div class="meta"><span class="badge ${badgeCls}">${escapeHtml(status)}</span>
  <span class="muted">${escapeHtml(s.model || "")}</span></div>
  <div class="path">${escapeHtml(s.cwd || "")}</div>
  ${
    s.mcpServers?.length || s.mcp_servers?.length
      ? `<div class="path">mcp: ${escapeHtml((s.mcpServers || s.mcp_servers || []).join(", "))}</div>`
      : ""
  }
</div>`;
    })
    .join("");
}

function renderTools() {
  const root = $("tool-list");
  if (!state.tools.length) {
    root.innerHTML = `<div class="empty-hint">No tool calls yet</div>`;
    return;
  }
  root.innerHTML = state.tools
    .slice(0, 40)
    .map(
      (t) => `<div class="tool-card">
  <div class="tool-name">${escapeHtml(t.tool || "tool")} · ${escapeHtml(t.status || "")}</div>
  <div class="tool-sum">${escapeHtml(t.summary || t.id || "")}</div>
</div>`
    )
    .join("");
}

function selectSession(id) {
  state.selectedSession = id;
  renderThreads();
  renderTranscript();
  const sess = state.sessions.find((s) => s.id === id);
  if (sess?.cwd && !$("cwd").value) $("cwd").value = sess.cwd;
}

// ── Live events from backend ────────────────────────────────────────────
function handleControlEvent(ev) {
  if (!ev || typeof ev !== "object") {
    pushEvent(String(ev));
    return;
  }
  const type = ev.type || "event";
  const sid = ev.session_id || ev.sessionId;

  if (type === "agent_message" || type === "agentMessage") {
    appendTranscript(sid, "agent", ev.text || JSON.stringify(ev));
    pushEvent(`agent ${shortId(sid)}: ${(ev.text || "").slice(0, 80)}`);
  } else if (type === "tool_call" || type === "toolCall") {
    const te = ev.event || ev;
    const tool = te.tool || te.name || "tool";
    const summary = te.args_summary || te.argsSummary || te.result_summary || "";
    const status = te.status || "running";
    state.tools.unshift({
      id: te.id,
      tool,
      status,
      summary: String(summary).slice(0, 120),
      sessionId: sid,
    });
    if (state.tools.length > 80) state.tools.length = 80;
    renderTools();
    appendTranscript(
      sid,
      "tool",
      `${tool} [${status}]\n${String(summary).slice(0, 400)}`
    );
    pushEvent(`tool ${tool} · ${status}`);
  } else if (type === "plan_update" || type === "planUpdate") {
    const pe = ev.event || ev;
    const steps = (pe.steps || [])
      .map((s) => `  - [${s.status || "pending"}] ${s.description || s.id}`)
      .join("\n");
    appendTranscript(sid, "plan", `${pe.title || "plan"} (${pe.status || ""})\n${steps}`);
    pushEvent(`plan update ${shortId(sid)}`);
  } else if (type === "session_created" || type === "sessionCreated") {
    pushEvent(`session created ${shortId(sid)}`, "ok");
    refreshSessions();
  } else if (type === "session_status_changed" || type === "sessionStatusChanged") {
    pushEvent(`status ${shortId(sid)} → ${ev.status}`);
    refreshSessions();
  } else if (type === "session_cancelled" || type === "sessionCancelled") {
    appendTranscript(sid, "system", "session cancelled");
    pushEvent(`cancelled ${shortId(sid)}`);
    refreshSessions();
  } else if (type === "error") {
    appendTranscript(sid || state.selectedSession, "error", ev.message || "error");
    pushEvent(ev.message || "error", "err");
    setStatus(state.ready ? "ready" : "error", ev.message || "error");
  } else if (type === "approval_required" || type === "approvalRequired") {
    appendTranscript(
      sid,
      "system",
      `approval required: ${ev.tool || "?"} — ${ev.summary || ev.request_id || ""}`
    );
    pushEvent(`approval · ${ev.tool || "?"}`, "err");
  } else {
    pushEvent(`${type} ${shortId(sid || "")}`);
  }
}

// ── API actions ─────────────────────────────────────────────────────────
async function refreshStatus() {
  const s = await invoke("get_runtime_status");
  state.ready = !!s.ready;
  setStatus(s.ready ? "ready" : "error", s.message);
  if (s.defaultCwd && !$("cwd").value) {
    $("cwd").value = s.defaultCwd;
    $("repo").value = s.defaultCwd;
  }
  $("sys-out").textContent = JSON.stringify(s, null, 2);
  return s;
}

async function refreshSessions() {
  try {
    const list = await invoke("list_sessions");
    state.sessions = Array.isArray(list) ? list : [];
    renderThreads();
    renderAgents();
    if (state.selectedSession && !state.sessions.some((s) => s.id === state.selectedSession)) {
      state.selectedSession = state.sessions[0]?.id || null;
    }
    if (!state.selectedSession && state.sessions.length) {
      state.selectedSession = state.sessions[0].id;
    }
    renderTranscript();
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

async function startAcp() {
  try {
    const cwd = $("cwd").value.trim();
    if (!cwd) throw new Error("Set project cwd (absolute path)");
    const rawModel = $("model")?.value?.trim?.() || "";
    const model = !rawModel || rawModel.toLowerCase() === "default" ? null : rawModel;
    const mcpNames = parseCsv($("mcp-attach-session")?.value || "");
    const highRisk = mcpNames.filter((n) =>
      /playwright|browser|grok-build|custom|^x$/i.test(n)
    );
    const opts = {
      mode: "acp",
      model,
      planMode: $("plan-mode").checked,
      alwaysApprove: $("always-approve").checked,
      mcpServerNames: mcpNames,
      approvedHighRiskMcp: highRisk,
      includeAutoMcp: false,
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
    state.selectedSession = res.id;
    appendTranscript(res.id, "system", `session started · cwd ${cwd}`);
    pushEvent(`ACP session ${shortId(res.id)}`, "ok");
    await refreshSessions();
    selectSession(res.id);
  } catch (e) {
    toastError(e);
  }
}

async function sendPrompt() {
  try {
    if (!state.selectedSession) throw new Error("Select a thread first");
    const prompt = $("prompt").value;
    if (!prompt.trim()) throw new Error("Empty prompt");
    appendTranscript(state.selectedSession, "user", prompt);
    $("prompt").value = "";
    await invoke("send_prompt", { id: state.selectedSession, prompt });
    pushEvent(`prompt → ${shortId(state.selectedSession)}`, "ok");
  } catch (e) {
    toastError(e);
  }
}

// Wire buttons
$("btn-new-session").onclick = startAcp;
$("btn-start-acp").onclick = startAcp;
$("btn-send").onclick = sendPrompt;
$("btn-cancel").onclick = async () => {
  try {
    if (!state.selectedSession) throw new Error("No session selected");
    await invoke("cancel_session", { id: state.selectedSession });
    appendTranscript(state.selectedSession, "system", "cancel requested");
    await refreshSessions();
  } catch (e) {
    toastError(e);
  }
};
$("btn-refresh").onclick = () => {
  refreshStatus().catch(toastError);
  refreshSessions();
};

$("prompt").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    sendPrompt();
  }
});

// MCP view
$("btn-mcp-list").onclick = async () => {
  try {
    $("mcp-out").textContent = JSON.stringify(await invoke("list_mcp_servers"), null, 2);
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-catalog").onclick = async () => {
  try {
    $("mcp-out").textContent = JSON.stringify(await invoke("list_mcp_catalog"), null, 2);
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-doctor").onclick = async () => {
  try {
    $("mcp-out").textContent = JSON.stringify(
      await invoke("doctor_mcp_server", { name: null }),
      null,
      2
    );
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-tools").onclick = async () => {
  try {
    $("mcp-out").textContent = JSON.stringify(
      await invoke("list_mcp_tools", { name: null }),
      null,
      2
    );
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
      autoAttach: false,
      requiresApproval: ["browser", "grok_build", "custom", "x_twitter"].includes(fromCatalog),
      headers: null,
      startupTimeoutSec: null,
      toolTimeoutSec: null,
      rateLimitPerMin: fromCatalog === "grok_build" ? 10 : null,
      credentialKeys: null,
    };
    $("mcp-out").textContent = JSON.stringify(
      await invoke("add_mcp_server", { request }),
      null,
      2
    );
  } catch (e) {
    toastError(e);
  }
};
$("btn-mcp-remove").onclick = async () => {
  try {
    await invoke("remove_mcp_server", { name: $("mcp-name").value });
    $("mcp-out").textContent = JSON.stringify(await invoke("list_mcp_servers"), null, 2);
  } catch (e) {
    toastError(e);
  }
};
$("btn-cred-set").onclick = async () => {
  try {
    await invoke("set_mcp_credential", {
      key: $("cred-key").value.trim(),
      value: $("cred-value").value,
    });
    $("cred-value").value = "";
    $("mcp-out").textContent = JSON.stringify(await invoke("list_mcp_credentials"), null, 2);
  } catch (e) {
    toastError(e);
  }
};

// Worktrees
$("btn-wt-list").onclick = async () => {
  try {
    $("wt-out").textContent = JSON.stringify(
      await invoke("list_worktrees", { repo: $("repo").value }),
      null,
      2
    );
  } catch (e) {
    toastError(e);
  }
};
$("btn-wt-create").onclick = async () => {
  try {
    $("wt-out").textContent = JSON.stringify(
      await invoke("create_worktree", {
        repo: $("repo").value,
        name: $("wt-name").value,
        baseRef: null,
      }),
      null,
      2
    );
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
    $("mem-out").textContent = JSON.stringify(
      await invoke("memory_list", { scope: $("mem-scope").value }),
      null,
      2
    );
  } catch (e) {
    toastError(e);
  }
};
$("btn-mem-list").onclick = async () => {
  try {
    $("mem-out").textContent = JSON.stringify(
      await invoke("memory_list", { scope: $("mem-scope").value }),
      null,
      2
    );
  } catch (e) {
    toastError(e);
  }
};
$("btn-mem-flush").onclick = async () => {
  try {
    $("mem-out").textContent = await invoke("memory_flush", { scope: $("mem-scope").value });
  } catch (e) {
    toastError(e);
  }
};

// System
$("btn-status").onclick = () => refreshStatus().catch(toastError);
$("btn-baseline").onclick = async () => {
  try {
    $("sys-out").textContent = JSON.stringify(await invoke("capture_baseline"), null, 2);
  } catch (e) {
    toastError(e);
  }
};
$("btn-config").onclick = async () => {
  try {
    $("sys-out").textContent = JSON.stringify(await invoke("get_config"), null, 2);
  } catch (e) {
    toastError(e);
  }
};
$("btn-shutdown").onclick = async () => {
  try {
    await invoke("shutdown_all");
    state.selectedSession = null;
    await refreshSessions();
    pushEvent("shutdown all", "ok");
  } catch (e) {
    toastError(e);
  }
};

async function boot() {
  if (hasTauri() && window.__TAURI__.event) {
    await window.__TAURI__.event.listen("control-event", (e) => handleControlEvent(e.payload));
  } else {
    setStatus("error", "Not inside Tauri — use the .app");
  }
  try {
    await refreshStatus();
    await refreshSessions();
    pushEvent("control panel ready", "ok");
  } catch (e) {
    toastError(e);
  }
}

boot();
