// Bomb Code — three-column Grok Build control panel.
// Pixel-bomb visual language: moods for thinking / tools / boom / wait.
// Turn status: BombPresence (presence.js) — single source of truth.

const $ = (id) => document.getElementById(id);

const LOGO = "assets/logo.png";
const P = window.BombPresence;
if (!P || typeof P.emptyPresence !== "function") {
  console.error("BombPresence missing — presence.js failed to load before app.js");
  document.addEventListener("DOMContentLoaded", () => {
    const st = document.getElementById("status-text");
    if (st) st.textContent = "Presence module failed to load";
    const pill = document.getElementById("status-pill");
    if (pill) pill.className = "status-pill status-error";
  });
}

const state = {
  selectedSession: null,
  sessions: [],
  tools: [],
  ready: false,
  auth: null,
  loggingIn: false,
  devServer: null,
  transcriptBySession: new Map(),
  transcriptLoaded: new Set(),
  /** @type {ReturnType<typeof P.emptyPresence>} */
  turn: P ? P.emptyPresence() : { phase: "idle" },
  /** Per-session presence map (all sessions, not only selected). */
  presenceBySession: new Map(),
  /** sessionId -> Set of in-flight tool ids */
  openToolsBySession: new Map(),
  phraseTimer: null,
  phraseIndex: 0,
  lastEventKey: "",
  boomTimer: null,
  boomSessionId: null,
  hostStatusKind: "unknown",
  hostStatusText: "…",
};

function openToolsFor(sid) {
  const key = sid || state.selectedSession || "_";
  if (!state.openToolsBySession.has(key)) {
    state.openToolsBySession.set(key, new Set());
  }
  return state.openToolsBySession.get(key);
}

/** Get mutable presence for a session; selected session aliases state.turn. */
function presenceFor(sid) {
  if (!P) return state.turn;
  if (!sid || sid === state.selectedSession) return state.turn;
  if (!state.presenceBySession.has(sid)) {
    state.presenceBySession.set(sid, P.emptyPresence());
  }
  return state.presenceBySession.get(sid);
}

function commitPresence(sid, p, { paint = true } = {}) {
  if (!sid || sid === state.selectedSession) {
    state.turn = p;
    if (state.selectedSession) {
      state.presenceBySession.set(state.selectedSession, p);
    }
    if (paint) updateBombChrome();
  } else {
    state.presenceBySession.set(sid, p);
  }
}

function isToolTerminal(status) {
  const st = String(status || "").toLowerCase();
  return (
    st.includes("complete") ||
    st.includes("done") ||
    st.includes("success") ||
    st.includes("fail") ||
    st.includes("error") ||
    st.includes("denied") ||
    st.includes("reject") ||
    st.includes("cancel")
  );
}

function endTurnPresence(sid, phase, note) {
  let p = presenceFor(sid);
  p = P.applySignal(p, phase, {
    note: note || "",
    toolsActive: 0,
    lastToolStatus: phase === "error" ? "failed" : "completed",
  });
  openToolsFor(sid).clear();
  if (state.boomTimer && phase !== "done") {
    clearTimeout(state.boomTimer);
    state.boomTimer = null;
    state.boomSessionId = null;
  }
  commitPresence(sid, p);
}

function emptyTurn() {
  return P.emptyPresence();
}

function bombHtml(mood = "idle", size = "sm", extraClass = "") {
  const wick =
    ["thinking", "stream", "tooling", "wait", "ready", "running"].includes(mood)
      ? " wick-on"
      : "";
  return `<span class="px-bomb ${size} mood-${mood} tier-satellite${wick} ${extraClass}" aria-hidden="true"><img src="${LOGO}" alt="" /></span>`;
}

function moodFromStatus(status) {
  const s = String(status || "").toLowerCase();
  if (s.includes("run") || s.includes("generat") || s.includes("busy")) return "ready";
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

function setBombMood(el, mood, opts = {}) {
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
  const prev = moods.find((m) => el.classList.contains(`mood-${m}`));
  el.classList.remove(...moods.map((m) => `mood-${m}`));
  el.classList.add(`mood-${mood}`);
  if (opts.entering && prev !== mood) {
    el.classList.remove("is-entering");
    // reflow so re-adding restarts animation
    void el.offsetWidth;
    el.classList.add("is-entering");
    const clear = () => el.classList.remove("is-entering");
    el.addEventListener("animationend", clear, { once: true });
    setTimeout(clear, 400);
  }
}

function anySessionBusy() {
  return state.sessions.some((s) => {
    const st = String(s.status || "").toLowerCase();
    return st.includes("run") || st.includes("wait");
  });
}

function turnActive() {
  return P.turnActive(state.turn);
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
  return P.formatElapsed(ms);
}

function formatCount(n) {
  return P.formatCount(n);
}

function clipPreview(text, n = 96) {
  return P.clipPreview(text, n);
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

/** Start or advance the turn with a concrete signal (selected session by default). */
function noteTurn(phase, patch = {}, sid = null) {
  if (!P) return;
  const target = sid || state.selectedSession;
  let p = presenceFor(target);
  p = P.applySignal(p, phase, patch, Date.now());
  if (phase === "error" || phase === "idle") {
    p.toolsActive = 0;
    openToolsFor(target).clear();
  }
  commitPresence(target, p);
}

function setTurnPhase(phase, detail = "") {
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

function flashBoomThenIdle(ms, sid = null) {
  const hold = ms != null ? ms : P.BOOM_HOLD_MS;
  const target = sid || state.selectedSession;
  if (state.boomTimer) clearTimeout(state.boomTimer);
  openToolsFor(target).clear();
  noteTurn("done", { note: "Turn finished", toolsActive: 0 }, target);
  state.boomSessionId = target;
  state.boomTimer = setTimeout(() => {
    state.boomTimer = null;
    const boomSid = state.boomSessionId;
    state.boomSessionId = null;
    if (!boomSid) return;
    if (boomSid !== state.selectedSession) {
      const p = state.presenceBySession.get(boomSid);
      if (p && p.phase === "done") {
        state.presenceBySession.set(boomSid, P.emptyPresence());
      }
      return;
    }
    if (state.turn.phase === "done") noteTurn("idle");
  }, hold);
}

function updateBombChrome() {
  const view = P.formatPresence(state.turn, {
    now: Date.now(),
    phraseIndex: state.phraseIndex,
  });
  const transition = P.consumeTransition(state.turn);
  const anyBusy = anySessionBusy() || view.active;

  // Brand: product name; optional live elapsed only (no phase essay)
  const brand = $("brand-header");
  if (brand) brand.classList.toggle("live", anyBusy);
  const brandSub = $("brand-sub");
  if (brandSub) {
    brandSub.textContent =
      view.active && view.elapsed ? `Live · ${view.elapsed}` : "Grok Build panel";
  }

  // Primary turn dock
  const dock = $("turn-dock");
  if (dock) {
    dock.classList.toggle("visible", view.show);
    dock.classList.toggle("stalled", !!view.stalled);
    dock.classList.toggle("has-flavor", !!view.flavor);
    ["idle", "send", "think", "tools", "reply", "wait", "done", "error"].forEach((p) => {
      dock.classList.toggle(`phase-${p}`, p === view.phase);
    });
    dock.setAttribute("aria-hidden", view.show ? "false" : "true");

    const bomb = $("turn-bomb");
    setBombMood(bomb, view.mood, { entering: !!transition });
    if (bomb) {
      bomb.classList.add("tier-dock", "lg");
      bomb.classList.remove("tier-satellite", "md", "sm", "xs");
      // Wick is driven by mood CSS (thinking/stream/tooling/wait/ready)
      bomb.classList.toggle(
        "wick-on",
        ["thinking", "stream", "tooling", "wait", "ready", "running"].includes(view.mood)
      );
    }

    const label = $("turn-phase-label");
    if (label) label.textContent = view.title;

    const elEl = $("turn-elapsed");
    if (elEl) elEl.textContent = view.elapsed || "";

    const flavor = $("turn-flavor");
    if (flavor) flavor.textContent = view.flavor || "";

    const detail = $("turn-detail");
    if (detail) detail.textContent = view.subtitle;

    const preview = $("turn-preview");
    if (preview) {
      if (view.show && view.preview) {
        preview.style.display = "block";
        preview.textContent = view.preview;
      } else {
        preview.style.display = "none";
        preview.textContent = "";
      }
    }

    document.querySelectorAll("#turn-stages .stage").forEach((el) => {
      const st = el.getAttribute("data-stage");
      el.classList.remove("active", "done", "error");
      const cls = P.stageClass(st, state.turn);
      if (cls) el.classList.add(cls);
    });

    const meter = document.querySelector(".turn-dock-meter");
    const bar = $("turn-meter-bar");
    if (meter) meter.setAttribute("data-mode", view.meterMode);
    if (bar) {
      if (view.meterMode === "progress" || view.meterMode === "tools") {
        bar.style.width = `${Math.round(view.meterProgress * 100)}%`;
        bar.style.transform = "none";
      } else {
        bar.style.width = "";
        bar.style.transform = "";
      }
    }
  }

  // Composer chip — static satellite bomb
  const composer = $("composer");
  if (composer) composer.classList.toggle("busy", view.active);
  const phaseChip = $("composer-phase");
  if (phaseChip) {
    if (view.active) {
      phaseChip.style.display = "inline-flex";
      phaseChip.innerHTML = `${bombHtml(view.mood, "sm")}<span>${escapeHtml(view.title)}${
        view.elapsed ? ` · ${view.elapsed}` : ""
      }</span>`;
    } else {
      phaseChip.style.display = "none";
      phaseChip.innerHTML = "";
    }
  }

  // Activity "Now" — compact mirror, satellite tier
  const nowPanel = $("now-panel");
  const nowElapsed = $("now-elapsed");
  if (nowElapsed) nowElapsed.textContent = view.active ? view.elapsed : "";
  if (nowPanel) {
    if (!view.show) {
      nowPanel.innerHTML = `<div class="empty-hint">No live turn</div>`;
    } else {
      nowPanel.innerHTML = `
        <div class="now-row">
          ${bombHtml(view.mood, "sm")}
          <div class="now-copy">
            <div class="now-phase">${escapeHtml(view.title)}</div>
            <div class="now-detail muted">${escapeHtml(view.subtitle)}</div>
          </div>
        </div>
        ${
          view.preview
            ? `<div class="now-preview">${escapeHtml(view.preview)}</div>`
            : ""
        }
        ${
          view.lastTool
            ? `<div class="now-tool"><span class="now-tool-label">${escapeHtml(
                view.lastTool
              )}${view.lastToolStatus ? ` · ${escapeHtml(view.lastToolStatus)}` : ""}</span></div>`
            : ""
        }`;
    }
  }

  // Activity header — ambient body; wick when any session busy
  const actBomb = $("activity-bomb");
  if (actBomb) {
    actBomb.classList.add("tier-ambient", "md");
    const actMood = anyBusy
      ? view.phase === "tools"
        ? "tooling"
        : view.phase === "reply"
          ? "stream"
          : view.active
            ? "thinking"
            : "ready"
      : "idle";
    setBombMood(actBomb, actMood);
    actBomb.classList.toggle("wick-on", actMood !== "idle");
  }

  // Status pill: HOST HEALTH ONLY (plan §5.3) — do not overwrite with turn monologue
  if (state.ready && state.hostStatusKind === "ready") {
    const pill = $("status-pill");
    if (pill && !pill.classList.contains("status-error")) {
      setBombMood($("status-bomb"), "ready");
      const st = $("status-text");
      if (st && !st.dataset.locked) {
        st.textContent = state.hostStatusText || "Ready";
      }
    }
  }
}

function startPhraseCycle() {
  if (state.phraseTimer) return;
  state.phraseTimer = setInterval(() => {
    if (!turnActive()) return;
    state.phraseIndex += 1;
    updateBombChrome();
  }, 6000);
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
  state.hostStatusKind = k;
  state.hostStatusText = text;
  pill.className = `status-pill status-${k}`;
  $("status-text").textContent = text;
  // Host health only — static / ambient bomb, never turn monologue
  let mood = "idle";
  if (k === "ready") mood = "ready";
  else if (k === "error") mood = "error";
  else if (k === "thinking" || k === "running") mood = "ready";
  else if (k === "unknown") mood = "idle";
  const sb = $("status-bomb");
  setBombMood(sb, mood);
  if (sb) {
    sb.classList.add("tier-satellite");
    sb.classList.remove("tier-dock");
  }
}

function pushEvent(text, cls = "", moodHint = null, opts = {}) {
  const feed = $("event-feed");
  if (!feed) return;
  const key = `${cls}|${text}`;
  if (!opts.force && key === state.lastEventKey && cls !== "err") return;
  state.lastEventKey = key;
  const line = document.createElement("div");
  line.className = `event-line ${cls}`;
  const ts = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  // Bombs only on milestones (errors, explicit force+mood, boom completions)
  const showBomb =
    opts.milestone ||
    cls === "err" ||
    moodHint === "error" ||
    moodHint === "boom" ||
    moodHint === "wait";
  const mood = moodHint || moodFromEventCls(cls);
  const icon = showBomb ? bombHtml(mood, "xs") : "";
  line.innerHTML = `${icon}<span class="event-body"><span class="ts">${ts}</span>${escapeHtml(text)}</span>`;
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
  pushEvent(msg, "err", "error", { force: true, milestone: true });
  // Host pill stays host-only; surface agent errors on timeline/dock only
  if (!state.ready) setStatus("error", msg);
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

  // Coalesce streaming agent/thought/term chunks into one live block (TTY feel).
  if (stream && (role === "agent" || role === "thought" || role === "term") && list.length) {
    const last = list[list.length - 1];
    if (last.role === role && last.streaming) {
      // term lines: newline-join; agent speech: append raw chunks
      if (role === "term") {
        last.body = (last.body || "") + (last.body ? "\n" : "") + text;
      } else {
        last.body = (last.body || "") + text;
      }
      last.at = at;
      if (sessionId === state.selectedSession) {
        patchLastTranscriptBody(last);
      }
      return;
    }
  }

  // Non-stream after stream → close previous stream bubble.
  if (!stream && list.length) {
    const last = list[list.length - 1];
    if (last.streaming) last.streaming = false;
  }

  list.push({
    role,
    body: text,
    at,
    streaming: stream && (role === "agent" || role === "thought" || role === "term"),
  });
  // Cap memory so huge TTY logs stay snappy
  if (list.length > 2000) {
    list.splice(0, list.length - 2000);
  }
  if (sessionId === state.selectedSession) {
    renderTranscript();
  }
}

/** Fast path: update only the last streaming block. */
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
    (!last.classList.contains("agent") &&
      !last.classList.contains("thought") &&
      !last.classList.contains("term"))
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
  if (time) time.textContent = shortTime(entry.at || "");
  last.classList.toggle("streaming", !!entry.streaming);
  root.scrollTop = root.scrollHeight;
}

function shortTime(iso) {
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } catch {
    return iso;
  }
}

function termPrefix(role) {
  if (role === "user") return "you";
  if (role === "agent") return "grok";
  if (role === "thought") return "think";
  if (role === "tool") return "tool";
  if (role === "plan") return "plan";
  if (role === "error") return "err";
  if (role === "term") return "acp";
  return "sys";
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
  if (role === "term") return "running";
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
<p class="muted">Center column mirrors the live agent terminal stream.</p>
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
<p>Connected. Messages, tools, thoughts, and ACP lines stream here like a terminal.</p>
</div>`;
    updateBombChrome();
    return;
  }

  // Terminal-style continuous log (mirrors CLI, not just chat bubbles).
  const liveView = P.formatPresence(state.turn, { phraseIndex: state.phraseIndex });
  const liveHint =
    turnActive() && isSelectedBusyForRender()
      ? `<div class="t-block term streaming term-live">
  <div class="t-role">${bombHtml(liveView.mood, "xs")}<span>live</span><span class="stream-caret" aria-hidden="true"></span></div>
  <div class="t-body">${escapeHtml(liveView.subtitle)}</div>
</div>`
      : "";

  root.innerHTML =
    entries
      .map((e) => {
        const role = e.role || "system";
        const label = termPrefix(role);
        const streamCls = e.streaming ? " streaming" : "";
        return `<div class="t-block ${escapeHtml(role)}${streamCls}">
  <div class="t-role"><span class="t-ts">${escapeHtml(shortTime(e.at || ""))}</span>${bombHtml(roleBombMood(role), "xs")}<span>${label}</span>${e.streaming ? '<span class="stream-caret" aria-hidden="true"></span>' : ""}</div>
  <div class="t-body">${escapeHtml(e.body)}</div>
</div>`;
      })
      .join("") + liveHint;
  root.scrollTop = root.scrollHeight;
  updateBombChrome();
}

function isSelectedBusyForRender() {
  return turnActive();
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
      const brain = String(s.brainMode || s.brain_mode || "").toLowerCase();
      let brainTag = "";
      if (live && brain === "full_brain") {
        brainTag = `<span class="badge brain-full" title="Agent reloaded prior ACP session">full brain</span>`;
      } else if (live && brain === "history_only") {
        brainTag = `<span class="badge brain-history" title="New ACP process; transcript injected as context">history-only</span>`;
      }
      return `<div class="thread-item ${selected}${live ? "" : " restored"}" data-id="${escapeHtml(id)}">
  <div class="name">${bombHtml(bombMood, "xs")} ${escapeHtml(mode)} · ${escapeHtml(shortId(id))}</div>
  <div class="meta"><span class="badge ${badgeCls}">${bombHtml(bombMood, "xs")}${escapeHtml(status)}</span>
  ${liveTag}${brainTag}
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
  const prev = state.selectedSession;
  // Persist current presence under previous id before switching
  if (prev && state.turn && P) {
    state.presenceBySession.set(prev, state.turn);
  }
  // Do not clear boom timer mid-hold — it is session-scoped in the callback
  state.selectedSession = id || null;
  if (!id) {
    state.turn = P ? P.emptyPresence() : { phase: "idle" };
  } else {
    state.turn = state.presenceBySession.get(id) || (P ? P.emptyPresence() : { phase: "idle" });
    state.presenceBySession.set(id, state.turn);
  }
  renderThreads();
  const sess = state.sessions.find((s) => s.id === id);
  if (sess?.cwd) $("cwd").value = sess.cwd;
  if (id) await loadTranscriptFromDb(id);
  renderTranscript();
  updateBombChrome();
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
    if (isNoiseAgentText(raw)) {
      appendTranscript(sid, "term", raw, nowIso(), { stream: true });
      if (sid && P.turnActive(presenceFor(sid))) {
        noteTurn(presenceFor(sid).phase === "idle" ? "think" : presenceFor(sid).phase, {
          note: raw.slice(0, 80),
        }, sid);
      }
      return;
    }
    if (
      raw.startsWith("🧠") ||
      raw.startsWith("📜") ||
      raw.startsWith("⚙") ||
      raw.startsWith("wrote ")
    ) {
      appendTranscript(sid, "term", raw);
      return;
    }
    const isThought = raw.startsWith("💭");
    const text = isThought ? raw.replace(/^💭\s*/, "") : raw;
    const role = isThought ? "thought" : "agent";
    appendTranscript(sid, role, text, nowIso(), { stream: true });
    if (!sid) return;
    const list = getTranscript(sid);
    const body = list[list.length - 1]?.body || text;
    let p = presenceFor(sid);
    if (isThought) {
      p = P.applySignal(p, "think", {
        thoughtChars: (p.thoughtChars || 0) + text.length,
        thoughtPreview: clipPreview(body),
      });
      commitPresence(sid, p);
      if (isSelected && (p.thoughtChars || 0) <= text.length + 1) {
        pushEvent(`thinking · ${shortId(sid)}`, "", null, { force: true });
      }
    } else {
      const prev = p.replyChars || 0;
      p = P.applySignal(p, "reply", {
        replyChars: prev + text.length,
        preview: clipPreview(body),
      });
      commitPresence(sid, p);
      if (
        isSelected &&
        (prev === 0 || Math.floor((prev + text.length) / 400) > Math.floor(prev / 400))
      ) {
        pushEvent(`reply · ${formatCount(prev + text.length)} chars`, "", null, { force: true });
      }
    }
  } else if (type === "tool_call" || type === "toolCall") {
    endAgentStream(sid);
    const te = ev.event || ev;
    const tool = te.tool || te.name || "tool";
    const toolId = String(te.id || `${tool}-${Date.now()}`);
    const summary = te.args_summary || te.argsSummary || te.result_summary || te.resultSummary || "";
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
    const terminal = isToolTerminal(status);
    appendTranscript(
      sid,
      "tool",
      `$ ${tool}  [${status}]\n${String(summary).slice(0, 2000)}${
        te.result_summary || te.resultSummary
          ? `\n→ ${String(te.result_summary || te.resultSummary).slice(0, 800)}`
          : ""
      }`
    );
    pushEvent(`tool · ${tool} · ${status}`, terminal ? "ok" : "", null, {
      force: true,
      milestone: terminal && String(status).toLowerCase().includes("fail"),
    });
    if (!sid) return;
    const open = openToolsFor(sid);
    let p = presenceFor(sid);
    if (terminal) {
      if (open.has(toolId)) {
        open.delete(toolId);
        p = P.markToolDone(p, tool, status, Date.now());
      } else {
        // Late terminal update without prior start — status patch only
        p = P.applySignal(p, p.phase === "idle" ? "tools" : p.phase, {
          lastTool: tool,
          lastToolStatus: status,
          toolsActive: open.size,
        });
      }
      p.toolsActive = open.size;
    } else if (!open.has(toolId)) {
      open.add(toolId);
      p = P.markToolStart(p, tool, Date.now());
      p.note = String(summary).slice(0, 60);
      p.toolsActive = open.size;
    } else {
      p = P.applySignal(p, "tools", {
        lastTool: tool,
        lastToolStatus: status,
        note: String(summary).slice(0, 60),
        toolsActive: open.size,
      });
    }
    commitPresence(sid, p);
  } else if (type === "plan_update" || type === "planUpdate") {
    const pe = ev.event || ev;
    const steps = (pe.steps || [])
      .map((s) => `  - [${s.status || "pending"}] ${s.description || s.id}`)
      .join("\n");
    appendTranscript(sid, "plan", `plan ${pe.title || ""}\n${steps}`);
    pushEvent(`plan · ${(pe.steps || []).length} steps`, "", null, { force: true });
    if (sid) noteTurn("think", { note: pe.title || "plan update" }, sid);
  } else if (type === "session_created" || type === "sessionCreated") {
    appendTranscript(sid, "term", `session ready · ${shortId(sid)}`);
    pushEvent(`session · ${shortId(sid)} ready`, "ok", "boom", { force: true, milestone: true });
    refreshSessions();
  } else if (type === "session_status_changed" || type === "sessionStatusChanged") {
    const st = String(ev.status || "").toLowerCase();
    appendTranscript(sid, "term", `status → ${ev.status}`);
    pushEvent(`session · ${shortId(sid)} → ${ev.status}`, "", null, { force: true });
    if (sid) {
      if (st.includes("wait") || st.includes("approv")) {
        noteTurn("wait", { note: "Waiting for approval" }, sid);
      } else if (st.includes("fail") || st.includes("error")) {
        endAgentStream(sid);
        endTurnPresence(sid, "error", String(ev.status));
      } else if (st.includes("cancel")) {
        endAgentStream(sid);
        endTurnPresence(sid, "error", "Cancelled");
      } else if (st.includes("idle") || st.includes("complete")) {
        endAgentStream(sid);
        const p = presenceFor(sid);
        if (P.turnActive(p) || p.replyChars || p.toolCount) {
          flashBoomThenIdle(undefined, sid);
        } else {
          noteTurn("idle", {}, sid);
        }
      } else if (st.includes("run") && !P.turnActive(presenceFor(sid))) {
        noteTurn("think", { note: "Session running" }, sid);
      }
    }
    refreshSessions();
  } else if (type === "session_cancelled" || type === "sessionCancelled") {
    endAgentStream(sid);
    appendTranscript(sid, "term", "session cancelled");
    pushEvent(`cancelled · ${shortId(sid)}`, "err", "error", { force: true, milestone: true });
    if (sid) endTurnPresence(sid, "error", "Cancelled");
    refreshSessions();
  } else if (type === "error") {
    const errSid = sid || state.selectedSession;
    endAgentStream(errSid);
    appendTranscript(errSid, "error", ev.message || "error");
    pushEvent(ev.message || "error", "err", "error", { force: true, milestone: true });
    if (!state.ready) setStatus("error", ev.message || "error");
    if (errSid) endTurnPresence(errSid, "error", ev.message || "error");
  } else if (type === "approval_required" || type === "approvalRequired") {
    endAgentStream(sid);
    appendTranscript(
      sid,
      "term",
      `! approval · ${ev.tool || "?"} — ${ev.summary || ev.request_id || ""}`
    );
    pushEvent(`approval · ${ev.tool || "?"}`, "err", "wait", { force: true, milestone: true });
    if (sid) {
      noteTurn(
        "wait",
        {
          lastTool: ev.tool || "approval",
          note: ev.summary || "approval required",
        },
        sid
      );
    }
  } else if (type === "raw") {
    const payload = ev.payload || ev;
    if (payload?.channel === "usage" && payload?.totalTokens != null) {
      const n = Number(payload.totalTokens) || 0;
      if (sid && n > 0) {
        const p = presenceFor(sid);
        if (P.turnActive(p) || p.phase === "idle") {
          noteTurn(p.phase === "idle" ? "think" : p.phase, { contextTokens: n }, sid);
        }
      }
      return;
    }
    if (payload?.channel === "term" && payload?.line) {
      const tSid = sid || state.selectedSession;
      appendTranscript(tSid, "term", String(payload.line), nowIso(), { stream: true });
      if (tSid && P.turnActive(presenceFor(tSid))) {
        noteTurn(
          presenceFor(tSid).phase === "idle" ? "think" : presenceFor(tSid).phase,
          { note: String(payload.line).slice(0, 80) },
          tSid
        );
      }
      return;
    }
    const maybe =
      payload?.update?.content?.text ||
      payload?.content?.text ||
      payload?.text ||
      (typeof payload?.message === "string" ? payload.message : null);
    if (maybe && typeof maybe === "string" && maybe.trim() && !isNoiseAgentText(maybe)) {
      const tSid = sid || state.selectedSession;
      appendTranscript(tSid, "agent", maybe, nowIso(), { stream: true });
      if (tSid) {
        const p = presenceFor(tSid);
        noteTurn(
          "reply",
          {
            replyChars: (p.replyChars || 0) + maybe.length,
            preview: clipPreview(maybe),
          },
          tSid
        );
      }
    } else {
      const dump = JSON.stringify(payload);
      if (dump && dump !== "{}" && dump !== "null") {
        appendTranscript(
          sid || state.selectedSession,
          "term",
          dump.length > 400 ? dump.slice(0, 400) + "…" : dump,
          nowIso(),
          { stream: true }
        );
      }
    }
  } else {
    appendTranscript(
      sid || state.selectedSession,
      "term",
      `event ${type}${sid ? ` · ${shortId(sid)}` : ""}`
    );
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
    const stillThere =
      state.selectedSession && state.sessions.some((s) => s.id === state.selectedSession);
    if (!stillThere) {
      const next = state.sessions[0]?.id || null;
      await selectSession(next);
    } else {
      await loadTranscriptFromDb(state.selectedSession);
      renderTranscript();
      updateBombChrome();
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
    appendTranscript(res.id, "system", `session started · cwd ${cwd}`);
    pushEvent(`ACP session ${shortId(res.id)}`, "ok", "boom", { force: true, milestone: true });
    await refreshSessions();
    // selectSession persists previous presence under the *previous* id
    await selectSession(res.id);
  } catch (e) {
    toastError(e);
  }
}

async function sendPrompt() {
  try {
    if (!state.selectedSession) throw new Error("Select a thread first");
    const prompt = $("prompt").value;
    if (!prompt.trim()) throw new Error("Empty prompt");
    const sess = state.sessions.find((s) => s.id === state.selectedSession);
    const needsResume =
      sess && (sess.live === false || String(sess.status || "").toLowerCase().includes("saved"));

    appendTranscript(state.selectedSession, "user", prompt);
    $("prompt").value = "";
    endAgentStream(state.selectedSession);
    state.phraseIndex = 0;
    if (state.boomTimer) {
      clearTimeout(state.boomTimer);
      state.boomTimer = null;
    }
    state.turn = P.emptyPresence();
    noteTurn("send", {
      promptChars: prompt.length,
      note: needsResume ? "Resuming saved thread…" : "On the wire",
    });
    if (needsResume) {
      noteTurn("think", {
        promptChars: prompt.length,
        note: "Resume ladder: load → history inject…",
      });
      pushEvent(
        `resume · ${shortId(state.selectedSession)} · try full brain else history-only`,
        "ok",
        "thinking",
        { force: true }
      );
    } else {
      noteTurn("think", { promptChars: prompt.length, note: "Waiting for first token or tool" });
    }
    startPhraseCycle();
    pushEvent(
      `you · ${formatCount(prompt.length)} chars → ${shortId(state.selectedSession)}`,
      "ok",
      "thinking",
      { force: true }
    );
    await invoke("send_prompt", { id: state.selectedSession, prompt });
    // Mark live after successful send/resume; refresh brain_mode from registry.
    if (sess) {
      sess.live = true;
      sess.status = "running";
    }
    await refreshSessions();
    if (needsResume) {
      const s2 = state.sessions.find((s) => s.id === state.selectedSession);
      const brain = String(s2?.brainMode || s2?.brain_mode || "history_only");
      const note =
        brain === "full_brain"
          ? "🧠 full brain — agent reloaded prior ACP session"
          : "📜 history-only — prior transcript injected into this prompt";
      appendTranscript(state.selectedSession, "system", note);
      pushEvent(`brain · ${brain.replace(/_/g, " ")}`, "ok", brain === "full_brain" ? "boom" : "thinking", {
        force: true,
      });
    }
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
$("btn-haven").onclick = async () => {
  try {
    const st = await invoke("haven_status");
    const jobs = await invoke("haven_list_jobs").catch(() => []);
    const files = await invoke("haven_list_files").catch(() => []);
    $("sys-out").textContent = JSON.stringify({ status: st, jobs, files }, null, 2);
    pushEvent(
      st.connected ? st.message : st.message || "Haven offline",
      st.connected ? "ok" : "err",
      st.connected ? "boom" : "error",
      { force: true }
    );
  } catch (e) {
    toastError(e);
  }
};
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
    // Haven (Hetzner) auto-link status
    try {
      const hv = await invoke("haven_status");
      if (hv?.connected) {
        pushEvent(hv.message || `Haven · ${hv.label} linked`, "ok", "boom", { force: true });
      } else if (hv?.configured) {
        pushEvent(hv.message || "Haven offline", "err", "error", { force: true });
      }
    } catch (_) {
      /* older build without haven */
    }
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
