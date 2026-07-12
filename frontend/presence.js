/**
 * TurnPresence — single source of truth for turn status + bomb mood.
 * Pure helpers; app.js owns the mutable store and DOM.
 *
 * Plan: docs/plan/status_and_bomb_animation_ux_plan.md
 */
(function (global) {
  "use strict";

  const STALL_MS = 25000;
  const FLAVOR_DELAY_MS = 8000;
  const BOOM_HOLD_MS = 1000;
  const MOOD_DEBOUNCE_MS = 120;

  const PHASE_RANK = {
    idle: 0,
    send: 1,
    think: 2,
    tools: 3,
    reply: 4,
    wait: 5,
    done: 6,
    error: 6,
  };

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

  function emptyPresence() {
    return {
      phase: "idle",
      startedAt: null,
      lastSignalAt: null,
      promptChars: 0,
      thoughtChars: 0,
      replyChars: 0,
      contextTokens: null,
      toolCount: 0,
      toolsActive: 0,
      lastTool: null,
      lastToolStatus: null,
      preview: "",
      thoughtPreview: "",
      note: "",
      stagesSeen: { send: false, think: false, tools: false, reply: false },
      transition: null,
      _lastMood: "idle",
      _lastMoodAt: 0,
    };
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
    const t = String(text || "")
      .replace(/\s+/g, " ")
      .trim();
    if (!t) return "";
    return t.length <= n ? t : `…${t.slice(-n)}`;
  }

  function turnActive(p) {
    return ["send", "think", "tools", "reply", "wait"].includes(p.phase);
  }

  function deriveStall(p, now) {
    if (p.phase === "wait") return "awaiting_user";
    if (!turnActive(p) || !p.lastSignalAt) return null;
    const quiet = now - p.lastSignalAt;
    if (quiet < STALL_MS) return null;
    const st = String(p.lastToolStatus || "").toLowerCase();
    const toolBusy =
      p.toolsActive > 0 ||
      (p.lastTool &&
        st &&
        !st.includes("complete") &&
        !st.includes("done") &&
        !st.includes("fail") &&
        !st.includes("success"));
    if (toolBusy) return "tool_hang";
    if (!p.replyChars && !p.thoughtChars && !p.toolCount) return "no_first_signal";
    return "stream_gap";
  }

  /**
   * Apply a phase signal + patch. Mutates and returns presence.
   */
  function applySignal(p, phase, patch, now) {
    now = now || Date.now();
    if (!p.startedAt && phase !== "idle" && phase !== "done") {
      p.startedAt = now;
    }

    const cur = PHASE_RANK[p.phase] ?? 0;
    const next = PHASE_RANK[phase] ?? 0;
    let phaseChanged = false;

    // Apply numeric/status patches first so sticky-tools sees updated toolsActive
    if (patch && typeof patch === "object") {
      for (const [k, v] of Object.entries(patch)) {
        if (k === "stagesSeen" || k === "transition" || k.startsWith("_")) continue;
        // Map legacy streamChars → replyChars
        if (k === "streamChars") {
          p.replyChars = v;
          continue;
        }
        if (
          k in p ||
          [
            "promptChars",
            "thoughtChars",
            "replyChars",
            "contextTokens",
            "toolCount",
            "toolsActive",
            "lastTool",
            "lastToolStatus",
            "preview",
            "thoughtPreview",
            "note",
            "startedAt",
            "lastSignalAt",
          ].includes(k)
        ) {
          p[k] = v;
        }
      }
    }

    // Sticky tools: while tools are in-flight, don't drop to reply
    // unless phase is wait/error/done/idle.
    const stickyTools =
      (p.toolsActive || 0) > 0 && phase === "reply" && p.phase === "tools";

    if (phase === "wait" || phase === "error" || phase === "done" || phase === "idle") {
      if (p.phase !== phase) phaseChanged = true;
      p.phase = phase;
    } else if (stickyTools) {
      // stay on tools; patches already applied
    } else if (next >= cur || p.phase === "wait") {
      if (p.phase !== phase) phaseChanged = true;
      p.phase = phase;
    }

    p.lastSignalAt = now;

    // Stages actually seen this turn
    if (p.phase === "send" || phase === "send") p.stagesSeen.send = true;
    if (p.phase === "think" || phase === "think" || p.thoughtChars > 0) p.stagesSeen.think = true;
    if (p.phase === "tools" || phase === "tools" || p.toolCount > 0) p.stagesSeen.tools = true;
    if (p.phase === "reply" || phase === "reply" || p.replyChars > 0) p.stagesSeen.reply = true;

    if (phase === "idle") {
      return emptyPresence();
    }

    if (phaseChanged) {
      p.transition = `enter_${p.phase}`;
    }

    return p;
  }

  function markToolStart(p, toolName, now) {
    return applySignal(
      p,
      "tools",
      {
        toolCount: (p.toolCount || 0) + 1,
        toolsActive: (p.toolsActive || 0) + 1,
        lastTool: toolName,
        lastToolStatus: "running",
        note: toolName,
      },
      now
    );
  }

  function markToolDone(p, toolName, status, now) {
    const active = Math.max(0, (p.toolsActive || 1) - 1);
    const st = String(status || "completed").toLowerCase();
    const nextPhase = active > 0 ? "tools" : p.replyChars > 0 ? "reply" : "tools";
    return applySignal(
      p,
      nextPhase,
      {
        toolsActive: active,
        lastTool: toolName || p.lastTool,
        lastToolStatus: status || "completed",
        note: toolName || p.note,
      },
      now
    );
  }

  function pickFlavor(phase, phraseIndex) {
    const list = BOMB_FLAVOR[phase] || BOMB_FLAVOR.idle;
    return list[phraseIndex % list.length];
  }

  function moodForPhase(phase, stall) {
    if (stall && phase !== "wait") {
      // Plan: no panic shake — stay calm thinking/stream glow
      return phase === "tools" ? "tooling" : phase === "reply" ? "stream" : "thinking";
    }
    const map = {
      idle: "idle",
      send: "thinking",
      think: "thinking",
      tools: "tooling",
      reply: "stream",
      wait: "wait",
      done: "boom",
      error: "error",
    };
    return map[phase] || "idle";
  }

  /**
   * Debounce mood class changes (except terminal phases).
   */
  function resolveMood(p, now) {
    now = now || Date.now();
    const stall = deriveStall(p, now);
    const desired = moodForPhase(p.phase, stall);
    const terminal = p.phase === "done" || p.phase === "error" || p.phase === "wait";
    if (terminal || desired === p._lastMood) {
      p._lastMood = desired;
      p._lastMoodAt = now;
      return desired;
    }
    if (now - (p._lastMoodAt || 0) < MOOD_DEBOUNCE_MS) {
      return p._lastMood || desired;
    }
    p._lastMood = desired;
    p._lastMoodAt = now;
    return desired;
  }

  function stageClass(stage, p) {
    const order = ["send", "think", "tools", "reply", "done"];
    // wait: hold last real stage (tools if tools were seen, else think/send)
    let phase = p.phase;
    if (p.phase === "wait") {
      if (p.stagesSeen.tools) phase = "tools";
      else if (p.stagesSeen.think) phase = "think";
      else if (p.stagesSeen.send) phase = "send";
      else phase = "think";
    } else if (p.phase === "error") {
      phase = "done";
    }
    const pi = order.indexOf(phase);
    const si = order.indexOf(stage);
    if (si < 0) return "";

    // Only light stages that occurred (except active current)
    const seen =
      stage === "done"
        ? phase === "done" || p.phase === "error"
        : !!p.stagesSeen[stage] || si === pi;

    if (p.phase === "error" && stage === "done") return "error";
    if (si < pi && seen) return "done";
    if (si === pi) return phase === "done" && p.phase !== "wait" ? "done" : "active";
    if (si < pi && !seen) return ""; // skipped
    return "";
  }

  function meterMode(p, stall) {
    if (p.phase === "done") return "complete";
    if (p.phase === "error") return "error";
    if (p.phase === "idle") return "idle";
    if (stall) return "stall";
    if (p.phase === "reply" && p.replyChars > 0) return "progress";
    if (p.phase === "tools" && p.toolCount > 0) return "tools";
    return "indeterminate";
  }

  /** Soft progress 0–1 from reply chars (asymptotic). */
  function meterProgress(p) {
    if (p.phase === "done") return 1;
    if (p.phase === "error") return 1;
    if (p.replyChars > 0) {
      // 1 - e^(-chars/800) roughly 0→0.7 over a short reply
      return Math.min(0.92, 1 - Math.exp(-p.replyChars / 800));
    }
    if (p.toolCount > 0) {
      return Math.min(0.85, 0.15 + p.toolCount * 0.12);
    }
    return 0;
  }

  /**
   * Single formatter all surfaces use.
   */
  function formatPresence(p, opts) {
    opts = opts || {};
    const now = opts.now || Date.now();
    const phraseIndex = opts.phraseIndex || 0;
    const stall = deriveStall(p, now);
    const active = turnActive(p);
    const show =
      active || p.phase === "done" || p.phase === "error";
    const elapsed = p.startedAt ? formatElapsed(now - p.startedAt) : "";
    const quietMs = p.lastSignalAt ? now - p.lastSignalAt : 0;
    const mood = resolveMood(p, now);
    const showFlavor =
      show &&
      active &&
      !stall &&
      p.startedAt &&
      now - p.startedAt >= FLAVOR_DELAY_MS;

    let title;
    if (stall === "awaiting_user") title = "Needs you";
    else if (stall === "tool_hang") title = "Quiet · tool";
    else if (stall === "no_first_signal") title = "Quiet";
    else if (stall === "stream_gap") title = "Quiet";
    else if (p.phase === "tools" && p.lastTool) title = `Running · ${p.lastTool}`;
    else if (p.phase === "reply") title = "Writing";
    else if (p.phase === "think") title = "Thinking";
    else if (p.phase === "send") title = "Sent";
    else if (p.phase === "done") title = "Done";
    else if (p.phase === "error") title = "Failed";
    else if (p.phase === "wait") title = "Needs you";
    else title = "Idle";

    const bits = [];
    if (p.promptChars) bits.push(`prompt ${formatCount(p.promptChars)} chars`);
    if (p.phase === "think" && !p.replyChars && !p.toolCount && !stall) {
      bits.push("waiting for first token or tool");
    }
    if (p.thoughtChars) bits.push(`${formatCount(p.thoughtChars)} thought chars`);
    if (p.replyChars) bits.push(`${formatCount(p.replyChars)} reply chars`);
    if (p.contextTokens != null) bits.push(`ctx ${formatCount(p.contextTokens)}`);
    if (p.toolCount) {
      bits.push(
        `${p.toolCount} tool${p.toolCount === 1 ? "" : "s"}${
          p.lastTool ? ` · last ${p.lastTool}` : ""
        }${p.lastToolStatus ? ` (${p.lastToolStatus})` : ""}`
      );
    }
    if (p.phase === "wait") bits.push(p.note || "approval required");
    if (p.phase === "error") bits.push(p.note || "see timeline");
    if (p.phase === "done") bits.push(p.note || "ready for next message");
    if (stall && stall !== "awaiting_user") {
      bits.push(`no new signal for ${formatElapsed(quietMs)}`);
      if (stall === "tool_hang" && p.lastTool) {
        bits.push(`waiting on ${p.lastTool}`);
      } else if (stall === "no_first_signal") {
        bits.push("waiting on agent stream");
      }
    }

    const subtitle = bits.join(" · ") || (show ? "Working…" : "No active turn");
    const preview =
      p.phase === "think" && p.thoughtPreview
        ? p.thoughtPreview
        : p.preview || p.thoughtPreview || "";

    return {
      show,
      active,
      phase: p.phase,
      title,
      elapsed,
      subtitle,
      preview,
      flavor: showFlavor ? pickFlavor(p.phase, phraseIndex) : "",
      mood,
      stall,
      stalled: !!stall && stall !== "awaiting_user",
      meterMode: meterMode(p, stall),
      meterProgress: meterProgress(p),
      stagesSeen: { ...p.stagesSeen },
      transition: p.transition,
      tierDock: 2,
      tierSatellite: 0,
      lastTool: p.lastTool,
      lastToolStatus: p.lastToolStatus,
    };
  }

  function consumeTransition(p) {
    const t = p.transition;
    p.transition = null;
    return t;
  }

  global.BombPresence = {
    STALL_MS,
    FLAVOR_DELAY_MS,
    BOOM_HOLD_MS,
    emptyPresence,
    formatElapsed,
    formatCount,
    clipPreview,
    turnActive,
    deriveStall,
    applySignal,
    markToolStart,
    markToolDone,
    pickFlavor,
    moodForPhase,
    resolveMood,
    stageClass,
    formatPresence,
    consumeTransition,
    BOMB_FLAVOR,
  };
})(typeof window !== "undefined" ? window : globalThis);
