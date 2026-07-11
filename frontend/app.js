// Bomb Code — three-column Grok Build control panel.
// Pixel-bomb visual language: moods for thinking / tools / boom / wait.

const $ = (id) => document.getElementById(id);

const LOGO = "assets/logo.png";

const STALL_MS = 25000; // no tokens/tools for this long → "waiting on agent"

const state = {
  selectedSession: null,
  sessions: [],
  tools: [], // recent tool calls
  ready: false,
  auth: null,
  loggingIn: false,
  devServer: null,
  transcriptBySession: new Map(), // id -> [{role, body, at}]
  transcriptLoaded: new Set(), // ids hydrated from SQLite
  /** Factual turn state — drives the dock (not fluff). */
  turn: emptyTurn(),
  phraseTimer: null,
  phraseIndex: 0,
  lastEventKey: "", // de-dupe noisy timeline
};

function emptyTurn() {
  return {
    phase: "idle", // idle | send | think | tools | reply | wait | done | error
    startedAt: null,
    lastSignalAt: null,
    promptChars: 0,
    streamChars: 0,
    thoughtChars: 0,
    toolCount: 0,
    lastTool: null,
    lastToolStatus: null,
    preview: "", // latest agent text snippet
    thoughtPreview: "",
    note: "",
  };
}

// Flavor only — never the primary status line.
const BOMB_FLAVOR = {
  send: ["fuse lit", "payload armed"],
  think: ["defusing the problem", "packing powder", "thinking in pixels"],
  tools: ["planting a charge", "cutting a wire", "rigging tools"],
  reply: ["words like sparks", "streaming the boom"],
  wait: ["holding the pin", "your move"],
  done: ["clean detonation", "charge spent"],
  error: ["dud fuse", "misfire"],
  idle: ["standby"],
};

const PHASE_LABEL = {
  idle: "Idle",
  send: "Sent",
  think: "Thinking",
  tools: "Using tools",
  reply: "Writing reply",
  wait: "Needs you",
  done: "Done",
  error: "Failed",
};

const PHASE_MOOD = {
  idle: "idle",
  send: "thinking",
  think: "thinking",
  tools: "tooling",
  reply: "stream",
  wait: "wait",
  done: "boom",
  error: "error",
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

function turnActive() {
  return ["send", "think", "tools", "reply", "wait"].includes(state.turn.phase);
}

function selectedBusy() {
  if (turnActive()) return true;
  if (!state.selectedSession) return false;
  const s = state.sessions.find((x) => x.id === state.selectedSession);
  if (!s) return false;
  const st = String(s.status || "").toLowerCase();
  return st.includes("run") || st.includes("wait");
}

function formatElapsed(ms) {
  if (ms == null || ms < 0) return "";
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function formatCount(n) {
  if (!n) return "0";
  if (n < 1000) return String(n);
  return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
}

function clipPreview(text, n = 96) {
  const t = String(text || "").replace(/\s+/g, " ").trim();
  if (!t) return "";
  return t.length <= n ? t : `…${t.slice(-n)}`;
}

function isNoiseAgentText(text) {
  const t = String(text || "").trim().toLowerCase();
  return (
    t.startsWith("prompt sent") ||
    t === "turn complete" ||
    t.startsWith("still generating after") ||
    t.startsWith("[local/mock]")
  );
}

/** Start or advance the turn with a concrete signal. */
function noteTurn(phase, patch = {}) {
  const t = state.turn;
  const now = Date.now();
  if (!t.startedAt && phase !== "idle" && phase !== "done") {
    t.startedAt = now;
  }
  // Don't go backwards: reply > tools > think > send (except wait/error/done)
  const rank = { idle: 0, send: 1, think: 2, tools: 3, reply: 4, wait: 5, done: 6, error: 6 };
  const cur = rank[t.phase] ?? 0;
  const next = rank[phase] ?? 0;
  if (phase === "wait" || phase === "error" || phase === "done" || phase === "idle") {
    t.phase = phase;
  } else if (next >= cur || t.phase === "wait") {
    t.phase = phase;
  }
  t.lastSignalAt = now;
  Object.assign(t, patch);
  if (phase === "idle") {
    state.turn = emptyTurn();
  }
  updateBombChrome();
}

function setTurnPhase(phase, detail = "") {
  // Back-compat wrapper used by older call sites
  const map = {
    thinking: "think",
    running: "think",
    tooling: "tools",
    stream: "reply",
    wait: "wait",
    boom: "done",
    error: "error",
    idle: "idle",
  };
  const p = map[phase] || phase;
  noteTurn(p, detail ? { lastTool: detail, note: detail } : {});
}

function flashBoomThenIdle(ms = 1200) {
  noteTurn("done", { note: "Turn finished" });
  setTimeout(() => {
    if (state.turn.phase === "done") noteTurn("idle");
  }, ms);
}

function pickFlavor(phase) {
  const list = BOMB_FLAVOR[phase] || BOMB_FLAVOR.idle;
  return list[state.phraseIndex % list.length];
}

function stageState(stage, phase) {
  const order = ["send", "think", "tools", "reply", "done"];
  const pi = order.indexOf(phase === "wait" ? "tools" : phase === "error" ? "done" : phase);
  const si = order.indexOf(stage);
  if (si < 0) return "";
  if (phase === "error" && stage === "done") return "error";
  if (si < pi) return "done";
  if (si === pi) return phase === "done" ? "done" : "active";
  return "";
}

function buildTurnDetail() {
  const t = state.turn;
  const bits = [];
  if (t.promptChars) bits.push(`prompt ${formatCount(t.promptChars)} chars`);
  if (t.phase === "think" && !t.streamChars && !t.toolCount) {
    bits.push("waiting for first token or tool");
  }
  if (t.thoughtChars) bits.push(`${formatCount(t.thoughtChars)} thought chars`);
  if (t.streamChars) bits.push(`${formatCount(t.streamChars)} reply chars`);
  if (t.toolCount) {
    bits.push(
      `${t.toolCount} tool${t.toolCount === 1 ? "" : "s"}${
        t.lastTool ? ` · last ${t.lastTool}` : ""
      }${t.lastToolStatus ? ` (${t.lastToolStatus})` : ""}`
    );
  }
  if (t.phase === "wait") bits.push(t.note || "approval required");
  if (t.phase === "error") bits.push(t.note || "see timeline");
  if (t.phase === "done") bits.push(t.note || "ready for next message");

  // Stall hint
  if (turnActive() && t.lastSignalAt) {
    const quiet = Date.now() - t.lastSignalAt;
    if (quiet >= STALL_MS) {
      bits.push(`no new signal for ${formatElapsed(quiet)}`);
    }
  }
  return bits.join(" · ") || "Working…";
}

function updateBombChrome() {
  const t = state.turn;
  const busy = turnActive() || selectedBusy();
  const anyBusy = anySessionBusy() || busy;
  const phase = turnActive()
    ? t.phase
    : t.phase === "done" || t.phase === "error"
      ? t.phase
      : "idle";
  const mood = PHASE_MOOD[phase] || "idle";
  const stalled =
    turnActive() && t.lastSignalAt && Date.now() - t.lastSignalAt >= STALL_MS;
  const elapsed = t.startedAt ? formatElapsed(Date.now() - t.startedAt) : "";

  // Brand: compact factual subline
  const brand = $("brand-header");
  if (brand) brand.classList.toggle("live", anyBusy);
  const brandSub = $("brand-sub");
  if (brandSub) {
    if (turnActive()) {
      brandSub.textContent = `${PHASE_LABEL[phase] || phase}${elapsed ? ` · ${elapsed}` : ""}`;
    } else {
      brandSub.textContent = "Grok Build panel";
    }
  }

  // Primary turn dock (center, above composer)
  const dock = $("turn-dock");
  if (dock) {
    const show = turnActive() || phase === "done" || phase === "error";
    dock.classList.toggle("visible", show);
    dock.classList.toggle("stalled", !!stalled);
    dock.classList.toggle(`phase-${phase}`, true);
    // clear other phase-* classes
    ["idle", "send", "think", "tools", "reply", "wait", "done", "error"].forEach((p) => {
      if (p !== phase) dock.classList.remove(`phase-${p}`);
    });
    dock.setAttribute("aria-hidden", show ? "false" : "true");

    setBombMood($("turn-bomb"), stalled && phase !== "wait" ? "running" : mood);

    const label = $("turn-phase-label");
    if (label) {
      label.textContent = stalled && phase !== "wait" ? "Still working" : PHASE_LABEL[phase] || phase;
    }
    const elEl = $("turn-elapsed");
    if (elEl) elEl.textContent = elapsed || "";

    const flavor = $("turn-flavor");
    if (flavor) {
      flavor.textContent = show && phase !== "idle" ? pickFlavor(phase) : "";
    }

    const detail = $("turn-detail");
    if (detail) detail.textContent = show ? buildTurnDetail() : "No active turn";

    const preview = $("turn-preview");
    if (preview) {
      const snip =
        phase === "think" && t.thoughtPreview
          ? t.thoughtPreview
          : t.preview || t.thoughtPreview;
      if (show && snip) {
        preview.style.display = "block";
        preview.textContent = snip;
      } else {
        preview.style.display = "none";
        preview.textContent = "";
      }
    }

    // Stages
    document.querySelectorAll("#turn-stages .stage").forEach((el) => {
      const st = el.getAttribute("data-stage");
      el.classList.remove("active", "done", "error");
      const cls = stageState(st, phase === "error" ? "error" : phase);
      if (cls) el.classList.add(cls);
    });
  }

  // Composer: one compact phase chip (not a second monologue)
  const composer = $("composer");
  if (composer) composer.classList.toggle("busy", turnActive());
  const phaseChip = $("composer-phase");
  if (phaseChip) {
    if (turnActive()) {
      phaseChip.style.display = "inline-flex";
      phaseChip.innerHTML = `${bombHtml(mood, "xs")}<span>${escapeHtml(
        PHASE_LABEL[phase] || phase
      )}${elapsed ? ` · ${elapsed}` : ""}</span>`;
    } else {
      phaseChip.style.display = "none";
      phaseChip.innerHTML = "";
    }
  }

  // Right "Now" panel — mirror of dock facts
  const nowPanel = $("now-panel");
  const nowElapsed = $("now-elapsed");
  if (nowElapsed) nowElapsed.textContent = turnActive() ? elapsed : "";
  if (nowPanel) {
    if (!turnActive() && phase !== "done" && phase !== "error") {
      nowPanel.innerHTML = `<div class="empty-hint">No live turn</div>`;
    } else {
      nowPanel.innerHTML = `
        <div class="now-row">
          ${bombHtml(mood, "sm")}
          <div class="now-copy">
            <div class="now-phase">${escapeHtml(PHASE_LABEL[phase] || phase)}</div>
            <div class="now-detail muted">${escapeHtml(buildTurnDetail())}</div>
          </div>
        </div>
        ${
          t.preview || t.thoughtPreview
            ? `<div class="now-preview">${escapeHtml(t.preview || t.thoughtPreview)}</div>`
            : ""
        }
        ${
          t.lastTool
            ? `<div class="now-tool">${bombHtml("tooling", "xs")}<span>${escapeHtml(
                t.lastTool
              )}${t.lastToolStatus ? ` · ${escapeHtml(t.lastToolStatus)}` : ""}</span></div>`
            : ""
        }`;
    }
  }

  // Activity header bomb
  setBombMood(
    $("activity-bomb"),
    anyBusy ? (phase === "tools" ? "tooling" : phase === "reply" ? "stream" : "running") : "idle"
  );

  // Status pill: host health only when idle; turn summary when busy
  if (turnActive() && state.ready) {
    const pill = $("status-pill");
    if (pill && !pill.classList.contains("status-error")) {
      setBombMood($("status-bomb"), mood);
      const st = $("status-text");
      if (st) {
        st.textContent = `${PHASE_LABEL[phase] || phase}${elapsed ? ` · ${elapsed}` : ""}`;
      }
    }
  }
}

function startPhraseCycle() {
  if (state.phraseTimer) return;
  state.phraseTimer = setInterval(() => {
    if (!turnActive()) return;
    state.phraseIndex += 1;
    const flavor = $("turn-flavor");
    if (flavor) flavor.textContent = pickFlavor(state.turn.phase);
    // Refresh stall clock / elapsed
    updateBombChrome();
  }, 2000);
}

function stopPhraseCycle() {
  if (state.phraseTimer) {
    clearInterval(state.phraseTimer);
    state.phraseTimer = null;
  }
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

function pushEvent(text, cls = "", moodHint = null, opts = {}) {
  const feed = $("event-feed");
  if (!feed) return;
  const key = `${cls}|${text}`;
  // Collapse spam (status flapping, token spam)
  if (!opts.force && key === state.lastEventKey && cls !== "err") return;
  state.lastEventKey = key;
  const line = document.createElement("div");
  line.className = `event-line ${cls}`;
  const ts = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const mood = moodHint || moodFromEventCls(cls);
  line.innerHTML = `${bombHtml(mood, "xs")}<span class="event-body"><span class="ts">${ts}</span>${escapeHtml(text)}</span>`;
  feed.prepend(line);
  while (feed.children.length > 80) feed.lastChild.remove();
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

function appendTranscript(sessionId, role, body, at = nowIso(), opts = {}) {
  if (!sessionId) return;
  const list = getTranscript(sessionId);
  const text = body == null ? "" : String(body);
  const stream = !!opts.stream;

  // Coalesce streaming agent chunks into one bubble (like a live terminal).
  if (stream && (role === "agent" || role === "thought") && list.length) {
    const last = list[list.length - 1];
    if (last.role === role && last.streaming) {
      last.body = (last.body || "") + text;
      last.at = at;
      if (sessionId === state.selectedSession) {
        patchLastTranscriptBody(last);
      }
      return;
    }
  }

  // Non-stream agent after stream → close previous stream bubble.
  if (!stream && list.length) {
    const last = list[list.length - 1];
    if (last.streaming) last.streaming = false;
  }

  list.push({
    role,
    body: text,
    at,
    streaming: stream && (role === "agent" || role === "thought"),
  });
  if (sessionId === state.selectedSession) {
    renderTranscript();
  }
}

/** Fast path: update only the last agent bubble while streaming. */
function patchLastTranscriptBody(entry) {
  const root = $("transcript");
  if (!root) {
    renderTranscript();
    return;
  }
  const blocks = root.querySelectorAll(".t-block");
  const last = blocks[blocks.length - 1];
  if (
    !last ||
    (!last.classList.contains("agent") && !last.classList.contains("thought"))
  ) {
    renderTranscript();
    return;
  }
  const body = last.querySelector(".t-body");
  const time = last.querySelector(".t-time");
  if (!body) {
    renderTranscript();
    return;
  }
  body.textContent = entry.body || "";
  if (time) time.textContent = entry.at || "";
  last.classList.toggle("streaming", !!entry.streaming);
  root.scrollTop = root.scrollHeight;
}

function endAgentStream(sessionId) {
  if (!sessionId) return;
  const list = getTranscript(sessionId);
  if (!list.length) return;
  const last = list[list.length - 1];
  if (last.streaming) {
    last.streaming = false;
    if (sessionId === state.selectedSession) {
      const root = $("transcript");
      const blocks = root?.querySelectorAll(".t-block");
      const el = blocks?.[blocks.length - 1];
      el?.classList.remove("streaming");
    }
  }
}

function roleBombMood(role) {
  if (role === "user") return "idle";
  if (role === "agent") return "stream";
  if (role === "thought") return "thinking";
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
            : role === "thought"
              ? "thinking"
              : role === "tool"
                ? "tool"
                : role === "plan"
                  ? "plan"
                  : role === "error"
                    ? "error"
                    : "system";
      const streamCls = e.streaming ? " streaming" : "";
      return `<div class="t-block ${escapeHtml(role)}${streamCls}">
  <div class="t-role">${bombHtml(roleBombMood(role), "xs")}<span>${label}</span>${e.streaming ? '<span class="stream-caret" aria-hidden="true"></span>' : ""}</div>
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
    String(b.updatedAt || b.updated_at || b.createdAt || b.created_at || "").localeCompare(
      String(a.updatedAt || a.updated_at || a.createdAt || a.created_at || "")
    )
  );
  root.innerHTML = sorted
    .map((s) => {
      const id = s.id;
      const status = String(s.status || "unknown").toLowerCase();
      const mode = String(s.mode || "acp").toLowerCase();
      const model = s.model || "";
      const isMock = model === "mock";
      const live = s.live !== false && !status.includes("saved");
      const selected = id === state.selectedSession ? "selected" : "";
      const badgeCls = isMock
        ? "mock"
        : status.includes("run")
          ? "running"
          : status.includes("fail") || status.includes("cancel")
            ? "failed"
            : status.includes("saved")
              ? "saved"
              : "idle";
      const bombMood = isMock ? "idle" : moodFromStatus(status);
      const cwd = s.cwd || "";
      const shortCwd = cwd.length > 28 ? "…" + cwd.slice(-27) : cwd;
      const msgs = s.messageCount ?? s.message_count ?? 0;
      const liveTag = live
        ? ""
        : `<span class="badge saved" title="Restored from disk">saved</span>`;
      return `<div class="thread-item ${selected}${live ? "" : " restored"}" data-id="${escapeHtml(id)}">
  <div class="name">${bombHtml(bombMood, "xs")} ${escapeHtml(mode)} · ${escapeHtml(shortId(id))}</div>
  <div class="meta"><span class="badge ${badgeCls}">${bombHtml(bombMood, "xs")}${escapeHtml(status)}</span>
  ${liveTag}
  <span>${escapeHtml(isMock ? "mock" : model || "—")}</span>
  ${msgs ? `<span class="muted">${msgs} msg</span>` : ""}</div>
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
  const live = state.sessions.filter((s) => s.live !== false && !String(s.status || "").includes("saved"));
  if (!live.length) {
    root.innerHTML = `<div class="empty-hint">No live agents · saved threads stay in Threads</div>`;
    return;
  }
  root.innerHTML = live
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

async function selectSession(id) {
  state.selectedSession = id;
  renderThreads();
  const sess = state.sessions.find((s) => s.id === id);
  if (sess?.cwd) $("cwd").value = sess.cwd;
  // Hydrate transcript from SQLite once (reboot / update memory).
  await loadTranscriptFromDb(id);
  renderTranscript();
}

/** Load durable thread history from ~/.grok/control-panel/sessions/control_panel.db */
async function loadTranscriptFromDb(id, { force = false } = {}) {
  if (!id) return;
  if (!force && state.transcriptLoaded.has(id)) return;
  try {
    const rows = await invoke("get_session_transcript", { id });
    if (!Array.isArray(rows)) {
      state.transcriptLoaded.add(id);
      return;
    }
    const existing = getTranscript(id);
    // Prefer live in-memory if it already has more messages (active stream).
    if (!force && existing.length > rows.length) {
      state.transcriptLoaded.add(id);
      return;
    }
    const mapped = rows.map((r) => ({
      role: r.role || "system",
      body: formatStoredBody(r.role, r.body),
      at: r.at || "",
      streaming: false,
    }));
    state.transcriptBySession.set(id, mapped);
    state.transcriptLoaded.add(id);
  } catch (e) {
    // Older builds / empty DB — ignore
    state.transcriptLoaded.add(id);
  }
}

function formatStoredBody(role, body) {
  if (role !== "tool" && role !== "plan") return body == null ? "" : String(body);
  try {
    const j = JSON.parse(body);
    if (role === "tool") {
      return `${j.tool || "tool"} [${j.status || ""}]\n${j.args || j.result || ""}`.trim();
    }
    if (role === "plan") {
      const steps = (j.steps || [])
        .map((s) => `  - [${s.status || "pending"}] ${s.description || s.id || ""}`)
        .join("\n");
      return `${j.title || "plan"} (${j.status || ""})\n${steps}`.trim();
    }
  } catch (_) {
    /* plain text */
  }
  return body == null ? "" : String(body);
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
    const raw = ev.text != null ? String(ev.text) : "";
    if (!raw) return;
    // Drop legacy status lines that were mis-tagged as agent speech.
    if (isNoiseAgentText(raw)) {
      if (isSelected && turnActive()) {
        noteTurn(state.turn.phase === "idle" ? "think" : state.turn.phase, {
          note: raw.slice(0, 80),
        });
      }
      return;
    }
    const isThought = raw.startsWith("💭");
    const text = isThought ? raw.replace(/^💭\s*/, "") : raw;
    const role = isThought ? "thought" : "agent";
    appendTranscript(sid, role, text, nowIso(), { stream: true });
    if (isSelected) {
      const list = getTranscript(sid);
      const body = list[list.length - 1]?.body || text;
      if (isThought) {
        noteTurn("think", {
          thoughtChars: (state.turn.thoughtChars || 0) + text.length,
          thoughtPreview: clipPreview(body),
        });
        // Timeline only on first thought chunk
        if ((state.turn.thoughtChars || 0) <= text.length + 1) {
          pushEvent(`thinking · ${shortId(sid)}`, "", "thinking");
        }
      } else {
        const prev = state.turn.streamChars || 0;
        noteTurn("reply", {
          streamChars: prev + text.length,
          preview: clipPreview(body),
        });
        // Timeline: first chunk + every ~400 chars
        if (prev === 0 || Math.floor((prev + text.length) / 400) > Math.floor(prev / 400)) {
          pushEvent(
            `reply · ${formatCount(prev + text.length)} chars`,
            "",
            "stream"
          );
        }
      }
    }
  } else if (type === "tool_call" || type === "toolCall") {
    endAgentStream(sid);
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
    const done =
      st.includes("done") ||
      st.includes("complete") ||
      st.includes("success") ||
      st.includes("completed");
    pushEvent(`tool · ${tool} · ${status}`, done ? "ok" : "", done ? "boom" : "tooling", {
      force: true,
    });
    if (isSelected) {
      noteTurn("tools", {
        toolCount: (state.turn.toolCount || 0) + (done ? 0 : 1),
        lastTool: tool,
        lastToolStatus: status,
        note: String(summary).slice(0, 60),
      });
      // If tool finished and we already had reply chars, stay on tools until more reply
      if (done && state.turn.streamChars) {
        noteTurn("reply", { lastTool: tool, lastToolStatus: status });
      }
    }
  } else if (type === "plan_update" || type === "planUpdate") {
    const pe = ev.event || ev;
    const steps = (pe.steps || [])
      .map((s) => `  - [${s.status || "pending"}] ${s.description || s.id}`)
      .join("\n");
    appendTranscript(sid, "plan", `${pe.title || "plan"} (${pe.status || ""})\n${steps}`);
    pushEvent(`plan · ${(pe.steps || []).length} steps`, "", "thinking");
    if (isSelected) noteTurn("think", { note: pe.title || "plan update" });
  } else if (type === "session_created" || type === "sessionCreated") {
    pushEvent(`session · ${shortId(sid)} ready`, "ok", "boom", { force: true });
    refreshSessions();
  } else if (type === "session_status_changed" || type === "sessionStatusChanged") {
    const st = String(ev.status || "").toLowerCase();
    // Quiet: only log meaningful transitions
    if (st.includes("idle") || st.includes("fail") || st.includes("cancel") || st.includes("wait")) {
      pushEvent(`session · ${shortId(sid)} → ${ev.status}`, "", moodFromStatus(st));
    }
    if (isSelected) {
      if (st.includes("wait") || st.includes("approv")) {
        noteTurn("wait", { note: "Waiting for approval" });
      } else if (st.includes("fail") || st.includes("error")) {
        endAgentStream(sid);
        noteTurn("error", { note: String(ev.status) });
      } else if (st.includes("cancel")) {
        endAgentStream(sid);
        noteTurn("error", { note: "Cancelled" });
      } else if (st.includes("idle") || st.includes("complete")) {
        endAgentStream(sid);
        if (turnActive() || state.turn.streamChars || state.turn.toolCount) {
          flashBoomThenIdle();
        } else {
          noteTurn("idle");
        }
      } else if (st.includes("run") && !turnActive()) {
        noteTurn("think", { note: "Session running" });
      }
    }
    refreshSessions();
  } else if (type === "session_cancelled" || type === "sessionCancelled") {
    endAgentStream(sid);
    appendTranscript(sid, "system", "session cancelled");
    pushEvent(`cancelled · ${shortId(sid)}`, "", "error", { force: true });
    if (isSelected) noteTurn("error", { note: "Cancelled" });
    refreshSessions();
  } else if (type === "error") {
    endAgentStream(sid || state.selectedSession);
    appendTranscript(sid || state.selectedSession, "error", ev.message || "error");
    pushEvent(ev.message || "error", "err", "error", { force: true });
    setStatus(state.ready ? "ready" : "error", ev.message || "error");
    if (isSelected) noteTurn("error", { note: ev.message || "error" });
  } else if (type === "approval_required" || type === "approvalRequired") {
    endAgentStream(sid);
    appendTranscript(
      sid,
      "system",
      `approval required: ${ev.tool || "?"} — ${ev.summary || ev.request_id || ""}`
    );
    pushEvent(`approval · ${ev.tool || "?"}`, "err", "wait", { force: true });
    if (isSelected) {
      noteTurn("wait", {
        lastTool: ev.tool || "approval",
        note: ev.summary || "approval required",
      });
    }
  } else if (type === "raw") {
    const payload = ev.payload || ev;
    const maybe =
      payload?.update?.content?.text ||
      payload?.content?.text ||
      payload?.text ||
      (typeof payload?.message === "string" ? payload.message : null);
    if (maybe && typeof maybe === "string" && maybe.trim() && !isNoiseAgentText(maybe)) {
      appendTranscript(sid || state.selectedSession, "agent", maybe, nowIso(), {
        stream: true,
      });
      if (isSelected) {
        noteTurn("reply", {
          streamChars: (state.turn.streamChars || 0) + maybe.length,
          preview: clipPreview(maybe),
        });
      }
    }
  }
  // swallow other low-value event types from timeline noise
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
    // Prefer list_threads (live + SQLite). Fall back to live-only list_sessions.
    let list;
    try {
      list = await invoke("list_threads");
    } catch (_) {
      list = await invoke("list_sessions");
    }
    state.sessions = Array.isArray(list) ? list : [];
    renderThreads();
    renderAgents();
    if (state.selectedSession && !state.sessions.some((s) => s.id === state.selectedSession)) {
      state.selectedSession = state.sessions[0]?.id || null;
    }
    if (!state.selectedSession && state.sessions.length) {
      state.selectedSession = state.sessions[0].id;
    }
    if (state.selectedSession) {
      await loadTranscriptFromDb(state.selectedSession);
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
    endAgentStream(state.selectedSession);
    state.phraseIndex = 0;
    state.turn = emptyTurn();
    noteTurn("send", {
      promptChars: prompt.length,
      note: "On the wire",
      startedAt: Date.now(),
      lastSignalAt: Date.now(),
    });
    // Immediately advance to think — waiting for first real signal
    noteTurn("think", { promptChars: prompt.length, note: "Waiting for first token or tool" });
    startPhraseCycle();
    pushEvent(
      `you · ${formatCount(prompt.length)} chars → ${shortId(state.selectedSession)}`,
      "ok",
      "thinking",
      { force: true }
    );
    await invoke("send_prompt", { id: state.selectedSession, prompt });
    // Keep think until agent_message / tool_call arrives
    if (state.turn.phase === "send") {
      noteTurn("think", { note: "Prompt accepted · waiting on agent" });
    }
    updateBombChrome();
  } catch (e) {
    noteTurn("error", { note: e?.message || String(e) });
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
    noteTurn("wait", { note: "Cancel requested…" });
    pushEvent("cancel · requested", "", "wait", { force: true });
    await invoke("cancel_session", { id: state.selectedSession });
    appendTranscript(state.selectedSession, "system", "cancel requested");
    endAgentStream(state.selectedSession);
    noteTurn("error", { note: "Cancelled" });
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

// Elapsed + stall clock
setInterval(() => {
  if (turnActive() || state.turn.phase === "done") updateBombChrome();
  if (turnActive()) startPhraseCycle();
  else stopPhraseCycle();
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
    noteTurn("idle");
    pushEvent("Bomb Code ready", "ok", "boom", { force: true });
    if (state.sessions.length) {
      const saved = state.sessions.filter((s) => s.live === false || String(s.status).includes("saved")).length;
      const live = state.sessions.length - saved;
      pushEvent(
        `memory · ${state.sessions.length} thread${state.sessions.length === 1 ? "" : "s"} (${live} live · ${saved} saved)`,
        "ok",
        "ready",
        { force: true }
      );
    }
    if (state.auth && !state.auth.loggedIn) {
      pushEvent("Not signed in — Log in with Grok", "", "wait", { force: true });
    }
    updateBombChrome();
  } catch (e) {
    toastError(e);
  }
}

boot();
