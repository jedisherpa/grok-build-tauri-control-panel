/**
 * Lightweight presence unit tests — run: node frontend/presence.test.mjs
 */
import { readFileSync } from "fs";
import { createContext, runInContext } from "vm";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const src = readFileSync(join(__dirname, "presence.js"), "utf8");
const sandbox = { window: {}, globalThis: {} };
sandbox.globalThis = sandbox;
runInContext(src, createContext(sandbox));
const P = sandbox.window.BombPresence;

function assert(cond, msg) {
  if (!cond) throw new Error(msg || "assert failed");
}

let p = P.emptyPresence();
p = P.applySignal(p, "send", { promptChars: 10 });
assert(p.phase === "send", "send");
p = P.applySignal(p, "think", {});
assert(p.stagesSeen.think, "think stage");
p = P.markToolStart(p, "read_file");
assert(p.phase === "tools" && p.toolsActive === 1, "tool start");
p = P.applySignal(p, "reply", { replyChars: 5 });
assert(p.phase === "tools", "sticky tools");
p = P.markToolDone(p, "read_file", "completed");
assert(p.toolsActive === 0 && p.phase === "reply", "reply after tool");

p = P.markToolStart(p, "x");
p = P.markToolDone(p, "x", "failed");
assert(p.toolsActive === 0, "failed terminal");

p.lastSignalAt = Date.now() - 30000;
assert(P.deriveStall(p, Date.now()) === "stream_gap", "stall gap");
assert(P.formatPresence(p).mood !== "running", "no panic mood");

p = P.applySignal(p, "done", {});
assert(P.formatPresence(p).mood === "boom", "boom");

let q = P.emptyPresence();
q = P.applySignal(q, "send", {});
q = P.applySignal(q, "think", {});
q = P.applySignal(q, "reply", { replyChars: 20 });
assert(P.stageClass("tools", q) !== "active", "no fake tools stage");
assert(P.stageClass("reply", q) === "active", "reply active");

// Output volume and tool counts cannot measure remaining task work.
for (const chars of [1, 800, 1000000]) {
  const active = P.emptyPresence(); active.phase = 'reply'; active.replyChars = chars; active.toolCount = 500;
  const view = P.formatPresence(active);
  assert(view.meterMode === 'indeterminate', 'reply activity is indeterminate');
  assert(view.meterProgress === null, 'no fabricated percentage');
  assert(view.completionLabel.includes('unknown'), 'unknown completion is explicit');
}
for (const phase of ['done','error','tools']) {
  const active = P.emptyPresence(); active.phase = phase;
  assert(P.formatPresence(active).meterProgress === null, 'terminal/tool state cannot prove task completion');
}

// An Idle registry signal is not the session/prompt response boundary.
let uncertain = P.markToolStart(P.emptyPresence(), "read", 1000);
uncertain = P.idleStatus(uncertain, 1100);
assert(uncertain.phase === "tools" && uncertain.toolsActive === 1, "Idle retains open tools");
assert(!P.normallyFinished(uncertain), "Idle cannot supply a completion receipt");
assert(P.formatPresence(uncertain, { now: 1200 }).title === "Completion unconfirmed", "missing boundary is visible");
let waiting = P.applySignal(P.emptyPresence(), "wait", { note: "approval pending" }, 1000);
waiting = P.idleStatus(waiting, 1100);
assert(waiting.phase === "wait" && waiting.note === "approval pending", "Idle retains pending approval phase");
assert(P.formatPresence(waiting, { now: 1200 }).title === "Needs you", "approval remains prominent");

for (const reason of ["end_turn", "mock"]) {
  let ended = P.finishPrompt(P.emptyPresence(), reason, 1000);
  assert(ended.phase === "done" && P.normallyFinished(ended), "typed normal response ends turn");
  assert(P.formatPresence(ended, { now: 1100 }).title === "Turn ended", "response outcome does not claim task acceptance");
  ended = P.applySignal(ended, "idle", {}, 2000);
  assert(P.normallyFinished(ended), "animation settling preserves exact receipt");
  assert(P.closeCompletedSession(ended, 2100), "completed-turn session cleanup is distinguishable");
  assert(ended.phase !== "error" && ended.sessionClosed, "cleanup is not response failure");
  assert(P.formatPresence(ended, { now: 2200 }).title.includes("session closed"), "closed session is labelled separately");
  ended = P.applySignal(ended, "run", {}, 2300);
  assert(ended.phase === "think" && !P.normallyFinished(ended) && !ended.sessionClosed, "new running signal invalidates receipt");
  assert(!P.closeCompletedSession(ended, 2400), "old receipt cannot mask later cancellation");
}
for (const reason of ["max_tokens", "cancelled", "refusal", "missing_stop_reason", null]) {
  const stopped = P.finishPrompt(P.emptyPresence(), reason, 1000);
  assert(stopped.phase !== "done" && !P.normallyFinished(stopped), "non-normal stop cannot celebrate completion");
  assert(!P.closeCompletedSession(stopped, 1100), "incomplete response cannot qualify session cleanup");
}
for (const phase of ["send", "think", "tools", "reply", "error"]) {
  const restarted = P.applySignal(P.finishPrompt(P.emptyPresence(), "end_turn", 1000), phase, {}, 1100);
  assert(!P.normallyFinished(restarted), "new activity/error invalidates exact receipt");
  assert(restarted.phase === phase, "later signal is not blocked by terminal phase rank");
}
waiting = P.finishPrompt(waiting, "end_turn", 1300);
assert(waiting.phase === "wait" && !P.closeCompletedSession(waiting), "turn ending does not erase pending permission");
assert(P.finishPrompt(P.applySignal(P.emptyPresence(), "wait", {}, 1000), "max_tokens", 1100).phase === "wait", "truncation does not hide a pending permission");

// Exercise the actual app handler with native state stores; no UI or provider.
const appSource = readFileSync(join(__dirname, "app.js"), "utf8");
const start = appSource.indexOf("function handleControlEvent(ev) {");
const handler = appSource.slice(start, appSource.indexOf("\n}\n", start) + 2);
const nativePresence = new Map([["a", P.markToolStart(P.emptyPresence(), "read", 1000)]]);
const open = new Set(["t1"]), timers = [];
const native = {
  P, state: { selectedSession: "a", ready: true, boomTimers: new Map() },
  Date, setTimeout: callback => { timers.push(callback); return timers.length; },
  presenceFor: sid => nativePresence.get(sid),
  commitPresence: (sid, value) => nativePresence.set(sid, value),
  openToolsFor: () => open,
  endTurnPresence: (sid, phase, note) => { open.clear(); nativePresence.set(sid, P.applySignal(nativePresence.get(sid), phase, { note, toolsActive: 0 })); },
  noteTurn: (phase, patch, sid) => nativePresence.set(sid, P.applySignal(nativePresence.get(sid), phase, patch)),
  endAgentStream() {}, clearBoomTimer() {}, appendTranscript() {}, pushEvent() {}, refreshSessions() {}, talkNote() {}, sweepToolsForSession() {},
  nowIso: () => new Date().toISOString(), shortId: sid => sid,
};
runInContext(handler, createContext(native));
native.handleControlEvent({ type: "session_status_changed", session_id: "a", status: "Idle" });
assert(open.has("t1") && nativePresence.get("a").completionUnconfirmed, "actual Idle handler preserves native tools");
native.handleControlEvent({ type: "prompt_finished", session_id: "a", stop_reason: "end_turn" });
assert(open.size === 0 && P.normallyFinished(nativePresence.get("a")), "actual typed boundary ends response");
timers.pop()();
assert(P.normallyFinished(nativePresence.get("a")) && nativePresence.get("a").phase === "idle", "background animation preserves receipt");
native.handleControlEvent({ type: "raw", session_id: "a", payload: { channel: "usage", totalTokens: 100 } });
assert(P.normallyFinished(nativePresence.get("a")), "late usage telemetry is not a new turn");
native.handleControlEvent({ type: "session_cancelled", session_id: "a" });
assert(nativePresence.get("a").sessionClosed && nativePresence.get("a").phase !== "error", "actual native cleanup preserves response outcome");
native.handleControlEvent({ type: "session_status_changed", session_id: "a", status: "Running" });
native.handleControlEvent({ type: "session_cancelled", session_id: "a" });
assert(nativePresence.get("a").phase === "error" && !P.normallyFinished(nativePresence.get("a")), "new turn cancellation is never masked");
nativePresence.set("a", P.finishPrompt(P.emptyPresence(), "end_turn", Date.now()));
native.handleControlEvent({ type: "error", session_id: "a", message: "later provider fault" });
assert(nativePresence.get("a").phase === "error" && !P.normallyFinished(nativePresence.get("a")), "later provider error is never masked by completion");
console.log("presence.test.mjs: all passed");
