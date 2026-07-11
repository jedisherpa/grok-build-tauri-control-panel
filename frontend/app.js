// Bomb Code — three-column Grok Build control panel.
// Pixel-bomb visual language: moods for thinking / tools / boom / wait.

const $ = (id) => document.getElementById(id);

const LOGO = "assets/logo.png";

const state = {
  selectedSession: null,
  sessions: [],
  tools: [], // recent tool calls
  ready: false,
  auth: null,
  loggingIn: false,
  devServer: null,
  transcriptBySession: new Map(), // id -> [{role, body, at}]
  // bomb UI
  turnPhase: "idle", // idle | thinking | running | tooling | stream | wait | boom | error
  lastTool: null,
  thinkingSince: null,
  phraseTimer: null,
  phraseIndex: 0,
};

// ── Pixel-bomb language ─────────────────────────────────────────────────
const BOMB_PHRASES = {
  thinking: [
    "lighting the fuse",
    "packing powder",
    "defusing the problem",
    "wiring the charge",
    "counting down",
    "thinking in pixels",
    "polishing the casing",
    "arming the plan",
    "waiting for the boom of insight",
    "snipping red wires only",
    "shaking the pixel bomb",
    "loading agent payload",
  ],
  tooling: [
    "planting a charge",
    "cutting a wire",
    "detonating a tool",
    "rigging the workbench",
    "dropping a payload",
    "running the blast plan",
  ],
  running: [
    "fuse crackling",
    "agent on the wire",
    "charge is live",
    "still cooking",
    "stream is hot",
  ],
  stream: [
    "words falling like sparks",
    "streaming the boom",
    "agent is talking",
  ],
  wait: [
    "holding the pin",
    "approval fuse lit",
    "your move, bomb squad",
  ],
  boom: ["boom — turn complete", "charge spent", "clean detonation"],
  error: ["dud fuse", "misfire", "smoke in the bay"],
  idle: ["standby", "safe and sound"],
};

const BOMB_SUB = {
  thinking: "agent is planning · stream stays open",
  tooling: "tool call in flight",
  running: "session running",
  stream: "receiving agent tokens",
  wait: "waiting for your approval",
  boom: "ready for the next charge",
  error: "check the event feed",
  idle: "no live fuse",
};

function bombHtml(mood = "idle", size = "sm", extraClass = "") {
  return `<span class="px-bomb ${size} mood-${mood} ${extraClass}" aria-hidden="true"><img src="${LOGO}" alt="" /></span>`;
}

function moodFromStatus(status) {
  const s = String(status || "").toLowerCase();
  if (s.includes("run") || s.includes("generat") || s.includes("busy")) return "running";
  if (s.includes("wait") || s.includes("approv")) return "wait";
  if (s.includes("fail") || s.includes("error")) return "error";
  if (s.includes("cancel")) return "error";
  if (s.includes("idle") || s.includes("ready") || s.includes("complete")) return "ready";
  return "idle";
}

function moodFromEventCls(cls) {
  if (cls === "err") return "error";
  if (cls === "ok") return "boom";
  return "idle";
}

function setBombMood(el, mood) {
  if (!el) return;
  const moods = [
    "idle",
    "ready",
    "thinking",
    "running",
    "tooling",
    "stream",
    "boom",
    "error",
    "wait",
  ];
  el.classList.remove(...moods.map((m) => `mood-${m}`));
  el.classList.add(`mood-${mood}`);
}

function anySessionBusy() {
  return state.sessions.some((s) => {
    const st = String(s.status || "").toLowerCase();
    return st.includes("run") || st.includes("wait");
  });
}

function selectedBusy() {
  if (!state.selectedSession) return false;
  const s = state.sessions.find((x) => x.id === state.selectedSession);
  if (!s) return state.turnPhase !== "idle" && state.turnPhase !== "boom";
  const st = String(s.status || "").toLowerCase();
  return (
    st.includes("run") ||
    st.includes("wait") ||
    ["thinking", "running", "tooling", "stream", "wait"].includes(state.turnPhase)
  );
}

function setTurnPhase(phase, detail = "") {
  state.turnPhase = phase;
  if (phase === "thinking" || phase === "running" || phase === "tooling" || phase === "stream") {
    if (!state.thinkingSince) state.thinkingSince = Date.now();
  }
  if (phase === "idle" || phase === "boom" || phase === "error") {
    state.thinkingSince = null;
  }
  if (detail) state.lastTool = detail;
  updateBombChrome();
}

function pickPhrase(phase) {
  const list = BOMB_PHRASES[phase] || BOMB_PHRASES.idle;
  return list[state.phraseIndex % list.length];
}

function startPhraseCycle() {
  if (state.phraseTimer) return;
  state.phraseTimer = setInterval(() => {
    if (!selectedBusy() && state.turnPhase === "idle") return;
    state.phraseIndex += 1;
    const phraseEl = $("thinking-phrase");
    if (phraseEl) phraseEl.textContent = pickPhrase(state.turnPhase);
  }, 2200);
}

function stopPhraseCycle() {
  if (state.phraseTimer) {
    clearInterval(state.phraseTimer);
    state.phraseTimer = null;
  }
}

function elapsedLabel() {
  if (!state.thinkingSince) return "";
  const sec = Math.floor((Date.now() - state.thinkingSince) / 1000);
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}m ${s}s`;
}

function updateBombChrome() {
  const busy = selectedBusy();
  const anyBusy = anySessionBusy() || busy;
  const phase = busy
    ? state.turnPhase === "idle"
      ? "running"
      : state.turnPhase
    : state.turnPhase === "boom" || state.turnPhase === "error"
      ? state.turnPhase
      : "idle";

  // Brand fuse
  const brand = $("brand-header");
  if (brand) brand.classList.toggle("live", anyBusy);
  const brandSub = $("brand-sub");
  if (brandSub) {
    brandSub.textContent = anyBusy ? "fuse is lit…" : "Grok Build panel";
  }

  // Thinking strip
  const strip = $("thinking-strip");
  if (strip) {
    const show =
      busy &&
      ["thinking", "running", "tooling", "stream", "wait"].includes(phase);
    strip.classList.toggle("visible", show);
    strip.setAttribute("aria-hidden", show ? "false" : "true");
    if (show) {
      setBombMood($("thinking-bomb"), phase === "wait" ? "wait" : phase === "tooling" ? "tooling" : "thinking");
      const phraseEl = $("thinking-phrase");
      if (phraseEl) phraseEl.textContent = pickPhrase(phase);
      const sub = $("thinking-sub");
      if (sub) {
        const el = elapsedLabel();
        const base = BOMB_SUB[phase] || BOMB_SUB.thinking;
        const toolBit = state.lastTool && phase === "tooling" ? ` · ${state.lastTool}` : "";
        sub.textContent = el ? `${base}${toolBit} · ${el}` : `${base}${toolBit}`;
      }
      startPhraseCycle();
    } else if (!anyBusy) {
      stopPhraseCycle();
    }
  }

  // Composer rail
  const composer = $("composer");
  if (composer) composer.classList.toggle("busy", busy);
  const moodEl = $("composer-mood");
  if (moodEl) {
    if (busy) {
      moodEl.style.display = "inline-flex";
      moodEl.innerHTML = `${bombHtml(phase === "tooling" ? "tooling" : "thinking", "xs")} ${escapeHtml(pickPhrase(phase))}`;
    } else {
      moodEl.style.display = "none";
      moodEl.innerHTML = "";
    }
  }

  // Activity header bomb
  setBombMood($("activity-bomb"), anyBusy ? (phase === "tooling" ? "tooling" : "running") : "idle");
}

function flashBoomThenIdle(ms = 900) {
  setTurnPhase("boom");
  setTimeout(() => {
    if (state.turnPhase === "boom") setTurnPhase("idle");
  }, ms);
}

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
  const k = kind || "unknown";
  pill.className = `status-pill status-${k}`;
  $("status-text").textContent = text;
  // Map runtime status → bomb mood
  let mood = "idle";
  if (k === "ready") mood = anySessionBusy() || selectedBusy() ? "running" : "ready";
  else if (k === "error") mood = "error";
  else if (k === "thinking" || k === "running") mood = "thinking";
  else if (k === "unknown") mood = "idle";
  setBombMood($("status-bomb"), mood);
}

function pushEvent(text, cls = "", moodHint = null) {
  const feed = $("event-feed");
  const line = document.createElement("div");
  line.className = `event-line ${cls}`;
  const ts = new Date().toLocaleTimeString();
  const mood = moodHint || moodFromEventCls(cls);
  line.innerHTML = `${bombHtml(mood, "xs")}<span class="event-body"><span class="ts">${ts}</span>${escapeHtml(text)}</span>`;
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

function roleBombMood(role) {
  if (role === "user") return "idle";
  if (role === "agent") return "stream";
  if (role === "tool") return "tooling";
  if (role === "plan") return "thinking";
  if (role === "error") return "error";
  if (role === "system") return "ready";
  return "idle";
}

function renderTranscript() {
  const root = $("transcript");
  const sid = state.selectedSession;
  if (!sid) {
    root.innerHTML = `<div class="welcome">
<div class="welcome-hero">
  ${bombHtml("ready", "xl")}
  <pre class="banner">  ╔══════════════════════════════════════╗
  ║              bomb code               ║
  ╚══════════════════════════════════════╝</pre>
</div>
<p>Select a thread or start a new ACP session.</p>
<p class="muted">Pixel bombs track thinking, tools, and turn status while you wait.</p>
</div>`;
    $("composer-session").textContent = "no session";
    $("composer-model").textContent = "";
    updateBombChrome();
    return;
  }

  const sess = state.sessions.find((s) => s.id === sid);
  $("composer-session").textContent = `${shortId(sid)} · ${sess?.status || "?"}`;
  $("composer-model").textContent = sess?.model || "";

  const entries = getTranscript(sid);
  if (!entries.length) {
    root.innerHTML = `<div class="welcome">
<div class="welcome-hero">
  ${bombHtml("ready", "lg")}
  <pre class="banner">session ${escapeHtml(shortId(sid))}</pre>
</div>
<p class="muted">${escapeHtml(sess?.cwd || "")}</p>
<p>Connected. Type a prompt — the fuse lights while Grok works.</p>
</div>`;
    updateBombChrome();
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
  <div class="t-role">${bombHtml(roleBombMood(role), "xs")}<span>${label}</span></div>
  <div class="t-body">${escapeHtml(e.body)}</div>
  <div class="t-time">${escapeHtml(e.at || "")}</div>
</div>`;
    })
    .join("");
  root.scrollTop = root.scrollHeight;
  updateBombChrome();
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
      const bombMood = isMock ? "idle" : moodFromStatus(status);
      const cwd = s.cwd || "";
      const shortCwd = cwd.length > 28 ? "…" + cwd.slice(-27) : cwd;
      return `<div class="thread-item ${selected}" data-id="${escapeHtml(id)}">
  <div class="name">${bombHtml(bombMood, "xs")} ${escapeHtml(mode)} · ${escapeHtml(shortId(id))}</div>
  <div class="meta"><span class="badge ${badgeCls}">${bombHtml(bombMood, "xs")}${escapeHtml(status)}</span>
  <span>${escapeHtml(isMock ? "mock" : model || "—")}</span></div>
  <div class="meta">${escapeHtml(shortCwd)}</div>
</div>`;
    })
    .join("");

  root.querySelectorAll(".thread-item").forEach((el) => {
    el.onclick = () => selectSession(el.dataset.id);
  });
  updateBombChrome();
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
      const bombMood = moodFromStatus(status);
      const runCls = status.includes("run") ? "running" : "";
      return `<div class="agent-card ${runCls}">
  <div class="name">${bombHtml(bombMood, "sm")}${escapeHtml(String(s.mode || "acp").toUpperCase())} · ${escapeHtml(shortId(s.id))}</div>
  <div class="meta"><span class="badge ${badgeCls}">${bombHtml(bombMood, "xs")}${escapeHtml(status)}</span>
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
  updateBombChrome();
}

function renderTools() {
  const root = $("tool-list");
  if (!state.tools.length) {
    root.innerHTML = `<div class="empty-hint">No tool calls yet</div>`;
    return;
  }
  root.innerHTML = state.tools
    .slice(0, 40)
    .map((t) => {
      const st = String(t.status || "").toLowerCase();
      const mood = st.includes("run") || st.includes("start")
        ? "tooling"
        : st.includes("fail") || st.includes("error")
          ? "error"
          : st.includes("done") || st.includes("complete") || st.includes("ok")
            ? "boom"
            : "tooling";
      const runCls = mood === "tooling" ? "running" : "";
      return `<div class="tool-card ${runCls}">
  <div class="tool-name">${bombHtml(mood, "xs")}${escapeHtml(t.tool || "tool")} · ${escapeHtml(t.status || "")}</div>
  <div class="tool-sum">${escapeHtml(t.summary || t.id || "")}</div>
</div>`;
    })
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
    pushEvent(String(ev), "", "idle");
    return;
  }
  const type = ev.type || "event";
  const sid = ev.session_id || ev.sessionId;
  const isSelected = !sid || sid === state.selectedSession;

  if (type === "agent_message" || type === "agentMessage") {
    appendTranscript(sid, "agent", ev.text || JSON.stringify(ev));
    pushEvent(`agent ${shortId(sid)}: ${(ev.text || "").slice(0, 80)}`, "", "stream");
    if (isSelected) setTurnPhase("stream");
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
    const st = String(status).toLowerCase();
    const done = st.includes("done") || st.includes("complete") || st.includes("success");
    pushEvent(`tool ${tool} · ${status}`, done ? "ok" : "", done ? "boom" : "tooling");
    if (isSelected) setTurnPhase(done ? "running" : "tooling", tool);
  } else if (type === "plan_update" || type === "planUpdate") {
    const pe = ev.event || ev;
    const steps = (pe.steps || [])
      .map((s) => `  - [${s.status || "pending"}] ${s.description || s.id}`)
      .join("\n");
    appendTranscript(sid, "plan", `${pe.title || "plan"} (${pe.status || ""})\n${steps}`);
    pushEvent(`plan update ${shortId(sid)}`, "", "thinking");
    if (isSelected) setTurnPhase("thinking");
  } else if (type === "session_created" || type === "sessionCreated") {
    pushEvent(`session created ${shortId(sid)}`, "ok", "boom");
    refreshSessions();
  } else if (type === "session_status_changed" || type === "sessionStatusChanged") {
    const st = String(ev.status || "").toLowerCase();
    pushEvent(`status ${shortId(sid)} → ${ev.status}`, "", moodFromStatus(st));
    if (isSelected) {
      if (st.includes("run")) setTurnPhase(state.turnPhase === "tooling" ? "tooling" : "running");
      else if (st.includes("wait") || st.includes("approv")) setTurnPhase("wait");
      else if (st.includes("fail") || st.includes("error")) setTurnPhase("error");
      else if (st.includes("idle") || st.includes("complete") || st.includes("cancel")) {
        if (st.includes("cancel")) setTurnPhase("error");
        else flashBoomThenIdle();
      }
    }
    refreshSessions();
  } else if (type === "session_cancelled" || type === "sessionCancelled") {
    appendTranscript(sid, "system", "session cancelled");
    pushEvent(`cancelled ${shortId(sid)}`, "", "error");
    if (isSelected) setTurnPhase("error");
    refreshSessions();
  } else if (type === "error") {
    appendTranscript(sid || state.selectedSession, "error", ev.message || "error");
    pushEvent(ev.message || "error", "err", "error");
    setStatus(state.ready ? "ready" : "error", ev.message || "error");
    if (isSelected) setTurnPhase("error");
  } else if (type === "approval_required" || type === "approvalRequired") {
    appendTranscript(
      sid,
      "system",
      `approval required: ${ev.tool || "?"} — ${ev.summary || ev.request_id || ""}`
    );
    pushEvent(`approval · ${ev.tool || "?"}`, "err", "wait");
    if (isSelected) setTurnPhase("wait", ev.tool || "approval");
  } else {
    pushEvent(`${type} ${shortId(sid || "")}`, "", "idle");
  }
}

// ── API actions ─────────────────────────────────────────────────────────
function renderAuth(auth) {
  state.auth = auth;
  const label = $("auth-label");
  const loginBtn = $("btn-login");
  const logoutBtn = $("btn-logout");
  const hint = $("auth-hint");
  const panel = $("auth-code-panel");
  if (!auth) {
    label.textContent = "Auth unknown";
    return;
  }
  if (auth.loggedIn) {
    const name = auth.email || auth.firstName || "Grok user";
    label.textContent = name;
    label.title = auth.message || name;
    loginBtn.style.display = "none";
    logoutBtn.style.display = "block";
    if (panel) panel.style.display = "none";
    hint.textContent = auth.authMode ? `via ${auth.authMode}` : "Signed in";
    state.loggingIn = false;
  } else {
    label.textContent = "Not signed in";
    loginBtn.style.display = "block";
    logoutBtn.style.display = "none";
    loginBtn.disabled = !!state.loggingIn;
    loginBtn.textContent = state.loggingIn ? "Login in progress…" : "Log in with Grok";
    if (!state.loggingIn) {
      hint.textContent = "Opens Grok / xAI sign-in. You’ll confirm a code in the browser.";
      if (panel) panel.style.display = "none";
    }
  }
}

function renderLoginSession(st) {
  if (!st) return;
  const panel = $("auth-code-panel");
  const codeEl = $("auth-confirm-code");
  const hint = $("auth-hint");
  const loginBtn = $("btn-login");

  if (st.status?.loggedIn || st.phase === "completed") {
    state.loggingIn = false;
    renderAuth(st.status);
    if (panel) panel.style.display = "none";
    pushEvent(st.status?.message || "Signed in with Grok", "ok");
    return;
  }

  if (st.phase === "failed" && !st.active) {
    state.loggingIn = false;
    loginBtn.disabled = false;
    loginBtn.textContent = "Log in with Grok";
    if (panel) panel.style.display = "none";
    hint.textContent = st.instructions || "Login failed — try again.";
    return;
  }

  state.loggingIn = true;
  loginBtn.disabled = true;
  loginBtn.textContent = "Login in progress…";
  if (panel) panel.style.display = "flex";
  if (st.confirmCode) {
    codeEl.textContent = st.confirmCode;
  } else {
    codeEl.textContent = "…";
  }
  hint.textContent = st.instructions || "";
  if (st.loginUrl) {
    $("btn-open-login-url").dataset.url = st.loginUrl;
  }
}

let loginPollTimer = null;
function stopLoginPoll() {
  if (loginPollTimer) {
    clearInterval(loginPollTimer);
    loginPollTimer = null;
  }
}

function startLoginPoll() {
  stopLoginPoll();
  loginPollTimer = setInterval(async () => {
    try {
      const st = await invoke("grok_login_status");
      renderLoginSession(st);
      if (st.status?.loggedIn || st.phase === "completed") {
        stopLoginPoll();
        state.loggingIn = false;
        await refreshStatus();
      } else if (st.phase === "failed" && !st.active) {
        stopLoginPoll();
        state.loggingIn = false;
      }
    } catch (e) {
      /* ignore poll errors */
    }
  }, 1000);
}

async function refreshAuth() {
  try {
    const auth = await invoke("get_auth_status");
    renderAuth(auth);
    return auth;
  } catch (e) {
    renderAuth({
      loggedIn: false,
      message: String(e.message || e),
    });
    throw e;
  }
}

async function refreshStatus() {
  const s = await invoke("get_runtime_status");
  state.ready = !!s.ready;
  setStatus(s.ready ? "ready" : "error", s.message);
  if (s.defaultCwd && !$("cwd").value) {
    $("cwd").value = s.defaultCwd;
    if ($("repo")) $("repo").value = s.defaultCwd;
  }
  if ($("sys-out")) $("sys-out").textContent = JSON.stringify(s, null, 2);
  await refreshAuth().catch(() => {});
  return s;
}

async function loginWithGrok() {
  if (state.loggingIn) return;
  state.loggingIn = true;
  renderAuth(state.auth || { loggedIn: false });
  pushEvent("Starting Grok login…");
  $("auth-hint").textContent = "Starting login…";
  $("auth-paste-code").value = "";
  try {
    let st;
    try {
      st = await invoke("start_grok_login");
    } catch (e1) {
      pushEvent(`device login start failed: ${e1.message || e1}`, "err");
      st = await invoke("start_grok_login_oauth");
    }
    renderLoginSession(st);
    if (st.confirmCode) {
      pushEvent(`Confirm code in browser: ${st.confirmCode}`, "ok");
    }
    if (st.loginUrl) {
      pushEvent("Login page opened", "ok");
    }
    startLoginPoll();
  } catch (e) {
    state.loggingIn = false;
    toastError(e);
    $("auth-hint").textContent = String(e.message || e);
    $("btn-login").disabled = false;
    $("btn-login").textContent = "Log in with Grok";
  }
}

async function submitLoginCode() {
  const code = $("auth-paste-code").value.trim();
  if (!code) {
    $("auth-hint").textContent = "Paste the code from the browser first.";
    return;
  }
  $("auth-hint").textContent = "Submitting code…";
  try {
    const st = await invoke("submit_grok_login_code", { code });
    renderLoginSession(st);
    if (st.status?.loggedIn || st.phase === "completed") {
      stopLoginPoll();
      state.loggingIn = false;
      pushEvent(st.status?.message || "Signed in", "ok");
      await refreshStatus();
    } else {
      $("auth-hint").textContent =
        st.instructions || "Code submitted — finish any remaining steps in the browser.";
      startLoginPoll();
    }
  } catch (e) {
    toastError(e);
    $("auth-hint").textContent = String(e.message || e);
  }
}

async function cancelLogin() {
  try {
    await invoke("cancel_grok_login");
  } catch (_) {
    /* ignore */
  }
  stopLoginPoll();
  state.loggingIn = false;
  $("auth-code-panel").style.display = "none";
  $("auth-paste-code").value = "";
  $("btn-login").disabled = false;
  $("btn-login").textContent = "Log in with Grok";
  $("auth-hint").textContent = "Login cancelled.";
  await refreshAuth().catch(() => {});
}

async function logoutGrok() {
  try {
    stopLoginPoll();
    const status = await invoke("logout_grok");
    renderAuth(status);
    $("auth-code-panel").style.display = "none";
    pushEvent("Signed out", "ok");
    await refreshStatus();
  } catch (e) {
    toastError(e);
  }
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
    const auth = state.auth || (await refreshAuth().catch(() => null));
    if (auth && !auth.loggedIn) {
      const go = confirm("Not signed in with Grok. Log in now?");
      if (go) {
        await loginWithGrok();
        if (!state.auth?.loggedIn) throw new Error("Login required before starting a session");
      } else {
        throw new Error("Sign in with Grok first");
      }
    }
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
    state.phraseIndex = Math.floor(Math.random() * BOMB_PHRASES.thinking.length);
    state.thinkingSince = Date.now();
    setTurnPhase("thinking");
    setStatus("thinking", "lighting the fuse…");
    pushEvent(`prompt → ${shortId(state.selectedSession)}`, "ok", "thinking");
    await invoke("send_prompt", { id: state.selectedSession, prompt });
    // Stay in thinking/running until status events arrive
    if (state.turnPhase === "thinking") setTurnPhase("running");
    setStatus(state.ready ? "ready" : "error", state.ready ? "fuse lit · agent working" : "error");
    updateBombChrome();
  } catch (e) {
    setTurnPhase("error");
    toastError(e);
  }
}

// Wire buttons
$("btn-new-session").onclick = startAcp;
$("btn-start-acp").onclick = startAcp;
$("btn-login").onclick = loginWithGrok;
$("btn-logout").onclick = logoutGrok;

// ── New project folder (top bar) ────────────────────────────────────────
const MYSTIC_NAMES = [
  "heraclitus",
  "plotinus",
  "hypatia",
  "rumi",
  "ibn-arabi",
  "hildegard",
  "mechtild",
  "eckhart",
  "boehme",
  "spinoza",
  "lao-tzu",
  "zhuangzi",
  "nagarjuna",
  "padmasambhava",
  "milarepa",
  "dogen",
  "hafez",
  "attar",
  "avicenna",
  "maimonides",
  "paradoxa",
  "gnosis-well",
  "void-mirror",
  "sophia-code",
  "alembic",
  "hermetica",
  "kabir",
  "teresa",
  "john-of-the-cross",
  "simone-weil",
  "blavatsky",
  "gurdjieff",
  "ouroboros",
  "emerald-tablet",
  "night-sea",
  "oracle-bone",
  "quinque-viae",
  "pneuma-lab",
  "axis-mundi",
  "enochian",
];

function shufflePick(arr, n) {
  const copy = [...arr];
  for (let i = copy.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy.slice(0, n);
}

function renderFolderSuggestions() {
  const root = $("folder-suggestions");
  if (!root) return;
  const picks = shufflePick(MYSTIC_NAMES, 8);
  root.innerHTML = picks
    .map(
      (name) =>
        `<button type="button" class="folder-chip" data-name="${escapeHtml(name)}">${escapeHtml(
          name
        )}</button>`
    )
    .join("");
  root.querySelectorAll(".folder-chip").forEach((chip) => {
    chip.onclick = () => {
      root.querySelectorAll(".folder-chip").forEach((c) => c.classList.remove("selected"));
      chip.classList.add("selected");
      $("new-folder-name").value = chip.dataset.name;
      $("new-folder-name").focus();
    };
  });
}

function toggleNewFolderPanel(show) {
  const panel = $("new-folder-panel");
  if (!panel) return;
  const open = show ?? panel.style.display === "none";
  panel.style.display = open ? "flex" : "none";
  if (open) {
    renderFolderSuggestions();
    $("new-folder-name").value = "";
    $("new-folder-name").focus();
  }
}

async function createProjectFolder() {
  const name = $("new-folder-name").value.trim();
  if (!name) {
    $("new-folder-name").focus();
    pushEvent("Enter a folder name or pick a suggestion", "err");
    return;
  }
  try {
    // Parent: sibling of current project under Projects/Code, else ~/Projects
    let parent = null;
    const cwd = $("cwd").value.trim().replace(/\/+$/, "");
    if (cwd) {
      const lower = cwd.toLowerCase();
      if (/\/(projects|code|developer|dev)$/i.test(cwd)) {
        parent = cwd;
      } else if (/\/(projects|code|developer|dev)\//i.test(lower)) {
        parent = cwd.replace(/\/[^/]+$/, "") || null;
      }
    }
    const res = await invoke("create_project_folder", { name, parent });
    $("cwd").value = res.path;
    if ($("repo")) $("repo").value = res.path;
    toggleNewFolderPanel(false);
    pushEvent(
      res.created
        ? `Created project folder ${res.name}`
        : `Using existing folder ${res.name}`,
      "ok"
    );
    appendTranscript(
      state.selectedSession,
      "system",
      `project folder ready: ${res.path}`
    );
  } catch (e) {
    toastError(e);
  }
}

$("btn-new-folder").onclick = () => toggleNewFolderPanel();
$("btn-create-folder").onclick = createProjectFolder;
$("btn-cancel-folder").onclick = () => toggleNewFolderPanel(false);
$("new-folder-name").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    createProjectFolder();
  } else if (e.key === "Escape") {
    toggleNewFolderPanel(false);
  }
});
$("btn-submit-code").onclick = submitLoginCode;
$("btn-cancel-login").onclick = cancelLogin;
$("btn-open-login-url").onclick = async () => {
  try {
    const url = await invoke("open_grok_login_url");
    if (!url && $("btn-open-login-url").dataset.url) {
      window.open($("btn-open-login-url").dataset.url, "_blank");
    }
  } catch (e) {
    const u = $("btn-open-login-url").dataset.url;
    if (u) window.open(u, "_blank");
    else toastError(e);
  }
};
$("auth-paste-code").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    submitLoginCode();
  }
});

// ── Dev server / live preview ───────────────────────────────────────────
function renderDevStatus(st) {
  state.devServer = st;
  const el = $("dev-status");
  const urlEl = $("dev-url");
  const openBtn = $("btn-dev-open");
  const stopBtn = $("btn-dev-stop");
  const startBtn = $("btn-dev-server");
  if (!st || !st.running) {
    el.textContent = st?.message || "Stopped";
    el.className = "dev-status muted";
    urlEl.style.display = "none";
    openBtn.style.display = "none";
    stopBtn.style.display = "none";
    startBtn.textContent = "Dev Server";
    startBtn.disabled = false;
    return;
  }
  el.textContent = st.message || `Running · ${st.url || ""}`;
  el.className = "dev-status running";
  if (st.url) {
    urlEl.style.display = "block";
    urlEl.textContent = st.url;
    urlEl.href = st.url;
  }
  openBtn.style.display = "inline-block";
  stopBtn.style.display = "inline-block";
  startBtn.textContent = "Restart";
}

function previewArgs() {
  const cwd = $("cwd")?.value?.trim?.() || null;
  return {
    cwd: cwd || null,
    sessionId: state.selectedSession || null,
  };
}

async function refreshDevStatus() {
  try {
    const st = await invoke("dev_server_status");
    renderDevStatus(st);
    return st;
  } catch (e) {
    /* ignore if backend old */
  }
}

async function startDevServer() {
  const btn = $("btn-dev-server");
  btn.disabled = true;
  btn.textContent = "Starting…";
  $("dev-status").textContent = "Detecting project…";
  try {
    const args = previewArgs();
    if (!args.cwd && !args.sessionId) {
      throw new Error("Select a session or set project cwd first");
    }
    let detect = null;
    try {
      detect = await invoke("detect_dev_server", args);
      pushEvent(`preview: ${detect.label || detect.kind}`, "ok");
      $("dev-status").textContent = `Starting ${detect.label || detect.kind}…`;
    } catch (e) {
      pushEvent(`detect: ${e.message || e}`, "err");
    }
    const st = await invoke("start_dev_server", {
      ...args,
      openBrowser: true,
    });
    renderDevStatus(st);
    pushEvent(st.message || "Dev server started", "ok");
    appendTranscript(
      state.selectedSession,
      "system",
      `dev server: ${st.url || st.message || "started"}\n${st.command || ""}`
    );
  } catch (e) {
    toastError(e);
    $("dev-status").textContent = String(e.message || e);
    btn.textContent = "Dev Server";
  } finally {
    btn.disabled = false;
    await refreshDevStatus();
  }
}

async function stopDevServer() {
  try {
    const st = await invoke("stop_dev_server");
    renderDevStatus(st);
    pushEvent("Dev server stopped");
  } catch (e) {
    toastError(e);
  }
}

async function openDevServer() {
  try {
    const url = await invoke("open_dev_server");
    pushEvent(`Opened ${url}`, "ok");
  } catch (e) {
    // fallback: open link if we have it
    if (state.devServer?.url) {
      window.open(state.devServer.url, "_blank");
    } else {
      toastError(e);
    }
  }
}

async function revealProject() {
  try {
    await invoke("reveal_project", previewArgs());
    pushEvent("Opened project folder", "ok");
  } catch (e) {
    toastError(e);
  }
}

$("btn-dev-server").onclick = startDevServer;
$("btn-dev-stop").onclick = stopDevServer;
$("btn-dev-open").onclick = openDevServer;
$("btn-dev-folder").onclick = revealProject;
$("btn-send").onclick = sendPrompt;
$("btn-cancel").onclick = async () => {
  try {
    if (!state.selectedSession) throw new Error("No session selected");
    setTurnPhase("wait");
    pushEvent("pulling the pin…", "", "wait");
    await invoke("cancel_session", { id: state.selectedSession });
    appendTranscript(state.selectedSession, "system", "cancel requested");
    setTurnPhase("error");
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

// Keep fuse meter / elapsed clock honest while waiting
setInterval(() => {
  if (selectedBusy() || state.thinkingSince) updateBombChrome();
}, 1000);

async function boot() {
  if (hasTauri() && window.__TAURI__.event) {
    await window.__TAURI__.event.listen("control-event", (e) => handleControlEvent(e.payload));
  } else {
    setStatus("error", "Not inside Tauri — use the .app");
    setBombMood($("status-bomb"), "error");
  }
  try {
    await refreshStatus();
    await refreshSessions();
    await refreshDevStatus();
    setTurnPhase("idle");
    pushEvent("Bomb Code ready — pixel bombs armed", "ok", "boom");
    if (state.auth && !state.auth.loggedIn) {
      pushEvent("Not signed in — click Log in with Grok", "", "wait");
    }
    updateBombChrome();
  } catch (e) {
    toastError(e);
  }
}

boot();
