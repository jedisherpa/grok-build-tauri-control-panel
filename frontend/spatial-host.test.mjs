import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const Host = require("./spatial-host.js");
const source = (overrides = {}) => ({ sessions: [{ id: "a", label: "Task", backend: "grok", live: true, status: "idle" }], selectedSession: "a", turn: { phase: "done", lastSignalAt: 900 }, openToolsBySession: new Map(), transcriptBySession: new Map(), ...overrides });

test("initial Idle and old UI celebration never report native completion", () => {
  const rows = Host.projectSessions(source(), Host.createTelemetry(), 1000);
  assert.equal(rows[0].phase, "idle");
  assert.equal(rows[0].toolsActive, null);
  assert.equal(rows[0].pendingApprovals, null);
  assert.equal(Host.projectSessions(source({ openToolsBySession: new Map([["a", new Set()]]) }), Host.createTelemetry(), 1000)[0].toolsActive, null);
});
test("timeout Idle stays uncertain; typed PromptFinished is the fresh boundary", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "session_status_changed", session_id: "a", status: "running" }, 700);
  ledger.observe({ type: "tool_call", session_id: "a", event: { id: "t1", status: "running" } }, 800);
  ledger.observe({ type: "session_status_changed", session_id: "a", status: "idle" }, 900);
  assert.equal(Host.projectSessions(source(), ledger, 1000)[0].phase, "unknown");
  assert.equal(ledger.sessions.get("a").tools.has("t1"), true);
  assert.equal(Host.projectSessions(source({ openToolsBySession: new Map([["a", new Set()]]) }), ledger, 1000)[0].toolsActive, null);
  ledger.observe({ type: "prompt_finished", session_id: "a", stop_reason: "end_turn", at: 1000 }, 1000);
  assert.equal(Host.projectSessions(source(), ledger, 1100)[0].phase, "done");
  assert.equal(Host.projectSessions(source(), ledger, 1100)[0].toolsActive, 0);
  assert.equal(Host.projectSessions(source(), ledger, 3000)[0].phase, "idle");
});
test("missing stop reason is not completion and terminal history is saved", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "prompt_finished", session_id: "a", stop_reason: "missing_stop_reason" }, 1000);
  assert.equal(Host.projectSessions(source(), ledger, 1000)[0].phase, "unknown");
  const saved = source({ sessions: [{ id: "a", live: false, status: "running" }] });
  const row = Host.projectSessions(saved, ledger, 1000)[0];
  assert.equal(row.live, false); assert.equal(row.savedOnly, true); assert.equal(row.phase, "idle");
  assert.equal(row.pendingApprovals, 0);
  assert.equal(Host.projectSessions(source({ sessions: [{ id: "a", status: "running" }] }), ledger, 1000)[0].live, null);
});
test("tool IDs and pending permission IDs are observed without resolving anything", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "session_created", session_id: "a" }, 1000);
  ledger.observe({ type: "tool_call", session_id: "a", event: { id: "t1", status: "running" } }, 1100);
  ledger.observe({ type: "tool_call", session_id: "a", event: { id: "t1", status: "running" } }, 1150);
  ledger.observe({ type: "approval_required", session_id: "a", request_id: "p1", auto_approved: false }, 1200);
  const row = Host.projectSessions(source(), ledger, 1300)[0];
  assert.equal(row.toolsActive, 1); assert.equal(row.pendingApprovals, 1); assert.equal(row.phase, "wait");
  ledger.observe({ type: "approval_resolved", session_id: "a", request_id: "p1", cancelled: true }, 1400);
  assert.equal(Host.projectSessions(source(), ledger, 1450)[0].pendingApprovals, 0);
  ledger.observe({ type: "tool_call", session_id: "a", event: { id: "t1", status: "completed" } }, 1500);
  assert.equal(Host.projectSessions(source(), ledger, 1600)[0].toolsActive, 0);
});
test("truncated, cancelled and unknown prompt responses are not normal turn completion", () => {
  for (const reason of ["max_tokens", "cancelled", "refusal", "missing_stop_reason", "unknown"]) {
    const ledger = Host.createTelemetry();
    ledger.observe({ type: "prompt_finished", session_id: "a", stop_reason: reason }, 1000);
    const row = Host.projectSessions(source(), ledger, 1100)[0];
    assert.notEqual(row.phase, "done"); assert.equal(row.toolsActive, null);
  }
});
test("Idle and normal turn ending do not erase unresolved permission IDs", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "approval_required", session_id: "a", request_id: "p1" }, 1000);
  ledger.observe({ type: "session_status_changed", session_id: "a", status: "idle" }, 1100);
  ledger.observe({ type: "prompt_finished", session_id: "a", stop_reason: "end_turn" }, 1200);
  assert.equal(Host.projectSessions(source(), ledger, 1250)[0].pendingApprovals, 1);
  ledger.observe({ type: "session_cancelled", session_id: "a" }, 1300);
  assert.equal(Host.projectSessions(source(), ledger, 1400)[0].pendingApprovals, 0);
});
test("known pending requests remain visible while other counts lack coverage", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "session_created", session_id: "a" }, 900);
  ledger.observe({ type: "approval_required", session_id: "a", request_id: "p1" }, 1000);
  const rows = Host.projectSessions(source({ sessions: [{ id: "a", live: true }, { id: "b", live: true, status: "idle" }] }), ledger, 1100);
  assert.deepEqual(Host.summarizeApprovals(rows), { pendingApprovals: 1, approvalCoverageKnown: false, coverageKnown: false });
  assert.equal(rows[0].approvalCoverageKnown, true);
  assert.equal(rows[1].pendingApprovals, null);
  ledger.observe({ type: "session_created", session_id: "b" }, 1200);
  assert.equal(Host.summarizeApprovals(Host.projectSessions(source({ sessions: [{ id: "a", live: true }, { id: "b", live: true }] }), ledger, 1300)).approvalCoverageKnown, true);
  const partial = Host.createTelemetry();
  partial.observe({ type: "approval_required", session_id: "a", request_id: "p1" }, 1000);
  assert.deepEqual(Host.summarizeApprovals(Host.projectSessions(source(), partial, 1100)), { pendingApprovals: 1, approvalCoverageKnown: false, coverageKnown: false });
});
test("resolved and cancelled permissions cannot return from stale native transcript metadata", () => {
  const approval = id => ({ role: "approval", meta: { requestId: id, options: [{ optionId: "yes" }], resolved: false } });
  const native = source({ transcriptBySession: new Map([["a", [approval("p1"), approval("p2")]]]) });
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "session_created", session_id: "a" }, 800);
  ledger.observe({ type: "approval_required", session_id: "a", request_id: "p1" }, 900);
  ledger.observe({ type: "approval_resolved", session_id: "a", request_id: "p1" }, 1000);
  assert.equal(Host.projectSessions(native, ledger, 1100)[0].pendingApprovals, 1);
  ledger.observe({ type: "session_cancelled", session_id: "a" }, 1200);
  assert.equal(Host.projectSessions(native, ledger, 1300)[0].pendingApprovals, 0);
  ledger.observe({ type: "approval_required", session_id: "a", request_id: "p3" }, 1400);
  assert.equal(Host.projectSessions(native, ledger, 1500)[0].pendingApprovals, 1);
});
test("session close after exact normal response preserves the turn outcome, not task acceptance", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "prompt_finished", session_id: "a", stop_reason: "end_turn" }, 1000);
  ledger.observe({ type: "session_status_changed", session_id: "a", status: "cancelled" }, 1100);
  ledger.observe({ type: "session_cancelled", session_id: "a" }, 1200);
  const row = Host.projectSessions(source(), ledger, 3000)[0];
  assert.equal(row.phase, "idle"); assert.equal(row.sessionClosed, true); assert.equal(row.turnEndedAt, 1000);
  assert.equal(row.toolsActive, 0);
  ledger.observe({ type: "agent_message", session_id: "a", text: "turn complete" }, 3100);
  assert.equal(Host.projectSessions(source(), ledger, 3200)[0].sessionClosed, true);
  ledger.observe({ type: "error", session_id: "a", message: "later fault" }, 3300);
  assert.equal(Host.projectSessions(source(), ledger, 3400)[0].phase, "error");
  assert.equal(Host.projectSessions(source(), ledger, 3400)[0].turnEndedAt, null);
});
test("new local send or native activity prevents an old completion from masking cancellation", () => {
  const ledger = Host.createTelemetry();
  ledger.observe({ type: "prompt_finished", session_id: "a", stop_reason: "mock" }, 1000);
  const restarted = source({ turn: { phase: "send", lastSignalAt: 1100, completedAt: null } });
  assert.equal(Host.projectSessions(restarted, ledger, 1200)[0].phase, "send");
  assert.equal(Host.projectSessions(restarted, ledger, 1200)[0].turnEndedAt, null);
  ledger.observe({ type: "session_status_changed", session_id: "a", status: "running" }, 1300);
  ledger.observe({ type: "session_cancelled", session_id: "a" }, 1400);
  const row = Host.projectSessions(source(), ledger, 1500)[0];
  assert.equal(row.phase, "error"); assert.equal(row.sessionClosed, false); assert.equal(row.turnEndedAt, null);
});
test("background sessions use their own presence and no persisted updatedAt signal", () => {
  const state = source({ sessions: [{ id: "a", live: true, status: "running" }, { id: "b", live: true, status: "running", updatedAt: "2099-01-01" }], turn: { phase: "reply", lastSignalAt: 950 }, presenceBySession: new Map([["b", { phase: "think", lastSignalAt: 600 }]]) });
  const rows = Host.projectSessions(state, Host.createTelemetry(), 1000);
  assert.equal(rows[0].lastSignalAt, 950); assert.equal(rows[1].lastSignalAt, 600);
  state.presenceBySession.clear();
  assert.equal(Host.projectSessions(state, Host.createTelemetry(), 1000)[1].lastSignalAt, null);
});
test("Joe invalidation, wrong thread and missing authority clear geometric activations", () => {
  const a = { placement: { position8: [1, 0, 0, 0, 0, 0, 0, 0], root_id: "e8-root:1" }, source_mapping_asserted: true };
  const result = { schema: "bomb-code/joe-result/v1", status: "grounded-model-proposal", threadId: "a", authority: { toolsDispatched: false, approvalsGranted: false, memoryCommitted: false }, interpretation: { binding: { readings: [{ e8_activations: [a] }] } } };
  const detail = { schema: "bomb-code/joe-visual-state/v1", status: result.status, result };
  assert.equal(Host.activationsOf(detail, source())[0], a);
  assert.deepEqual(Host.activationsOf({ ...detail, status: "invalidated" }, source()), []);
  assert.deepEqual(Host.activationsOf(detail, { selectedSession: "b" }), []);
  assert.deepEqual(Host.activationsOf({ ...detail, result: { ...result, authority: null } }, source()), []);
});
test("Phaser atlas uses intact run rectangles; malformed frames never enter the renderer", () => {
  const frame = x => ({ frame: { x, y: 0, w: 20, h: 30 }, rotated: false });
  const sprite = Host.atlasSprite({ frames: { "front-idle": frame(0), "run-1": frame(20), "run-2": frame(40), "bad": { frame: { x: 0, y: 0, w: -1, h: 5 } } } }, "assets/joe/wizard-joe-hd.webp");
  assert.equal(sprite.animation, "run"); assert.deepEqual(sprite.animations.run.frames, [1, 2]);
  assert.equal(sprite.frames.length, 3); assert.equal(sprite.frames[1].width, 20);
  assert.equal(sprite.animations.run.fps, 14); assert.equal(sprite.animations.walk.fps, 10); assert.equal(sprite.animations.idle.fps, 4);
  assert.equal(sprite.size, 170); assert.equal(sprite.opacity, 0.85);
});
test("World focus delegates selection only and retains the original native content node", async () => {
  const calls = [], activeClasses = new Set(["active"]);
  const content = { classList: { add: key => activeClasses.add(key), toggle: (key, enabled) => enabled ? activeClasses.add(key) : activeClasses.delete(key) } };
  const doc = new EventTarget();
  doc.defaultView = { CustomEvent: class extends Event { constructor(type, options) { super(type); this.detail = options.detail; } }, queueMicrotask };
  doc.getElementById = id => id === "view-chat" ? content : null;
  const native = source(), container = { ownerDocument: doc };
  let rendererOptions, paused = false;
  globalThis.BombSpatialWorld = { attach(node, options) {
    assert.equal(node, container); rendererOptions = options;
    return { element: {}, update() {}, setView: view => calls.push(["presentation", view]), setSprite() {}, setMotionPaused: value => { paused = value; }, getState: () => ({ paused }), mountContent: node => calls.push(["mount", node]), detachContent: restore => calls.push(["restore", restore]), destroy: () => calls.push(["destroy"]) };
  } };
  const background = {};
  const host = Host.attach(container, { getState: () => native, contentElement: content, loadSprite: false, backgroundScene: background, renderScaffold: false, selectSession: async id => { calls.push(["select", id]); native.selectedSession = id; }, activateView: view => { calls.push(["activate", view]); doc.dispatchEvent(new doc.defaultView.CustomEvent("bomb-code:view-selected", { detail: { view } })); } });
  try {
    assert.equal(rendererOptions.backgroundScene, background); assert.equal(rendererOptions.renderScaffold, false);
    await rendererOptions.onSelect("a");
    assert.deepEqual(calls.filter(call => ["select", "activate", "presentation"].includes(call[0])), [["select", "a"], ["activate", "spatial"], ["presentation", "focus"]]);
    assert.equal(calls.find(call => call[0] === "mount")[1], content);
    assert.equal(activeClasses.has("active"), true);
    host.setHostView("settings");
    assert.equal(activeClasses.has("active"), false); assert.equal(paused, true);
    await assert.rejects(rendererOptions.onSelect("removed"), /no longer available/);
    host.destroy();
  } finally { delete globalThis.BombSpatialWorld; }
});
