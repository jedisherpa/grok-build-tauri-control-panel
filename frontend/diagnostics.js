/**
 * BombDiagnostics — pure helpers for user-facing error text and composer gating.
 * Mirrors crates/grok_events/src/diagnostics.rs (defence in depth: the host
 * sanitizes too, but older events / other channels must not leak either).
 */
(function (global) {
  "use strict";

  const ANSI_OSC = /\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;
  const ANSI_CSI = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-Z\\-_]/g;
  const TEAM = /(\/team\/)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;
  const XAI = /xai-(?:\.{2,}|…)?[A-Za-z0-9_\-]{2,}/g;
  const SK = /\bsk-[A-Za-z0-9_\-]{8,}/g;
  const BEARER = /(bearer\s+)[A-Za-z0-9._~+/\-]{8,}=*/gi;

  function stripAnsi(s) {
    return String(s ?? "").replace(ANSI_OSC, "").replace(ANSI_CSI, "").replace(/\x1b/g, "");
  }

  function redactSecrets(s) {
    return String(s ?? "")
      .replace(TEAM, "$1[team]")
      .replace(XAI, "xai-[redacted]")
      .replace(SK, "sk-[redacted]")
      .replace(BEARER, "$1[redacted]");
  }

  function sanitize(s) {
    return redactSecrets(stripAnsi(s));
  }

  /**
   * Whether the composer may send into the selected thread, and what to say
   * when it may not. `status` is the thread's registry status string.
   */
  function composerGate(sessionId, status) {
    if (!sessionId) return { canSend: true, reason: "" };
    const st = String(status || "").toLowerCase();
    if (st.includes("start")) {
      return {
        canSend: false,
        reason: "Session is still starting — your message stays here until it's ready.",
      };
    }
    if (st.includes("fail")) {
      return {
        canSend: false,
        reason: "This thread failed to start (see the error above). Fix sign-in, then start a new thread — your message is kept.",
      };
    }
    return { canSend: true, reason: "" };
  }

  global.BombDiagnostics = { stripAnsi, redactSecrets, sanitize, composerGate };
})(typeof window !== "undefined" ? window : globalThis);
