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
  startingSession: false,
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
  /** Persisted project folders (sidebar groups exist even with 0 threads). */
  projects: [],
  /** sessionId → ELI12 explainer cards for the right panel. */
  explainBySession: new Map(),
  explainPending: false,
  explainerEnabled: true,
  /** sessionId → boom-hold timer (per-session; one global slot let a second
   *  session's boom cancel the first one's reset). */
  boomTimers: new Map(),
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
  if (phase !== "done") clearBoomTimer(sid);
  commitPresence(sid, p);
}

function clearBoomTimer(sid) {
  const key = sid || state.selectedSession;
  const t = state.boomTimers.get(key);
  if (t) {
    clearTimeout(t);
    state.boomTimers.delete(key);
  }
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

function flashBoomThenIdle(ms, sid = null) {
  const hold = ms != null ? ms : P.BOOM_HOLD_MS;
  const target = sid || state.selectedSession;
  clearBoomTimer(target);
  openToolsFor(target).clear();
  noteTurn("done", { note: "Turn finished", toolsActive: 0 }, target);
  const timer = setTimeout(() => {
    state.boomTimers.delete(target);
    if (target !== state.selectedSession) {
      const p = state.presenceBySession.get(target);
      if (p && p.phase === "done") {
        state.presenceBySession.set(target, P.emptyPresence());
      }
      return;
    }
    if (state.turn.phase === "done") noteTurn("idle");
  }, hold);
  state.boomTimers.set(target, timer);
}

function updateBombChrome() {
  if (typeof updateSendButton === "function") updateSendButton();
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
      if (st) {
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

/// Minimal markdown → HTML for chat bodies. Everything is HTML-escaped first;
/// only the tags we emit ourselves survive. CSP forbids CDN renderers.
function renderMarkdown(src) {
  const codeBlocks = [];
  // Pull fenced code blocks out before any inline formatting.
  let text = String(src).replace(/```(\w*)[ \t]*\n?([\s\S]*?)```/g, (_, lang, code) => {
    const label = lang ? `<span class="md-lang">${escapeHtml(lang)}</span>` : "";
    codeBlocks.push(
      `<pre class="md-code">${label}<code>${escapeHtml(code.replace(/\n$/, ""))}</code></pre>`
    );
    return `\u0000${codeBlocks.length - 1}\u0000`;
  });
  text = escapeHtml(text);
  // Inline code
  text = text.replace(/`([^`\n]+)`/g, '<code class="md-inline">$1</code>');
  // Bold / italic
  text = text.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  text = text.replace(/(^|[\s(])\*([^*\n]+)\*/g, "$1<em>$2</em>");
  // Links — only http(s), escaped upstream
  text = text.replace(
    /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener">$1</a>'
  );
  // Line-level: headings, bullets, blockquotes
  text = text
    .split("\n")
    .map((line) => {
      const h = line.match(/^#{1,6}\s+(.*)$/);
      if (h) return `<span class="md-h">${h[1]}</span>`;
      const b = line.match(/^(\s*)[-*]\s+(.*)$/);
      if (b) return `${b[1]}• ${b[2]}`;
      const q = line.match(/^&gt;\s?(.*)$/);
      if (q) return `<span class="md-quote">${q[1]}</span>`;
      return line;
    })
    .join("\n");
  // Restore code blocks (strip surrounding blank lines — pre has its own box)
  text = text.replace(/\n?\u0000(\d+)\u0000\n?/g, (_, i) => codeBlocks[+i] || "");
  return text;
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
  // Interleaved ACP noise (term/tool/plan rows) must not split a response —
  // look back past it to find the still-streaming block of the same role.
  if (stream && (role === "agent" || role === "thought" || role === "term") && list.length) {
    for (let i = list.length - 1, hops = 0; i >= 0 && hops < 8; i--, hops++) {
      const entry = list[i];
      if (entry.role === role && entry.streaming) {
        // Rotate giant stream blocks: one multi-MB text node re-escaped on
        // every render tanks the whole transcript.
        if ((entry.body || "").length > 64_000) {
          entry.streaming = false;
          break;
        }
        if (role === "term") {
          entry.body = (entry.body || "") + (entry.body ? "\n" : "") + text;
        } else {
          entry.body = (entry.body || "") + text;
        }
        entry.at = at;
        if (sessionId === state.selectedSession) {
          if (i === list.length - 1) {
            patchLastTranscriptBody(entry);
          } else {
            renderTranscript();
          }
        }
        return;
      }
      // Skip over noise rows that landed mid-stream; stop at real content.
      if (entry.role === "term" || entry.role === "tool" || entry.role === "plan") continue;
      break;
    }
  }

  // Non-stream after stream → close previous stream bubble. Noise rows
  // (term/tool/plan) don't end a response that is still streaming.
  if (!stream && list.length && role !== "term" && role !== "tool" && role !== "plan") {
    for (const entry of list.slice(-8)) {
      if (entry.streaming) entry.streaming = false;
    }
  }

  const entry = {
    role,
    body: text,
    at,
    streaming: stream && (role === "agent" || role === "thought" || role === "term"),
  };
  if (opts.meta) entry.meta = opts.meta;
  list.push(entry);
  // Cap memory so huge TTY logs stay snappy
  if (list.length > 2000) {
    list.splice(0, list.length - 2000);
  }
  if (sessionId === state.selectedSession) {
    renderTranscript();
  }
}

/** True when the user is following the tail (within a small threshold). */
function isNearBottom(root) {
  if (!root) return true;
  return root.scrollHeight - root.scrollTop - root.clientHeight < 48;
}

/** Fast path: update only the last streaming block. */
/// Pin the transcript to the bottom reliably: once now, once after layout,
/// and once more after async content (bomb sprites) settles height.
function scrollTranscriptBottom() {
  const root = $("transcript");
  if (!root) return;
  const pin = () => {
    root.scrollTop = root.scrollHeight;
  };
  pin();
  requestAnimationFrame(() => {
    pin();
    setTimeout(pin, 60);
  });
}

function patchLastTranscriptBody(entry) {
  const root = $("transcript");
  if (!root) {
    renderTranscript();
    return;
  }
  const follow = isNearBottom(root);
  // Skip the synthetic live-hint row — patching it froze the real bubble.
  const blocks = root.querySelectorAll(".t-block:not(.term-live)");
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
  const lines = String(entry.body || "").split("\n");
  if (entry.role === "term" && !entry.expanded && lines.length > 1) {
    // Collapsed live log: tail the newest line + bump the counter, keeping
    // the expand button intact instead of dumping the whole payload.
    const tail = last.querySelector(".term-tail");
    const btn = last.querySelector(".term-toggle");
    if (tail && btn) {
      tail.textContent = lines[lines.length - 1].slice(0, 400);
      btn.textContent = `▸ ${lines.length - 1} more line${lines.length > 2 ? "s" : ""}`;
    } else {
      // Structure not built yet (entry just grew past one line).
      renderTranscript({ keepScroll: true });
      return;
    }
  } else {
    body.textContent = entry.body || "";
  }
  if (time) time.textContent = shortTime(entry.at || "");
  last.classList.toggle("streaming", !!entry.streaming);
  // Only follow the stream if the user was already at the bottom.
  if (follow) scrollTranscriptBottom();
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
  if (role === "approval") return "ask";
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
  if (role === "approval") return "wait";
  return "idle";
}

// ── ELI12 explainer panel (right sidebar) ────────────────────────────────
function explainListFor(sid) {
  if (!state.explainBySession.has(sid)) state.explainBySession.set(sid, []);
  return state.explainBySession.get(sid);
}

function handleExplainEvent(sid, payload) {
  if (!sid) return;
  const kind = String(payload.kind || "tick");
  if (kind === "pending") {
    state.explainPending = true;
    if (sid === state.selectedSession) renderExplainFeed();
    return;
  }
  state.explainPending = false;
  const text = String(payload.text || "").trim();
  if (!text) return;
  const list = explainListFor(sid);
  list.push({ text, kind, requestId: payload.requestId || null, at: payload.at || nowIso() });
  if (list.length > 50) list.splice(0, list.length - 50);

  // Approval explanations also land under the matching approval card.
  if (kind === "approval" && payload.requestId) {
    const entries = getTranscript(sid);
    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (e.role === "approval" && e.meta?.requestId === payload.requestId) {
        e.meta.explanation = text;
        if (sid === state.selectedSession) renderTranscript({ keepScroll: true });
        break;
      }
    }
  }
  if (sid === state.selectedSession) renderExplainFeed();
}

function renderExplainFeed() {
  const root = $("explain-feed");
  if (!root) return;
  const follow = isNearBottom(root);
  const sid = state.selectedSession;
  const list = sid ? state.explainBySession.get(sid) || [] : [];
  if (!state.explainerEnabled) {
    root.innerHTML = `<div class="empty-hint">Narrator is off — toggle it on to get plain-English explanations.</div>`;
    return;
  }
  if (!list.length && !state.explainPending) {
    root.innerHTML = `<div class="empty-hint">${
      sid
        ? "Waiting for activity in this thread — I'll explain it as it happens."
        : "Select a thread and send a prompt — I'll explain what the agent is doing in plain English."
    }</div>`;
    return;
  }
  // Chronological like a chat: newest at the bottom, auto-follow the tail
  // unless the user scrolled up to read.
  const pending = state.explainPending
    ? `<div class="explain-card pending"><span class="explain-dots">thinking…</span></div>`
    : "";
  root.innerHTML =
    list
      .map((e) => {
        const cls = e.kind === "approval" ? " approval" : e.kind === "error" ? " error" : "";
        return `<div class="explain-card${cls}">
  <div class="explain-ts">${escapeHtml(shortTime(e.at))}${e.kind === "approval" ? " · about the approval" : ""}</div>
  <div class="explain-text">${escapeHtml(e.text)}</div>
</div>`;
      })
      .join("") + pending;
  if (follow) root.scrollTop = root.scrollHeight;
}

/** Mark an approval card resolved and refresh it if visible. */
function resolveApprovalEntry(sessionId, requestId, resolution) {
  if (!sessionId || !requestId) return;
  const list = getTranscript(sessionId);
  for (let i = list.length - 1; i >= 0; i--) {
    const e = list[i];
    if (e.role === "approval" && e.meta?.requestId === requestId) {
      e.meta.resolved = resolution;
      break;
    }
  }
  if (sessionId === state.selectedSession) {
    renderTranscript({ keepScroll: true });
  }
}

function renderTranscript({ keepScroll = false } = {}) {
  // Respect the reader: full re-renders only pin to bottom when the user was
  // already following the tail. Switching threads always starts at the tail.
  const rootEl = $("transcript");
  const switchedSession = renderTranscript._lastSid !== state.selectedSession;
  renderTranscript._lastSid = state.selectedSession;
  const wasNearBottom = switchedSession || isNearBottom(rootEl);
  const prevScroll = rootEl?.scrollTop ?? null;
  const root = rootEl;
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
  const backendName = String(sess?.backend || "grok").toLowerCase();
  $("composer-session").textContent = `${shortId(sid)} · ${sess?.status || "?"}`;
  $("composer-model").textContent = [backendName, sess?.model].filter(Boolean).join(" · ");
  updateThreadGitRow(sess);

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
    turnActive()
      ? `<div class="t-block term streaming term-live">
  <div class="t-role">${bombHtml(liveView.mood, "xs")}<span>live</span><span class="stream-caret" aria-hidden="true"></span></div>
  <div class="t-body">${escapeHtml(liveView.subtitle)}</div>
</div>`
      : "";

  root.innerHTML =
    entries
      .map((e, idx) => {
        const role = e.role || "system";
        // Agent speech is labeled by which agent is talking, not a fixed "grok".
        const label = role === "agent" ? backendName : termPrefix(role);
        const streamCls = e.streaming ? " streaming" : "";
        // Agent-authored text renders markdown; everything else stays literal.
        let body =
          role === "agent" || role === "thought" || role === "user"
            ? renderMarkdown(e.body)
            : escapeHtml(e.body);
        if (role === "approval") {
          const m = e.meta || {};
          const opts = Array.isArray(m.options) ? m.options : [];
          const buttons = opts
            .map(
              (o) => `<button class="approval-btn kind-${escapeHtml(String(o.kind || "other"))}"
  data-sid="${escapeHtml(String(m.sid || sid))}"
  data-request-id="${escapeHtml(String(m.requestId || ""))}"
  data-option-id="${escapeHtml(String(o.id))}"${m.resolved ? " disabled" : ""}>${escapeHtml(o.label || o.kind || o.id)}</button>`
            )
            .join("");
          const deny = m.resolved
            ? ""
            : `<button class="approval-btn kind-cancel"
  data-sid="${escapeHtml(String(m.sid || sid))}"
  data-request-id="${escapeHtml(String(m.requestId || ""))}"
  data-option-id="">Cancel</button>`;
          const explain = m.explanation
            ? `<div class="approval-explain">💡 ${escapeHtml(String(m.explanation))}</div>`
            : "";
          const foot = m.resolved
            ? `${explain}<div class="approval-resolved">resolved · ${escapeHtml(String(m.resolved))}</div>`
            : `${explain}<div class="approval-actions">${buttons}${deny}</div>`;
          return `<div class="t-block approval${m.resolved ? "" : " pending"}">
  <div class="t-role"><span class="t-ts">${escapeHtml(shortTime(e.at || ""))}</span>${bombHtml("wait", "xs")}<span>${label}</span></div>
  <div class="t-body">${escapeHtml(e.body)}${foot}</div>
</div>`;
        }
        // Multi-line ACP noise collapses by default — streaming included
        // (giant live protocol dumps were eating the whole column). While
        // streaming, the collapsed view tails the newest line.
        const lines = String(e.body || "").split("\n");
        if (role === "term" && lines.length > 1) {
          if (e.expanded) {
            body = `${escapeHtml(e.body)}\n<button class="term-toggle" type="button" data-idx="${idx}">▾ collapse</button>`;
          } else {
            const shown = e.streaming ? lines[lines.length - 1] : lines[0];
            body = `<span class="term-tail">${escapeHtml(shown).slice(0, 400)}</span> <button class="term-toggle" type="button" data-idx="${idx}">▸ ${lines.length - 1} more line${lines.length > 2 ? "s" : ""}</button>`;
          }
        }
        return `<div class="t-block ${escapeHtml(role)}${streamCls}">
  <div class="t-role"><span class="t-ts">${escapeHtml(shortTime(e.at || ""))}</span>${bombHtml(roleBombMood(role), "xs")}<span>${label}</span>${e.streaming ? '<span class="stream-caret" aria-hidden="true"></span>' : ""}</div>
  <div class="t-body">${body}</div>
</div>`;
      })
      .join("") + liveHint;
  root.querySelectorAll(".term-toggle").forEach((btn) => {
    btn.onclick = (ev) => {
      ev.stopPropagation();
      const entry = entries[Number(btn.dataset.idx)];
      if (entry) {
        entry.expanded = !entry.expanded;
        renderTranscript({ keepScroll: true });
      }
    };
  });
  if ((keepScroll || !wasNearBottom) && prevScroll != null) {
    root.scrollTop = prevScroll;
  } else {
    scrollTranscriptBottom();
  }
  updateBombChrome();
}

// ── Threads / agents ────────────────────────────────────────────────────
function threadProjectKey(s) {
  return s.projectRoot || s.project_root || s.cwd || "(no project)";
}

function projectCollapsed(key) {
  return localStorage.getItem(`bomb.projCollapsed.${key}`) === "1";
}

function renderThreadRow(s) {
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
  const msgs = s.messageCount ?? s.message_count ?? 0;
  const liveTag = live
    ? ""
    : `<span class="badge saved" title="Restored from disk">saved</span>`;
  const backend = String(s.backend || "grok").toLowerCase();
  const backendTag =
    backend === "grok"
      ? ""
      : `<span class="badge backend-${escapeHtml(backend)}" title="Agent backend">${escapeHtml(backend)}</span>`;
  const brain = String(s.brainMode || s.brain_mode || "").toLowerCase();
  let brainTag = "";
  if (live && brain === "full_brain") {
    brainTag = `<span class="badge brain-full" title="Agent reloaded prior ACP session">full brain</span>`;
  } else if (live && brain === "history_only") {
    brainTag = `<span class="badge brain-history" title="New ACP process; transcript injected as context">history-only</span>`;
  }
  const worktreeTag = (s.projectRoot || s.project_root)
    ? `<span class="badge branch" title="Isolated worktree — land to merge back">🌱 ${escapeHtml(s.worktree || "worktree")}</span>`
    : "";
  const syncTag = s.needsSync
    ? `<span class="badge needs-sync" title="Landing conflicted — Sync, let the agent resolve, land again">needs sync</span>`
    : "";
  // Smart label when we have one; the raw id lives in the tooltip.
  const title = s.label
    ? escapeHtml(s.label)
    : `${escapeHtml(mode)} · ${escapeHtml(shortId(id))}`;
  return `<div class="thread-item ${selected}${live ? "" : " restored"}" data-id="${escapeHtml(id)}" title="${escapeHtml(shortId(id))} · ${escapeHtml(s.cwd || "")}">
  <div class="name">${bombHtml(bombMood, "xs")} <span class="thread-title">${title}</span>
    <button class="thread-delete" type="button" data-id="${escapeHtml(id)}" title="Delete thread">✕</button></div>
  <div class="meta"><span class="badge ${badgeCls}">${bombHtml(bombMood, "xs")}${escapeHtml(status)}</span>
  ${liveTag}${backendTag}${brainTag}${worktreeTag}${syncTag}
  <span>${escapeHtml(isMock ? "mock" : model || "—")}</span>
  ${msgs ? `<span class="muted">${msgs} msg</span>` : ""}</div>
</div>`;
}

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
  // Group by project (real folder) — worktree threads collapse under it.
  // Persisted projects appear even when they have no threads yet.
  const groups = new Map();
  for (const p of state.projects || []) {
    if (!groups.has(p)) groups.set(p, []);
  }
  for (const s of sorted) {
    const key = threadProjectKey(s);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(s);
  }
  root.innerHTML = [...groups.entries()]
    .map(([key, list]) => {
      const name = key.split("/").filter(Boolean).pop() || key;
      const collapsed = projectCollapsed(key) && list.length > 0;
      const active = $("cwd")?.value?.replace(/\/+$/, "") === key ? " active" : "";
      const rows = collapsed
        ? ""
        : list.length
          ? list.map(renderThreadRow).join("")
          : `<div class="empty-hint project-empty">no threads yet — hit + to start one here</div>`;
      const marker = active
        ? `<span class="badge active-proj" title="New threads start in this project">● active</span>`
        : `<span class="project-set">set active</span>`;
      return `<div class="project-group${active ? " active" : ""}" data-key="${escapeHtml(key)}">
  <button class="project-header${active}" type="button" data-key="${escapeHtml(key)}" title="${escapeHtml(key)}${active ? " — active project" : " — click to make this the active project"}">
    <span class="project-caret" data-key="${escapeHtml(key)}">${collapsed ? "▸" : "▾"}</span>
    <span class="project-name">${escapeHtml(name)}</span>
    ${marker}
    <span class="project-count muted">${list.length}</span>
  </button>
  ${rows}
</div>`;
    })
    .join("");

  root.querySelectorAll(".thread-item").forEach((el) => {
    el.onclick = () => selectSession(el.dataset.id);
  });
  root.querySelectorAll(".project-header").forEach((el) => {
    el.onclick = (ev) => {
      ev.stopPropagation();
      const key = el.dataset.key;
      if (ev.target.classList.contains("project-caret")) {
        // Caret toggles collapse; the name selects the project.
        const now = !projectCollapsed(key);
        localStorage.setItem(`bomb.projCollapsed.${key}`, now ? "1" : "0");
        renderThreads();
        return;
      }
      // Clicking a project makes it the active folder for the next thread.
      setProjectCwd(key);
      renderThreads();
      pushEvent(`project → ${key.split("/").filter(Boolean).pop()}`, "ok", null, { force: true });
    };
  });
  root.querySelectorAll(".thread-delete").forEach((el) => {
    el.onclick = (ev) => {
      ev.stopPropagation();
      deleteThread(el.dataset.id);
    };
  });
  updateBombChrome();
}

/** Show Land/Sync controls only for threads that own a worktree. */
function updateThreadGitRow(sess) {
  const row = $("thread-git-row");
  if (!row) return;
  const isolated = !!(sess && (sess.projectRoot || sess.project_root));
  row.style.display = isolated ? "" : "none";
  if (isolated) {
    $("thread-branch").textContent = `🌱 ${sess.worktree || "worktree"}`;
  }
}

async function landThread() {
  const id = state.selectedSession;
  if (!id) return;
  const btn = $("btn-land-thread");
  if (btn) btn.disabled = true;
  try {
    const res = await invoke("land_thread", { id });
    const sess = state.sessions.find((s) => s.id === id);
    if (res.status === "landed") {
      if (sess) sess.needsSync = false;
      pushEvent(`⬆ landed into ${res.targetBranch}`, "ok", "boom", { force: true, milestone: true });
    } else {
      if (sess) sess.needsSync = true;
      pushEvent(
        `landing conflicted (${(res.files || []).join(", ")}) — hit Sync, let the agent resolve, land again`,
        "err",
        "wait",
        { force: true, milestone: true }
      );
    }
    renderThreads();
  } catch (e) {
    toastError(e);
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function syncThread() {
  const id = state.selectedSession;
  if (!id) return;
  const btn = $("btn-sync-thread");
  if (btn) btn.disabled = true;
  try {
    const res = await invoke("sync_thread", { id });
    const sess = state.sessions.find((s) => s.id === id);
    if (res.status === "synced") {
      pushEvent(`⟳ synced from ${res.targetBranch}`, "ok", null, { force: true });
    } else {
      // Conflicts live in the worktree now — prefill a resolution prompt so
      // one click + send puts this thread's own agent on conflict duty.
      const files = (res.files || []).join(", ");
      const promptBox = $("prompt");
      if (promptBox && !promptBox.value.trim()) {
        promptBox.value = `Merge conflicts from ${res.targetBranch} were left in this worktree (${files}). Resolve them, keeping both sides' intent, then commit the result.`;
      }
      pushEvent(`sync left conflicts in ${files} — prompt prefilled, send it to let the agent resolve`, "err", "wait", {
        force: true,
        milestone: true,
      });
    }
    if (sess && res.status === "synced") sess.needsSync = false;
    renderThreads();
  } catch (e) {
    toastError(e);
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function deleteThread(id) {
  const sess = state.sessions.find((s) => s.id === id);
  if (!sess) return;
  const name = sess.label || shortId(id);
  if (!confirm(`Delete thread "${name}"? Its transcript is removed permanently.`)) return;
  let removeWorktree = false;
  if (sess.projectRoot || sess.project_root) {
    removeWorktree = confirm(
      "Also remove its git worktree?\n\nOK = remove worktree (unlanded changes are LOST)\nCancel = keep the worktree on disk"
    );
  }
  try {
    await invoke("remove_session", { id, removeWorktree });
    state.transcriptBySession.delete(id);
    state.explainBySession.delete(id);
    if (state.selectedSession === id) state.selectedSession = null;
    await refreshSessions();
    pushEvent(`deleted thread ${name}`, "ok", null, { force: true });
  } catch (e) {
    toastError(e);
  }
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

/** Cancelled/failed turns never deliver terminal tool events — sweep the
 *  session's in-flight tool entries so the panel doesn't say "running" forever. */
function sweepToolsForSession(sid, finalStatus = "cancelled") {
  if (!sid) return;
  let changed = false;
  for (const t of state.tools) {
    if (t.sessionId === sid && !isToolTerminal(t.status)) {
      t.status = finalStatus;
      changed = true;
    }
  }
  openToolsFor(sid).clear();
  if (changed) renderTools();
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
  // Don't discard a path the user just typed for their next session.
  if (sess?.cwd && !state.cwdDirty) setProjectCwd(sess.cwd, { remember: false });
  syncSelectorsToSession(sess);
  if (id) await loadTranscriptFromDb(id);
  renderTranscript();
  // Point the ELI12 narrator at the newly selected thread (best-effort).
  state.explainPending = false;
  renderExplainFeed();
  invoke("explainer_focus", { id: id || null }).catch(() => {});
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
    talkNote(sid, isThought ? "think" : "speak", text);
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
    talkNote(sid, "tool", tool);
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
    // Only nudge presence during an actual turn — agents emit an initial plan
    // right after session start, which left the dock stuck on "thinking".
    if (sid && P.turnActive(presenceFor(sid))) {
      noteTurn("think", { note: pe.title || "plan update" }, sid);
    }
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
        sweepToolsForSession(sid, "failed");
        endTurnPresence(sid, "error", String(ev.status));
      } else if (st.includes("cancel")) {
        endAgentStream(sid);
        sweepToolsForSession(sid, "cancelled");
        endTurnPresence(sid, "error", "Cancelled");
      } else if (st.includes("idle") || st.includes("complete")) {
        endAgentStream(sid);
        const p = presenceFor(sid);
        if (P.turnActive(p) || p.replyChars || p.toolCount) {
          talkNote(sid, "boom");
          flashBoomThenIdle(undefined, sid);
        } else {
          talkNote(sid, "idle");
          noteTurn("idle", {}, sid);
        }
      } else if (st.includes("run") && !P.turnActive(presenceFor(sid))) {
        noteTurn("think", { note: "Session running" }, sid);
      }
    }
    refreshSessions();
  } else if (type === "session_cancelled" || type === "sessionCancelled") {
    endAgentStream(sid);
    sweepToolsForSession(sid, "cancelled");
    appendTranscript(sid, "term", "session cancelled");
    pushEvent(`cancelled · ${shortId(sid)}`, "err", "error", { force: true, milestone: true });
    if (sid) endTurnPresence(sid, "error", "Cancelled");
    refreshSessions();
  } else if (type === "error") {
    // Session-less errors are host-level — status feed only. Attributing them
    // to whatever thread happens to be selected caused phantom red rows.
    pushEvent(ev.message || "error", "err", "error", { force: true, milestone: true });
    if (!state.ready) setStatus("error", ev.message || "error");
    if (sid) {
      endAgentStream(sid);
      appendTranscript(sid, "error", ev.message || "error");
      endTurnPresence(sid, "error", ev.message || "error");
    }
  } else if (type === "approval_required" || type === "approvalRequired") {
    const autoApproved = !!(ev.auto_approved ?? ev.autoApproved);
    if (autoApproved) {
      appendTranscript(sid, "term", `auto-approved (yolo) · ${ev.tool || "?"}`);
      pushEvent(`auto-approved · ${ev.tool || "?"}`, "ok", null, { force: true });
      return;
    }
    endAgentStream(sid);
    appendTranscript(sid, "approval", ev.summary || `${ev.tool || "tool"} requests permission`, nowIso(), {
      meta: {
        requestId: ev.request_id || ev.requestId || "",
        options: ev.options || [],
        sid,
      },
    });
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
  } else if (type === "approval_resolved" || type === "approvalResolved") {
    const requestId = ev.request_id || ev.requestId || "";
    const cancelled = !!ev.cancelled;
    resolveApprovalEntry(
      sid,
      requestId,
      cancelled ? "cancelled" : ev.option_id || ev.optionId || "allowed"
    );
    pushEvent(`approval ${cancelled ? "cancelled" : "answered"} · ${shortId(sid)}`, "", null, {
      force: true,
    });
    if (sid) noteTurn("run", { note: "Approval resolved" }, sid);
  } else if (type === "raw") {
    const payload = ev.payload || ev;
    if (payload?.channel === "explain") {
      handleExplainEvent(sid, payload);
      return;
    }
    if (payload?.channel === "thread" && payload?.kind === "label") {
      // Smart-name upgrade (slug → narrator title) — update the row live.
      const sess = state.sessions.find((s) => s.id === sid);
      if (sess && payload.label) {
        sess.label = String(payload.label);
        renderThreads();
      }
      return;
    }
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
      // Only the owning session's thread gets the line — never the selected
      // one as a fallback (another session's stderr showed up mid-thread).
      if (!sid) {
        pushEvent(String(payload.line).slice(0, 120), "", null);
        return;
      }
      appendTranscript(sid, "term", String(payload.line), nowIso(), { stream: true });
      if (P.turnActive(presenceFor(sid))) {
        noteTurn(
          presenceFor(sid).phase === "idle" ? "think" : presenceFor(sid).phase,
          { note: String(payload.line).slice(0, 80) },
          sid
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
      if (!sid) return; // agent text without a session id has nowhere to go
      appendTranscript(sid, "agent", maybe, nowIso(), { stream: true });
      const p = presenceFor(sid);
      noteTurn(
        "reply",
        {
          replyChars: (p.replyChars || 0) + maybe.length,
          preview: clipPreview(maybe),
        },
        sid
      );
    } else if (sid) {
      // Unrecognized payload for a known session → its own thread, clipped.
      const dump = JSON.stringify(payload);
      if (dump && dump !== "{}" && dump !== "null") {
        appendTranscript(
          sid,
          "term",
          dump.length > 400 ? dump.slice(0, 400) + "…" : dump,
          nowIso(),
          { stream: true }
        );
      }
    } else {
      console.debug("unattributed raw event", payload);
    }
  } else if (sid) {
    appendTranscript(sid, "term", `event ${type} · ${shortId(sid)}`);
  } else {
    pushEvent(`event ${type}`, "", null);
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
    setProjectCwd(s.defaultCwd, { remember: false });
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
    } else if (!state.transcriptLoaded.has(state.selectedSession)) {
      // First sight of this thread: hydrate from SQLite. Already-loaded
      // threads update via events — a full innerHTML rebuild on every status
      // change destroyed the DOM mid-scroll.
      await loadTranscriptFromDb(state.selectedSession);
      renderTranscript();
      updateBombChrome();
    } else {
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

// ── Agent Talk visualizer (fuse & fireworks) ─────────────────────────────
// Each live agent is a pixel bomb: fuse burns while it thinks, sparks carry
// thought fragments, tool calls stamp the casing, and a finished turn pops
// into a firework whose embers are words from the reply.
const talk = {
  agents: new Map(), // sessionId → agent viz state
  collapsed: localStorage.getItem("bomb.talkCollapsed") === "1",
  raf: null,
  lastFrame: 0,
};

const TALK_COLORS = {
  grok: "#8be28b",
  claude: "#d97757",
  codex: "#74aa9c",
};

function talkAgent(sessionId) {
  if (!talk.agents.has(sessionId)) {
    talk.agents.set(sessionId, {
      id: sessionId,
      backend: "grok",
      phase: "idle", // idle | think | speak | tool | boom
      fuse: 0, // 0..1 burn progress within current turn
      sparks: [], // {x,y,vx,vy,life,text?,color}
      embers: [], // firework word particles
      toolFlash: null, // {name, life}
      lastText: "",
      replyText: "",
      boomAt: 0,
    });
  }
  return talk.agents.get(sessionId);
}

function talkFragment(text) {
  const t = String(text || "").replace(/\s+/g, " ").trim();
  if (!t) return "";
  return t.length > 26 ? t.slice(-26) : t;
}

/// Feed one control-event into the visualizer.
function talkNote(sessionId, kind, text) {
  if (!sessionId) return;
  const a = talkAgent(sessionId);
  const sess = state.sessions.find((s) => s.id === sessionId);
  if (sess?.backend) a.backend = String(sess.backend).toLowerCase();
  const color = TALK_COLORS[a.backend] || TALK_COLORS.grok;

  if (kind === "think" || kind === "speak") {
    a.phase = kind;
    // Beat energy: every token/thought signal pushes the dancers harder.
    a.energy = Math.min(1, (a.energy || 0) + 0.12);
    a.fuse = Math.min(1, a.fuse + 0.006);
    a.lastText = talkFragment(text) || a.lastText;
    if (kind === "speak") a.replyText += ` ${text || ""}`;
    if (a.sparks.length < 36 && Math.random() < 0.6) {
      a.sparks.push({
        x: 0, y: 0,
        vx: (Math.random() - 0.5) * 26,
        vy: -22 - Math.random() * 26,
        life: 1,
        text: Math.random() < 0.22 ? a.lastText : null,
        color: kind === "think" ? "#e8b04b" : color,
      });
    }
  } else if (kind === "tool") {
    a.phase = "tool";
    a.energy = Math.min(1, (a.energy || 0) + 0.2);
    a.toolFlash = { name: talkFragment(text).slice(0, 14) || "tool", life: 1 };
    a.fuse = Math.min(1, a.fuse + 0.02);
  } else if (kind === "boom") {
    // Turn finished — firework of reply words.
    a.phase = "boom";
    a.boomAt = performance.now();
    const words = a.replyText.split(/\s+/).filter((w) => w.length > 2).slice(-18);
    for (const w of words) {
      const ang = Math.random() * Math.PI * 2;
      const speed = 30 + Math.random() * 55;
      a.embers.push({
        x: 0, y: -6,
        vx: Math.cos(ang) * speed,
        vy: Math.sin(ang) * speed - 24,
        life: 1,
        text: w.slice(0, 14),
        color,
      });
    }
    a.replyText = "";
    a.fuse = 0;
  } else if (kind === "idle") {
    if (a.phase !== "boom") a.phase = "idle";
  }
  ensureTalkLoop();
}

// Original ASCII dance banks for working agents (hand-drawn frames; every
// line in a frame is the same width so centered monospace stays aligned).
// GROOVE: standing warm-up. TWERK: bent-over hip-bounce — the visualizer
// drops into it when the token stream runs hot (energy > threshold).
const TALK_GROOVE_FRAMES = [
  [" \\o/   ", "  |    ", " <|>   ", "  |    ", "  |    ", " / \\   "],
  [" \\o_   ", "  |    ", " <|    ", "  |    ", "  |    ", " / \\   "],
  [" _o/   ", "  |    ", "  |>   ", "  |    ", "  |    ", " / \\   "],
  [" \\o/   ", "  |    ", "  |>   ", " <|    ", "  |    ", " / \\   "],
];
const TALK_TWERK_FRAMES = [
  // alternating hips-up / hips-down every frame = bounce; drifts R→L
  [" o     ", "  \\_   ", "   (_) ", "   ||  ", "   ||  ", "  _||_ "],
  [" o     ", "  \\__  ", "   ||  ", "  '(_)'", "   ||  ", "  _||_ "],
  [" o     ", "  \\_   ", "    (_)", "   ||  ", "   ||  ", "  _||_ "],
  [" o     ", "  \\__  ", "   ||  ", "   '(_)", "   ||  ", "  _||_ "],
  [" o     ", "  \\_   ", "  (_)  ", "   ||  ", "   ||  ", "  _||_ "],
  [" o     ", "  \\__  ", "   ||  ", "  (_)  ", "   ||  ", "  _||_ "],
];
const TALK_NOTES = ["♪", "♫", "♬"];
const TWERK_ENERGY_THRESHOLD = 0.35;

/** Draw one dancer frame centered at (cx, baseY), lines rising upward. */
function drawDancer(ctx, frame, cx, baseY, px, color, alpha) {
  ctx.save();
  ctx.globalAlpha = alpha;
  ctx.fillStyle = color;
  ctx.textAlign = "center";
  ctx.font = `bold ${px}px ui-monospace, monospace`;
  const lineH = px;
  frame.forEach((line, li) => {
    ctx.fillText(line, cx, baseY - (frame.length - 1 - li) * lineH);
  });
  ctx.restore();
}

function ensureTalkLoop() {
  if (talk.collapsed || talk.raf) return;
  talk.lastFrame = performance.now();
  talk.raf = requestAnimationFrame(talkFrame);
}

function talkFrame(now) {
  talk.raf = null;
  const dt = Math.min(0.05, (now - talk.lastFrame) / 1000);
  talk.lastFrame = now;
  const canvas = $("agent-talk-canvas");
  if (!canvas || talk.collapsed) return;
  const ctx = canvas.getContext("2d");
  const wrap = canvas.parentElement;
  if (canvas.width !== wrap.clientWidth) canvas.width = Math.max(200, wrap.clientWidth);
  const W = canvas.width;
  const H = canvas.height;
  ctx.clearRect(0, 0, W, H);

  // Only live agents appear on the bench.
  const liveIds = new Set(
    (state.sessions || [])
      .filter((s) => s.live !== false && !String(s.status || "").includes("saved"))
      .map((s) => s.id)
  );
  for (const id of [...talk.agents.keys()]) {
    if (!liveIds.has(id)) talk.agents.delete(id);
  }
  const agents = [...talk.agents.values()];
  if (!agents.length) {
    ctx.fillStyle = "rgba(255,255,255,0.25)";
    ctx.font = "11px ui-monospace, monospace";
    ctx.textAlign = "center";
    ctx.fillText("no live agents — fuses are cold", W / 2, H / 2);
    return;
  }

  const slot = W / agents.length;
  let anyActive = false;
  agents.forEach((a, i) => {
    const cx = slot * i + slot / 2;
    const cy = H - 42;
    const color = TALK_COLORS[a.backend] || TALK_COLORS.grok;
    const active = a.phase === "think" || a.phase === "speak" || a.phase === "tool";
    if (active || a.sparks.length || a.embers.length || a.toolFlash) anyActive = true;

    if (active) {
      // Beat sync: energy decays between signals; frame rate rides it
      // (idle stream ≈ 3fps sway, hot stream ≈ 12fps bounce).
      a.energy = Math.max(0, (a.energy || 0) - dt * 0.2);
      a.beat = (a.beat || 0) + dt * (3 + (a.energy || 0) * 9);
      const bank =
        (a.energy || 0) > TWERK_ENERGY_THRESHOLD ? TALK_TWERK_FRAMES : TALK_GROOVE_FRAMES;
      const idx = Math.floor(a.beat) % bank.length;

      ctx.shadowColor = color;
      ctx.shadowBlur = 10;
      // Backup dancers first (behind, smaller, out of phase) when there's room.
      if (slot >= 110) {
        const backupBank = bank; // formation dances the same routine
        const bl = backupBank[(idx + 2) % backupBank.length];
        const br = backupBank[(idx + 4) % backupBank.length];
        drawDancer(ctx, bl, cx - 34, cy, 10, color, 0.55);
        drawDancer(ctx, br, cx + 34, cy, 10, color, 0.55);
      }
      // Lead dancer.
      drawDancer(ctx, bank[idx], cx, cy, 13, color, 1);
      ctx.shadowBlur = 0;

      // Floating music notes — more of them the harder the stream hits.
      if (a.sparks.length < 36 && Math.random() < 0.08 + (a.energy || 0) * 0.25) {
        a.sparks.push({
          x: (Math.random() - 0.5) * 40,
          y: -bank[idx].length * 13,
          vx: (Math.random() - 0.5) * 12,
          vy: -16 - Math.random() * 10,
          life: 1,
          text: TALK_NOTES[Math.floor(Math.random() * TALK_NOTES.length)],
          color,
        });
      }
    } else {
      // Idle / post-boom: the classic dim bomb on the bench.
      ctx.font = "20px system-ui";
      ctx.textAlign = "center";
      ctx.globalAlpha = a.phase === "idle" ? 0.55 : 1;
      ctx.fillText("💣", cx, cy);
      ctx.globalAlpha = 1;
    }

    // Label.
    ctx.font = "9px ui-monospace, monospace";
    ctx.fillStyle = color;
    ctx.textAlign = "center";
    ctx.fillText(a.backend, cx, H - 26);

    // Fuse spark rides along while active (sparkler next to the dancer).
    const fx = cx + 9, fy = cy - 16;
    if (!active) {
      ctx.strokeStyle = "rgba(255,255,255,0.28)";
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.quadraticCurveTo(fx + 8, fy - 9, fx + 3, fy - 16);
      ctx.stroke();
    }
    if (active) {
      const t = a.fuse;
      const bx = fx + 8 * t * (1 - t) * 2 + 3 * t;
      const by = fy - 9 * t - 7 * t * t;
      ctx.fillStyle = "#ffd966";
      ctx.shadowColor = "#ffb84d";
      ctx.shadowBlur = 8;
      ctx.beginPath();
      ctx.arc(bx, by, 2.2 + Math.random() * 1.2, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
      // Emit a passive spark sometimes even without new tokens.
      if (a.sparks.length < 36 && Math.random() < 0.25) {
        a.sparks.push({
          x: bx - cx, y: by - cy,
          vx: (Math.random() - 0.5) * 20,
          vy: -14 - Math.random() * 18,
          life: 0.8,
          text: null,
          color: "#ffd966",
        });
      }
    }

    // Sparks (thought/speech fragments).
    for (const s of a.sparks) {
      s.x += s.vx * dt;
      s.y += s.vy * dt;
      s.vy += 26 * dt;
      s.life -= dt * 0.9;
      if (s.life <= 0) continue;
      ctx.globalAlpha = Math.max(0, s.life);
      if (s.text) {
        const isNote = TALK_NOTES.includes(s.text);
        ctx.font = isNote ? "12px ui-monospace, monospace" : "9px ui-monospace, monospace";
        ctx.fillStyle = s.color;
        ctx.textAlign = "center";
        ctx.fillText(isNote ? s.text : `“${s.text}”`, cx + s.x, cy + s.y - 14);
      } else {
        ctx.fillStyle = s.color;
        ctx.fillRect(cx + s.x, cy + s.y - 14, 2, 2);
      }
      ctx.globalAlpha = 1;
    }
    a.sparks = a.sparks.filter((s) => s.life > 0);

    // Tool stamp flash.
    if (a.toolFlash) {
      a.toolFlash.life -= dt * 1.4;
      if (a.toolFlash.life > 0) {
        ctx.globalAlpha = Math.min(1, a.toolFlash.life + 0.2);
        ctx.font = "10px ui-monospace, monospace";
        ctx.fillStyle = "#7db7ff";
        ctx.textAlign = "center";
        ctx.fillText(`🔨 ${a.toolFlash.name}`, cx, cy - 30);
        ctx.globalAlpha = 1;
      } else {
        a.toolFlash = null;
      }
    }

    // Firework embers (reply words).
    for (const e of a.embers) {
      e.x += e.vx * dt;
      e.y += e.vy * dt;
      e.vy += 34 * dt;
      e.life -= dt * 0.55;
      if (e.life <= 0) continue;
      ctx.globalAlpha = Math.max(0, e.life);
      ctx.font = "9px ui-monospace, monospace";
      ctx.fillStyle = e.color;
      ctx.textAlign = "center";
      ctx.fillText(e.text, cx + e.x, cy + e.y - 10);
      ctx.globalAlpha = 1;
    }
    a.embers = a.embers.filter((e) => e.life > 0);
    if (a.phase === "boom" && !a.embers.length) a.phase = "idle";
  });

  // Keep animating while anything moves or burns; else park until next event.
  if (anyActive) {
    talk.raf = requestAnimationFrame(talkFrame);
  }
}

function wireAgentTalk() {
  const toggle = $("agent-talk-toggle");
  const body = $("agent-talk-body");
  if (!toggle || !body) return;
  const apply = () => {
    body.style.display = talk.collapsed ? "none" : "";
    $("agent-talk-caret").textContent = talk.collapsed ? "▸" : "▾";
    toggle.setAttribute("aria-expanded", String(!talk.collapsed));
    if (!talk.collapsed) ensureTalkLoop();
  };
  toggle.onclick = () => {
    talk.collapsed = !talk.collapsed;
    localStorage.setItem("bomb.talkCollapsed", talk.collapsed ? "1" : "0");
    apply();
  };
  apply();
}

// ── Project chip (cwd) ────────────────────────────────────────────────────
const RECENT_PROJECTS_KEY = "bomb.recentProjects";

function recentProjects() {
  try {
    const list = JSON.parse(localStorage.getItem(RECENT_PROJECTS_KEY) || "[]");
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

function setProjectCwd(path, { remember = true } = {}) {
  const p = String(path || "").trim().replace(/\/+$/, "");
  if (remember) state.cwdDirty = false; // explicit choice supersedes typing
  $("cwd").value = p;
  $("project-chip-name").textContent = p || "choose project";
  $("project-chip").title = p || "Choose project folder";
  if (p && remember) {
    const list = [p, ...recentProjects().filter((x) => x !== p)].slice(0, 8);
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(list));
    // Register as a project so the sidebar group appears immediately.
    if (!state.projects.includes(p)) {
      invoke("add_project", { path: p })
        .then((projects) => {
          state.projects = Array.isArray(projects) ? projects : state.projects;
          renderThreads();
        })
        .catch(() => {});
    } else {
      renderThreads();
    }
  }
}

async function loadProjects() {
  try {
    const projects = await invoke("list_projects");
    state.projects = Array.isArray(projects) ? projects : [];
  } catch (_) {
    state.projects = [];
  }
}

function renderProjectRecents() {
  const root = $("project-recents");
  // Merge saved recents with cwds seen in thread history.
  const fromThreads = (state.sessions || []).map((s) => s.cwd).filter(Boolean);
  const seen = new Set();
  const items = [...recentProjects(), ...fromThreads].filter((p) => {
    if (!p || seen.has(p)) return false;
    seen.add(p);
    return true;
  });
  if (!items.length) {
    root.innerHTML = `<div class="empty-hint">No recent projects</div>`;
    return;
  }
  root.innerHTML = items
    .slice(0, 8)
    .map((p) => {
      const name = p.split("/").filter(Boolean).pop() || p;
      return `<button class="project-recent" type="button" data-path="${escapeHtml(p)}" title="${escapeHtml(p)}">
        <span class="project-recent-name">${escapeHtml(name)}</span>
        <span class="project-recent-path muted">${escapeHtml(p)}</span>
      </button>`;
    })
    .join("");
  root.querySelectorAll(".project-recent").forEach((el) => {
    el.onclick = () => {
      setProjectCwd(el.dataset.path);
      toggleProjectMenu(false);
    };
  });
}

function toggleProjectMenu(show) {
  const menu = $("project-menu");
  const next = show ?? menu.style.display === "none";
  menu.style.display = next ? "" : "none";
  if (next) {
    renderProjectRecents();
    $("project-path-input").value = $("cwd").value || "";
  }
}

function wireProjectChip() {
  $("project-chip").onclick = (e) => {
    e.stopPropagation();
    toggleProjectMenu();
  };
  document.addEventListener("click", (e) => {
    const menu = $("project-menu");
    if (menu.style.display !== "none" && !menu.contains(e.target) && e.target !== $("project-chip")) {
      toggleProjectMenu(false);
    }
  });
  $("btn-project-path-set").onclick = () => {
    const p = $("project-path-input").value.trim();
    if (p) {
      setProjectCwd(p);
      toggleProjectMenu(false);
    }
  };
  $("project-path-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") $("btn-project-path-set").click();
  });
  $("btn-browse-folder").onclick = async () => {
    try {
      const picked = await window.__TAURI__.dialog.open({
        directory: true,
        multiple: false,
        title: "Choose project folder",
        defaultPath: $("cwd").value || undefined,
      });
      if (picked) {
        setProjectCwd(picked);
        toggleProjectMenu(false);
      }
    } catch (e) {
      toastError(e);
    }
  };
}

// ── Agent backend / model selection ──────────────────────────────────────
const CUSTOM_MODEL_VALUE = "__custom__";

// Plan / yolo are mutually exclusive pill toggles; neither = default mode.
function modeOn(id) {
  return !!$(id)?.classList.contains("active");
}

function setMode(id, on) {
  const el = $(id);
  if (!el) return;
  el.classList.toggle("active", on);
  el.setAttribute("aria-pressed", String(on));
}

function wireModeButtons() {
  // Push a toggle change into the live selected session (best-effort —
  // saved threads pick the values up on next send instead).
  const applyLive = async (cmd, enabled) => {
    const sess = state.sessions.find((s) => s.id === state.selectedSession);
    if (!sess || sess.live === false) return;
    try {
      await invoke(cmd, { id: state.selectedSession, enabled });
      pushEvent(`${cmd === "set_plan_mode" ? "plan" : "yolo"} ${enabled ? "on" : "off"} · ${shortId(state.selectedSession)}`, "ok", null, { force: true });
    } catch (e) {
      pushEvent(`mode change failed: ${e?.message || e}`, "err", "error", { force: true });
    }
  };
  $("plan-mode")?.addEventListener("click", () => {
    const next = !modeOn("plan-mode");
    setMode("plan-mode", next);
    if (next) setMode("always-approve", false);
    applyLive("set_plan_mode", next);
    if (next) applyLive("set_always_approve", false);
  });
  $("always-approve")?.addEventListener("click", () => {
    const next = !modeOn("always-approve");
    setMode("always-approve", next);
    if (next) setMode("plan-mode", false);
    applyLive("set_always_approve", next);
  });
  // Worktree isolation applies at thread START only (no live toggle).
  $("worktree-mode")?.addEventListener("click", () => {
    setMode("worktree-mode", !modeOn("worktree-mode"));
  });
}

function currentBackend() {
  return $("agent-backend")?.value || "grok";
}

function currentModel() {
  const sel = $("agent-model");
  if (!sel) return null;
  let model = sel.value;
  if (model === CUSTOM_MODEL_VALUE) {
    model = $("agent-model-custom")?.value?.trim?.() || "";
  }
  if (!model || model.toLowerCase() === "default") {
    // Legacy free-text field still honored when the selector is untouched.
    const legacy = $("model")?.value?.trim?.() || "";
    return !legacy || legacy.toLowerCase() === "default" ? null : legacy;
  }
  return model;
}

function populateModelSelect(backendInfo) {
  const sel = $("agent-model");
  if (!sel || !backendInfo) return;
  const remembered = localStorage.getItem(`bomb.model.${backendInfo.id}`);
  const models = backendInfo.models || [];
  sel.innerHTML =
    models
      .map((m) => `<option value="${escapeHtml(m)}">${escapeHtml(m)}</option>`)
      .join("") + `<option value="${CUSTOM_MODEL_VALUE}">custom…</option>`;
  const pick =
    remembered && (models.includes(remembered) || remembered === CUSTOM_MODEL_VALUE)
      ? remembered
      : backendInfo.defaultModel || models[0] || CUSTOM_MODEL_VALUE;
  sel.value = pick;
  $("agent-model-custom").style.display = sel.value === CUSTOM_MODEL_VALUE ? "" : "none";
}

/// Mirror the selected thread's backend/model into the header selectors so a
/// later selector change is an explicit "switch this thread" intent.
function syncSelectorsToSession(sess) {
  if (!sess) return;
  const backendSel = $("agent-backend");
  const modelSel = $("agent-model");
  if (!backendSel || !modelSel || !state.backends?.length) return;
  const backend = String(sess.backend || "grok").toLowerCase();
  const info = state.backends.find((b) => b.id === backend);
  if (!info) return;
  if (backendSel.value !== backend) {
    backendSel.value = backend;
    populateModelSelect(info);
  }
  const model = sess.model || "";
  if (model && model !== "mock") {
    if ([...modelSel.options].some((o) => o.value === model)) {
      modelSel.value = model;
      $("agent-model-custom").style.display = "none";
    } else {
      modelSel.value = CUSTOM_MODEL_VALUE;
      $("agent-model-custom").value = model;
      $("agent-model-custom").style.display = "";
    }
  }
}

async function loadBackends() {
  const sel = $("agent-backend");
  if (!sel) return;
  try {
    state.backends = await invoke("list_backends");
  } catch (e) {
    console.warn("list_backends failed", e);
    state.backends = [
      { id: "grok", displayName: "Grok", available: true, models: [], defaultModel: "" },
    ];
  }
  sel.innerHTML = state.backends
    .map((b) => {
      const label = b.available ? b.displayName : `${b.displayName} — unavailable`;
      const title = b.available
        ? b.via === "npx"
          ? "runs via npx adapter"
          : b.via || ""
        : b.reason || "not found";
      return `<option value="${escapeHtml(b.id)}" ${b.available ? "" : "disabled"} title="${escapeHtml(title)}">${escapeHtml(label)}</option>`;
    })
    .join("");
  const remembered = localStorage.getItem("bomb.backend");
  const pick = state.backends.find((b) => b.id === remembered && b.available)
    ? remembered
    : (state.backends.find((b) => b.available) || state.backends[0])?.id || "grok";
  sel.value = pick;
  populateModelSelect(state.backends.find((b) => b.id === pick));

  sel.onchange = () => {
    const b = state.backends.find((x) => x.id === sel.value);
    localStorage.setItem("bomb.backend", sel.value);
    populateModelSelect(b);
  };
  $("agent-model").onchange = () => {
    $("agent-model-custom").style.display =
      $("agent-model").value === CUSTOM_MODEL_VALUE ? "" : "none";
    localStorage.setItem(`bomb.model.${sel.value}`, $("agent-model").value);
  };
}

async function startAcp() {
  const newBtn = $("btn-new-session");
  const startBtn = $("btn-start-acp");
  if (state.startingSession) return; // double-click guard
  state.startingSession = true;
  if (newBtn) newBtn.disabled = true;
  if (startBtn) startBtn.disabled = true;
  try {
    const backend = currentBackend();
    // Grok login gate only applies to the grok backend; claude/codex ride
    // their own CLI logins (or env API keys).
    const auth =
      backend === "grok" ? state.auth || (await refreshAuth().catch(() => null)) : null;
    if (auth && !auth.loggedIn) {
      const go = confirm("Not signed in with Grok. Log in now?");
      if (go) {
        await loginWithGrok();
        // The device-code flow finishes in the browser — wait for the poll
        // to land instead of failing instantly.
        const deadline = Date.now() + 120_000;
        while (state.loggingIn && Date.now() < deadline) {
          await new Promise((r) => setTimeout(r, 1000));
        }
        await refreshAuth().catch(() => null);
        if (!state.auth?.loggedIn) throw new Error("Login required before starting a session");
      } else {
        throw new Error("Sign in with Grok first");
      }
    }
    const cwd = $("cwd").value.trim();
    if (!cwd) throw new Error("Set project cwd (absolute path)");
    const model = currentModel();
    const mcpNames = parseCsv($("mcp-attach-session")?.value || "");
    // Typing a server name into the attach box IS the user's approval —
    // the backend still gates unknown/auto servers on its high-risk policy.
    const highRisk = [...mcpNames];
    const opts = {
      mode: "acp",
      backend,
      model,
      planMode: modeOn("plan-mode"),
      alwaysApprove: modeOn("always-approve"),
      isolateWorktree: modeOn("worktree-mode"),
      projectRoot: null,
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
    localStorage.setItem("bomb.backend", backend);
    if (backend !== "grok") {
      const b = (state.backends || []).find((x) => x.id === backend);
      pushEvent(
        `${b?.displayName || backend} · uses ${backend} CLI login${b?.via === "npx" ? " · npx adapter (first launch is slow)" : ""}`,
        "ok",
        "thinking",
        { force: true }
      );
    }
    const res = await invoke("start_session", { cwd, opts });
    appendTranscript(res.id, "system", `session starting · ${backend} · cwd ${cwd}`);
    pushEvent(`ACP session ${shortId(res.id)} starting`, "ok", "thinking", { force: true, milestone: true });
    await refreshSessions();
    // selectSession persists previous presence under the *previous* id
    await selectSession(res.id);
  } catch (e) {
    toastError(e);
  } finally {
    state.startingSession = false;
    if (newBtn) newBtn.disabled = false;
    if (startBtn) startBtn.disabled = false;
  }
}

async function sendPrompt() {
  try {
    const prompt = $("prompt").value;
    if (!prompt.trim()) throw new Error("Empty prompt");
    if (state.selectedSession && turnActive()) {
      pushEvent("turn in progress — wait for it to finish or cancel first", "err", "wait", {
        force: true,
      });
      return;
    }
    // Sending into a still-starting session fails backend-side; don't spin up
    // turn presence for a prompt that can't be delivered yet.
    const selectedSess = state.sessions.find((s) => s.id === state.selectedSession);
    if (selectedSess && String(selectedSess.status || "").toLowerCase().includes("start")) {
      pushEvent("session is still starting — give it a second, then send", "err", "wait", {
        force: true,
      });
      return;
    }
    if (!state.selectedSession) {
      // No thread selected — start one with the current cwd/agent settings.
      await startAcp();
      if (!state.selectedSession) return; // startAcp already surfaced the error
    }
    const sess = state.sessions.find((s) => s.id === state.selectedSession);
    const needsResume =
      sess && (sess.live === false || String(sess.status || "").toLowerCase().includes("saved"));

    appendTranscript(state.selectedSession, "user", prompt);
    $("prompt").value = "";
    endAgentStream(state.selectedSession);
    state.phraseIndex = 0;
    clearBoomTimer(state.selectedSession);
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
    await invoke("send_prompt", {
      id: state.selectedSession,
      prompt,
      backend: currentBackend(),
      model: currentModel(),
      planMode: modeOn("plan-mode"),
      alwaysApprove: modeOn("always-approve"),
    });
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
$("btn-new-project") &&
  ($("btn-new-project").onclick = async () => {
    try {
      const picked = await window.__TAURI__.dialog.open({
        directory: true,
        multiple: false,
        title: "Choose a project folder",
        defaultPath: $("cwd").value || undefined,
      });
      if (picked) {
        setProjectCwd(picked); // registers the project + makes it active
        pushEvent(`project added · ${String(picked).split("/").filter(Boolean).pop()}`, "ok", null, {
          force: true,
        });
      }
    } catch (e) {
      toastError(e);
    }
  });
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
    setProjectCwd(res.path);
    if ($("repo")) $("repo").value = res.path;
    toggleNewFolderPanel(false);
    toggleProjectMenu(false);
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
  // window.open is a silent no-op in the Tauri webview — show the URL so
  // the user can open it themselves if the backend open fails.
  const fallback = () => {
    const u = $("btn-open-login-url").dataset.url;
    if (u) {
      pushEvent(`open manually: ${u}`, "err", "wait", { force: true });
      $("auth-hint").textContent = `Open this URL in your browser: ${u}`;
    }
  };
  try {
    const url = await invoke("open_grok_login_url");
    if (!url) fallback();
  } catch (e) {
    fallback();
    toastError(e);
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
    loadHavenSettings().catch(() => {});
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
    // window.open is a no-op in the Tauri webview — surface the URL instead.
    if (state.devServer?.url) {
      pushEvent(`open manually: ${state.devServer.url}`, "err", "wait", { force: true });
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
$("btn-land-thread") && ($("btn-land-thread").onclick = landThread);
$("btn-sync-thread") && ($("btn-sync-thread").onclick = syncThread);
// Send doubles as Stop while a turn is running.
$("btn-send").onclick = () => {
  if (turnActive() && state.selectedSession) {
    cancelCurrentTurn();
  } else {
    sendPrompt();
  }
};

function updateSendButton() {
  const btn = $("btn-send");
  if (!btn) return;
  const busy = turnActive() && !!state.selectedSession;
  btn.textContent = busy ? "Stop" : "Send";
  btn.classList.toggle("danger", busy);
  btn.classList.toggle("primary", !busy);
}
async function cancelCurrentTurn() {
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
}
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
/** Populate the MCP catalog picker from the backend catalog (single source
 *  of truth for ids, titles, and which servers need credentials). */
async function loadMcpCatalog() {
  try {
    const cat = await invoke("list_mcp_catalog");
    state.mcpCatalog = Array.isArray(cat) ? cat : [];
    const sel = $("mcp-catalog");
    if (sel && state.mcpCatalog.length) {
      sel.innerHTML = state.mcpCatalog
        .map((e) => {
          const creds = e.credentialKeys || e.credential_keys || [];
          const needs = creds.length ? ` (needs ${creds.join(", ")})` : "";
          return `<option value="${escapeHtml(e.id)}">${escapeHtml(e.title || e.id)}${escapeHtml(needs)}</option>`;
        })
        .join("");
    }
  } catch (e) {
    console.warn("mcp catalog load failed", e);
  }
}

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
    renderHavenState(st);
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

// ── Haven settings card ───────────────────────────────────────────────────
function renderHavenState(st) {
  const el = $("haven-state");
  if (!el || !st) return;
  el.textContent = st.connected ? "connected" : st.configured ? "offline" : "not configured";
  el.className = `badge ${st.connected ? "running" : st.configured ? "failed" : "saved"}`;
  el.title = st.message || "";
}

function havenOut(data) {
  const el = $("haven-out");
  el.style.display = "";
  el.textContent = typeof data === "string" ? data : JSON.stringify(data, null, 2);
}

async function loadHavenSettings() {
  try {
    const cfg = await invoke("haven_get_config");
    $("haven-url").value = cfg.base_url || "";
    $("haven-token").value = cfg.auth_token || "";
    $("haven-label").value = cfg.label || "haven";
    $("haven-enabled").checked = !!cfg.enabled;
    $("haven-autoconnect").checked = !!cfg.auto_connect;
    $("haven-insecure").checked = !!cfg.allow_insecure_http;
    const st = await invoke("haven_status").catch(() => null);
    renderHavenState(st);
    if (st?.connected) refreshHavenJobs().catch(() => {});
  } catch (e) {
    console.warn("haven config load failed", e);
  }
}

function havenConfigFromForm() {
  return {
    enabled: $("haven-enabled").checked,
    base_url: $("haven-url").value.trim(),
    auth_token: $("haven-token").value.trim(),
    label: $("haven-label").value.trim() || "haven",
    auto_connect: $("haven-autoconnect").checked,
    allow_insecure_http: $("haven-insecure").checked,
  };
}

$("btn-haven-save").onclick = async () => {
  try {
    // Backend keeps the existing secret when it receives a masked token.
    const st = await invoke("haven_set_config", { config: havenConfigFromForm() });
    renderHavenState(st);
    havenOut(st);
    pushEvent(`Haven saved · ${st.message || "config updated"}`, st.connected ? "ok" : "", null, {
      force: true,
    });
  } catch (e) {
    // Surface save failures in the card itself, not only the timeline.
    const el = $("haven-state");
    if (el) {
      el.textContent = "save failed";
      el.className = "badge failed";
      el.title = String(e);
    }
    havenOut(`Save failed: ${e?.message || e}`);
    toastError(e);
  }
};

$("btn-haven-test").onclick = async () => {
  try {
    const st = await invoke("haven_status");
    renderHavenState(st);
    havenOut(st);
  } catch (e) {
    toastError(e);
  }
};

$("btn-haven-run").onclick = async () => {
  try {
    const name = $("haven-job-name").value.trim();
    const command = $("haven-job-cmd").value.trim();
    if (!name || !command) throw new Error("Job needs a name and a command");
    const cwd = $("haven-job-cwd").value.trim() || null;
    const keepAlive = $("haven-job-keepalive").checked;
    const res = await invoke("haven_start_shell", { name, command, cwd, keepAlive });
    havenOut(res);
    pushEvent(`Haven job started · ${name}${keepAlive ? " (keep-alive)" : ""}`, "ok", "boom", {
      force: true,
      milestone: true,
    });
    $("haven-job-cmd").value = "";
    await refreshHavenJobs();
  } catch (e) {
    toastError(e);
  }
};

/// Render jobs as an interactive list: status, keep-alive, Log / Stop.
async function refreshHavenJobs() {
  const root = $("haven-jobs");
  if (!root) return;
  let jobs;
  try {
    jobs = await invoke("haven_list_jobs");
  } catch (e) {
    root.innerHTML = `<div class="empty-hint">${escapeHtml(String(e))}</div>`;
    return;
  }
  const list = Array.isArray(jobs) ? jobs : jobs?.jobs || [];
  if (!list.length) {
    root.innerHTML = `<div class="empty-hint">No jobs on Haven</div>`;
    return;
  }
  root.innerHTML = list
    .map((j) => {
      const id = j.id || j.job_id || j.name || "";
      const status = String(j.status || j.state || "unknown").toLowerCase();
      const running = /run|alive|active/.test(status);
      return `<div class="haven-job" data-id="${escapeHtml(String(id))}">
        <span class="badge ${running ? "running" : "saved"}">${escapeHtml(status)}</span>
        <span class="haven-job-name">${escapeHtml(j.name || String(id))}</span>
        ${j.keep_alive || j.keepAlive ? '<span class="badge idle">keep-alive</span>' : ""}
        <span class="haven-job-cmd muted">${escapeHtml(String(j.command || "").slice(0, 60))}</span>
        <span class="haven-job-actions">
          <button class="btn ghost haven-log" type="button">Log</button>
          <button class="btn danger ghost haven-stop" type="button">Stop</button>
        </span>
      </div>`;
    })
    .join("");
  root.querySelectorAll(".haven-job").forEach((el) => {
    const id = el.dataset.id;
    el.querySelector(".haven-log").onclick = async () => {
      try {
        havenOut(await invoke("haven_job_log", { id, bytes: 64000 }));
      } catch (e) {
        toastError(e);
      }
    };
    el.querySelector(".haven-stop").onclick = async () => {
      try {
        await invoke("haven_remove_job", { id });
        pushEvent(`Haven job stopped · ${id}`, "ok", null, { force: true });
        await refreshHavenJobs();
      } catch (e) {
        toastError(e);
      }
    };
  });
}

$("btn-haven-jobs").onclick = () => refreshHavenJobs().catch(toastError);

$("btn-haven-files").onclick = async () => {
  try {
    havenOut(await invoke("haven_list_files"));
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

// Explainer toggle + demoted Log accordion.
function wireExplainer() {
  const toggle = $("explain-toggle");
  const applyToggle = () => {
    if (!toggle) return;
    toggle.classList.toggle("on", state.explainerEnabled);
    toggle.textContent = state.explainerEnabled ? "on" : "off";
    toggle.setAttribute("aria-pressed", String(state.explainerEnabled));
  };
  if (toggle) {
    toggle.onclick = async () => {
      const next = !state.explainerEnabled;
      try {
        state.explainerEnabled = await invoke("set_explainer_enabled", { enabled: next });
      } catch (e) {
        toastError(e);
      }
      applyToggle();
      renderExplainFeed();
    };
  }
  // Gear menu: pick which connected provider/model narrates.
  const gear = $("explain-config");
  const row = $("explain-config-row");
  const backendSel = $("explain-backend");
  const modelSel = $("explain-model");
  const fillModels = (backendId, current) => {
    if (!modelSel) return;
    const b = (state.backends || []).find((x) => x.id === backendId);
    const models = b?.models?.length ? b.models : [];
    modelSel.innerHTML = models
      .map((m) => `<option value="${escapeHtml(m)}"${m === current ? " selected" : ""}>${escapeHtml(m)}</option>`)
      .join("");
  };
  const fillProviders = (currentBackend, currentModel) => {
    if (!backendSel) return;
    const avail = (state.backends || []).filter((b) => b.available);
    backendSel.innerHTML = avail
      .map((b) => `<option value="${escapeHtml(b.id)}"${b.id === currentBackend ? " selected" : ""}>${escapeHtml(b.displayName || b.id)}</option>`)
      .join("");
    fillModels(backendSel.value || currentBackend, currentModel);
  };
  if (gear && row) {
    gear.onclick = () => {
      const open = row.style.display === "none";
      if (open) fillProviders(state.explainerBackend || "grok", state.explainerModel || "");
      row.style.display = open ? "" : "none";
    };
  }
  if (backendSel) backendSel.onchange = () => fillModels(backendSel.value, "");
  const apply = $("explain-apply");
  if (apply) {
    apply.onclick = async () => {
      try {
        state.explainerBackend = backendSel?.value || "grok";
        state.explainerModel = modelSel?.value || null;
        await invoke("set_explainer_provider", {
          backend: state.explainerBackend,
          model: state.explainerModel,
        });
        pushEvent(`narrator → ${state.explainerBackend} · ${state.explainerModel || "default"}`, "ok");
        row.style.display = "none";
      } catch (e) {
        toastError(e);
      }
    };
  }

  // Reflect persisted config once it loads (best-effort).
  invoke("get_config")
    .then((cfg) => {
      if (typeof cfg?.explainer_enabled === "boolean") {
        state.explainerEnabled = cfg.explainer_enabled;
      } else if (typeof cfg?.explainerEnabled === "boolean") {
        state.explainerEnabled = cfg.explainerEnabled;
      }
      state.explainerBackend = cfg?.explainer_backend || cfg?.explainerBackend || "grok";
      state.explainerModel = cfg?.explainer_model || cfg?.explainerModel || null;
      const wtDefault = cfg?.worktree_isolation_default ?? cfg?.worktreeIsolationDefault ?? true;
      setMode("worktree-mode", !!wtDefault);
      applyToggle();
      renderExplainFeed();
    })
    .catch(() => applyToggle());

  const logToggle = $("log-toggle");
  const feed = $("event-feed");
  if (logToggle && feed) {
    let open = false;
    logToggle.onclick = () => {
      open = !open;
      feed.style.display = open ? "" : "none";
      $("log-caret").textContent = open ? "▾" : "▸";
      logToggle.setAttribute("aria-expanded", String(open));
    };
  }
}

// Approval cards: delegated listener — innerHTML rebuilds kill per-button handlers.
$("transcript")?.addEventListener("click", async (e) => {
  const btn = e.target.closest?.(".approval-btn");
  if (!btn || btn.disabled) return;
  const actions = btn.closest(".approval-actions");
  const buttons = actions ? [...actions.querySelectorAll("button")] : [btn];
  buttons.forEach((b) => (b.disabled = true));
  try {
    await invoke("respond_approval", {
      id: btn.dataset.sid,
      requestId: btn.dataset.requestId,
      optionId: btn.dataset.optionId || null,
    });
    // Card collapses via the approval_resolved event.
  } catch (err) {
    pushEvent(`approval failed: ${err?.message || err}`, "err", "error", { force: true });
    buttons.forEach((b) => (b.disabled = false));
  }
});

// Elapsed + stall clock
setInterval(() => {
  if (turnActive() || state.turn.phase === "done") updateBombChrome();
  if (turnActive()) startPhraseCycle();
  else stopPhraseCycle();
  // Watchdog: presence must never disagree with the thread for long. If the
  // dock says a turn is running but the session has been idle/failed with no
  // new signal for 20s, the turn desynced (missed event, rejected send) —
  // reset instead of showing "Quiet" forever.
  if (turnActive() && state.selectedSession) {
    const sess = state.sessions.find((s) => s.id === state.selectedSession);
    const st = String(sess?.status || "").toLowerCase();
    const settled =
      st.includes("idle") || st.includes("complete") || st.includes("fail") || st.includes("cancel");
    const lastSignal = state.turn.lastSignalAt || state.turn.startedAt || 0;
    if (settled && lastSignal && Date.now() - lastSignal > 20_000) {
      noteTurn("idle", {}, state.selectedSession);
      pushEvent("turn indicator desynced from thread status — reset", "", null, { force: true });
    }
  }
}, 1000);

async function boot() {
  // Every boot step is independent — one failure must not take down the rest
  // (a failed listen() used to die as an unhandled rejection and nothing
  // loaded; a failed refreshStatus skipped session loading entirely).
  try {
    if (hasTauri() && window.__TAURI__.event) {
      await window.__TAURI__.event.listen("control-event", (e) => handleControlEvent(e.payload));
    } else {
      setStatus("error", "Not inside Tauri — use the .app");
      setBombMood($("status-bomb"), "error");
    }
  } catch (e) {
    toastError(e);
  }
  await refreshStatus().catch(toastError);
  await loadBackends().catch(toastError);
  await loadMcpCatalog().catch(() => {});
  await loadProjects();
  await refreshSessions().catch(toastError);
  await refreshDevStatus().catch(() => {});
  try {
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

$("cwd")?.addEventListener("input", () => {
  state.cwdDirty = !!$("cwd").value.trim();
});

wireModeButtons();
wireProjectChip();
wireAgentTalk();
wireExplainer();
boot();
