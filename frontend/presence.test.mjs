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
console.log("presence.test.mjs: all passed");
