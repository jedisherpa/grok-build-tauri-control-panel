import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createContext, runInContext } from "node:vm";

// Exercise the actual app owner rather than a copied transition reducer.
const source = readFileSync(new URL("./app.js", import.meta.url), "utf8");
const modes = source.slice(source.indexOf("const APPROVAL_CYCLE ="), source.indexOf("/** Shift+Tab cycles"));
const send = source.slice(source.indexOf("async function sendPrompt() {"), source.indexOf("// Wire buttons"));
function harness(invoke) {
  const elements = new Map();
  const applied = [], errors = [], events = [];
  const state = { selectedSession: "first", sessions: [{ id: "first", live: true, approvalMode: "ask" }], approvalModePending: null, approvalModeUnknown: new Set() };
  const context = createContext({ state, invoke, askConfirm: async () => true, $: id => {
    if (!elements.has(id)) elements.set(id, { disabled: false });
    return elements.get(id);
  }, setApprovalMode: mode => applied.push(mode), updateSendButton() {}, pushEvent: text => events.push(text), shortId: id => id,
  toastError: error => errors.push(String(error)), noteTurn() { throw new Error("blocked send must not change turn presence"); } });
  runInContext(modes + "\n" + send, context);
  return { context, state, applied, errors, events, elements };
}
function deferred() {
  let resolve;
  const promise = new Promise(r => { resolve = r; });
  return { promise, resolve };
}

test("unsupported live Plan leaves acknowledged Ask visible", async () => {
  const calls = [];
  const h = harness(async (name, args) => {
    calls.push([name, args]);
    if (name === "set_approval_mode") throw new Error("Plan capability unavailable");
    return { metadata: { approvalMode: "ask" } };
  });
  await h.context.changeApprovalMode("plan");
  assert.deepEqual(h.applied, ["ask"]);
  assert.equal(calls[0][1].id, "first");
  assert.equal(h.state.sessions[0].approvalMode, "ask");
  assert.equal(h.state.approvalModePending, null);
  assert.ok(h.events.some(text => text.includes("mode change failed")));
});

test("late mode response cannot overwrite another thread's controls", async () => {
  const pending = deferred();
  const h = harness(async name => name === "set_approval_mode" ? pending.promise : { metadata: { approvalMode: "auto" } });
  const request = h.context.changeApprovalMode("auto");
  assert.deepEqual(h.applied, []);
  h.state.selectedSession = "second";
  pending.resolve();
  await request;
  assert.deepEqual(h.applied, []);
  assert.equal(h.state.sessions[0].approvalMode, "auto");
});

test("overlapping mode requests do not widen a pending transition", async () => {
  const pending = deferred();
  let sets = 0;
  const h = harness(async name => {
    if (name === "set_approval_mode") { sets++; return pending.promise; }
    return { metadata: { approvalMode: "plan" } };
  });
  const request = h.context.changeApprovalMode("plan");
  await h.context.changeApprovalMode("yolo");
  assert.equal(sets, 1);
  assert.equal(h.elements.get("plan-mode").disabled, true);
  pending.resolve(); await request;
  assert.deepEqual(h.applied, ["plan"]);
});

test("unconfirmed mode blocks direct keyboard send without touching the draft or turn", async () => {
  let calls = 0;
  const h = harness(async () => { calls++; throw new Error("disconnected"); });
  await h.context.changeApprovalMode("yolo");
  assert.equal(h.state.approvalModeUnknown.has("first"), true);
  assert.deepEqual(h.applied, []);
  const before = calls;
  await h.context.sendPrompt();
  assert.equal(calls, before);
  assert.match(h.errors[0], /Approval state is unconfirmed/);
  assert.equal(h.elements.has("prompt"), false);
});

test("refresh confirms actual host mode before reopening Send", async () => {
  const h = harness(async () => ({ metadata: { approval_mode: "ask" } }));
  h.state.approvalModeUnknown.add("first");
  await h.context.confirmApprovalMode("first");
  assert.equal(h.state.approvalModeUnknown.size, 0);
  assert.deepEqual(h.applied, ["ask"]);
  assert.equal(h.state.sessions[0].approvalMode, "ask");
});

test("mode reconciliation and new transitions share one UI admission gate", async () => {
  const pending = deferred();
  let sets = 0;
  const h = harness(async name => {
    if (name === "set_approval_mode") sets++;
    return pending.promise;
  });
  h.state.approvalModeUnknown.add("first");
  const refresh = h.context.confirmApprovalMode("first");
  await h.context.changeApprovalMode("yolo");
  assert.equal(sets, 0);
  pending.resolve({ metadata: { approvalMode: "ask" } });
  await refresh;
  assert.deepEqual(h.applied, ["ask"]);
  assert.equal(h.state.approvalModePending, null);
});

test("keyboard/controller Yolo cannot bypass declined human confirmation", async () => {
  let calls = 0;
  const h = harness(async () => { calls++; });
  h.context.askConfirm = async () => false;
  await h.context.changeApprovalMode("yolo");
  assert.equal(calls, 0);
  assert.deepEqual(h.applied, []);
  assert.equal(h.state.approvalModePending, null);
});

test("switching threads during Yolo confirmation discards the pending change", async () => {
  let calls = 0;
  const confirmation = deferred();
  const h = harness(async () => { calls++; });
  h.context.askConfirm = () => confirmation.promise;
  const request = h.context.changeApprovalMode("yolo");
  h.state.selectedSession = "second";
  confirmation.resolve(true);
  await request;
  assert.equal(calls, 0);
  assert.deepEqual(h.applied, []);
});
