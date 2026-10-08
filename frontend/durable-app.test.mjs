import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createContext, runInContext } from "node:vm";

const source = readFileSync(new URL("./app.js", import.meta.url), "utf8");
const sendSource = source.slice(source.indexOf("async function sendPrompt() {"), source.indexOf("// Wire buttons"));
const selectionSource = source.slice(source.indexOf("async function selectSession(id,"), source.indexOf("function formatStoredBody("));
const approvalSource = source.slice(source.indexOf("function liveApproval("), source.indexOf("function openToolsFor("));
function deferred() { let resolve, reject; const promise = new Promise((r, j) => { resolve = r; reject = j; }); return { promise, resolve, reject }; }
function sendHarness(invoke, owner = {}) {
  const calls = [], errors = [], elements = new Map([["prompt", { value: "original draft", dispatchEvent() {} }]]);
  const state = { selectedSession: "a", sessions: [{ id: "a", status: "idle", live: true }],
    approvalModePending: null, approvalModeUnknown: new Set(), turn: { phase: "idle" }, startingSession: false };
  const context = createContext({ state, crypto: { randomUUID: () => "submission-id" }, Event: class {},
    $: id => elements.get(id), durableOwner: { gate: () => ({ canSend: true }), select: async () => {}, ...owner },
    invoke: async (name, args) => { calls.push({ name, args }); return invoke?.(name, args); },
    P: { emptyPresence: () => ({ phase: "idle" }) }, updateSendButton() {}, toastError: error => errors.push(String(error)),
    turnActive: () => false, sessionTurnBusy: () => false, pushEvent() {}, holdComposerFocus() {}, shortId: id => id,
    endAgentStream() {}, clearBoomTimer() {}, noteTurn() {}, startPhraseCycle() {}, formatCount: String,
    currentBackend: () => "grok", currentModel: () => "model", currentApprovalMode: () => "ask", modeOn: () => false,
    refreshSessions: async () => {}, appendTranscript() { throw new Error("optimistic transcript write"); }, updateBombChrome() {},
  });
  runInContext(sendSource, context);
  return { context, state, calls, errors, elements };
}

test("actual Send preserves draft on backend failure and uses checked submission identity", async () => {
  const h = sendHarness(() => { throw new Error("intent commit failed"); });
  await h.context.sendPrompt();
  assert.equal(h.elements.get("prompt").value, "original draft");
  assert.equal(h.state.promptSubmissionPending, null);
  assert.equal(h.calls[0].args.clientSubmissionId, "submission-id");
  assert.ok(h.errors.some(error => error.includes("intent commit failed")));
});

test("actual Send clears only its accepted draft, retaining newer edits", async () => {
  const pending = deferred(); const h = sendHarness(() => pending.promise);
  const first = h.context.sendPrompt(); await new Promise(r => setImmediate(r));
  h.elements.get("prompt").value = "newer draft";
  pending.resolve(); await first;
  assert.equal(h.elements.get("prompt").value, "newer draft");
});

test("actual Send admission is serialized across pending history and IPC", async () => {
  const pending = deferred(); const h = sendHarness(undefined, { select: () => pending.promise });
  const first = h.context.sendPrompt(); await h.context.sendPrompt();
  assert.equal(h.calls.length, 0);
  pending.resolve(); await first;
  assert.equal(h.calls.filter(c => c.name === "send_prompt").length, 1);
  assert.equal(h.elements.get("prompt").value, "");
});

test("thread change while Send hydrates abandons admission and retains new thread draft", async () => {
  const pending = deferred(); const h = sendHarness(undefined, { select: () => pending.promise });
  const first = h.context.sendPrompt(); h.state.selectedSession = "b";
  h.elements.get("prompt").value = "b draft"; pending.resolve(); await first;
  assert.equal(h.calls.length, 0); assert.equal(h.elements.get("prompt").value, "b draft");
});

test("unhealthy durable coverage blocks direct keyboard Send without touching draft", async () => {
  const h = sendHarness(undefined, { gate: () => ({ canSend: false, reason: "unknown live approvals" }) });
  await h.context.sendPrompt(); assert.equal(h.calls.length, 0);
  assert.equal(h.elements.get("prompt").value, "original draft");
  assert.match(h.errors[0], /unknown live approvals/);
});

test("late selected-thread load cannot overwrite a newer thread's focus", async () => {
  const pending = deferred(), focus = [], rendered = [];
  const state = { selectedSession: null, sessions: [], presenceBySession: new Map(), transcriptLoaded: new Set() };
  const owner = { select: id => id === "a" ? pending.promise : Promise.resolve(), coverage: () => ({ loaded: true }) };
  const context = createContext({ state, durableOwner: owner, P: { emptyPresence: () => ({ phase: "idle" }) },
    $: () => null, threadDrafts: { switchThread() {} }, document: { dispatchEvent() {} }, CustomEvent: class {},
    renderThreads() {}, syncSelectorsToSession() {}, activateView() {}, setProjectCwd() {},
    renderTranscript: () => rendered.push(state.selectedSession), renderExplainFeed() {}, updateBombChrome() {},
    invoke: async (name, args) => focus.push(args.id) });
  runInContext(selectionSource, context);
  const a = context.selectSession("a"), b = context.selectSession("b"); await b; pending.resolve(); await a;
  assert.deepEqual(focus, ["b"]); assert.deepEqual(rendered, ["b"]);
});

test("live approval action passes exact query-backed runtime/epoch and rejects old cards", async () => {
  const calls = [], pending = [{ requestId: "request", runtimeId: "current-runtime", hostEpoch: 7, options: [] }];
  const context = createContext({ state: { selectedSession: "a" }, durableOwner: {
    coverage: () => ({ healthy: true, durable: true, pendingKnown: true, loading: false, gap: false }),
    pending: () => pending, reconcile: async () => {},
  }, invoke: async (name, args) => calls.push({ name, args }) });
  runInContext(approvalSource, context);
  await context.respondLiveApproval("a", "request", "allow");
  assert.equal(calls[0].args.runtimeId, "current-runtime"); assert.equal(calls[0].args.hostEpoch, 7);
  await assert.rejects(context.respondLiveApproval("a", "historical", "allow"), /historical/);
  assert.equal(calls.length, 1);
});

test("controller transcript render caps actual DOM work at 300 rows", () => {
  // Run the production renderer with a small DOM double; evaluate its full
  // function through the exact next-function boundary rather than a reducer.
  const end = source.indexOf("\nfunction ", source.indexOf("function renderTranscript() {") + 10);
  const actualRender = source.slice(source.indexOf("function renderTranscript() {"), end);
  const nodes = new Map();
  for (const id of ["transcript", "technical-transcript", "composer-session", "composer-model"]) nodes.set(id, {
    innerHTML: "", scrollTop: 0, scrollHeight: 0, querySelectorAll: () => [], classList: { contains: () => false },
  });
  const rows = Array.from({ length: 2000 }, (_, n) => ({ role: "agent", body: `row-${n}`, at: "now", historical: true }));
  const context = createContext({ state: { selectedSession: "a", sessions: [{ id: "a", status: "idle" }], showAcpLines: false, followTail: false },
    $: id => nodes.get(id), getTranscript: () => rows, renderDurableCoverage() {}, shortId: String,
    escapeHtml: String, renderMarkdown: String, termPrefix: String, shortTime: String, bombHtml: () => "",
    updateThreadGitRow() {}, updateBombChrome() {}, scrollTranscriptBottom() {}, roleBombMood: String,
    isNearBottom: () => true, document: { querySelectorAll: () => [] }, queueMicrotask() {}, scrollPendingApprovalIntoView() {},
    requestAnimationFrame: callback => callback(), wireTranscriptFollow() {}, durableOwner: null,
  });
  runInContext(actualRender, context); context.renderTranscript();
  assert.equal((nodes.get("transcript").innerHTML.match(/class="t-block agent/g) || []).length, 300);
  assert.ok(!nodes.get("transcript").innerHTML.includes("row-0<"));
  assert.ok(nodes.get("transcript").innerHTML.includes("row-1999"));
});

test("actual row owner reuses unchanged rows and patches only a growing canonical tail", () => {
  const actual = source.slice(source.indexOf("function applyDurableRows("), source.indexOf("function renderDurableCoverage("));
  const scheduled = [], state = { selectedSession: "a", transcriptBySession: new Map(), transcriptRevisionBySession: new Map(), transcriptLoaded: new Set() };
  const context = createContext({ state, durableOwner: { pending: () => [], coverage: () => ({ loaded: true }) },
    formatStoredBody: (role, body) => body, scheduleDurableRender: tail => scheduled.push(tail) });
  runInContext(actual, context);
  context.applyDurableRows("a", [{ seq: 1, role: "user", body: "question", at: "then" }, { seq: 2, role: "agent", body: "A", at: "now" }]);
  const first = state.transcriptBySession.get("a")[0];
  context.applyDurableRows("a", [{ seq: 1, role: "user", body: "question", at: "then" }, { seq: 2, role: "agent", body: "AB", at: "later" }]);
  assert.equal(state.transcriptBySession.get("a")[0], first);
  assert.equal(scheduled[1].seq, 2); assert.equal(scheduled[1].body, "AB");
  context.applyDurableRows("a", [{ seq: 1, role: "user", body: "question", at: "then" }, { seq: 2, role: "agent", body: "AB", at: "later" }]);
  assert.equal(scheduled.length, 2, "unchanged replay rows do not rebuild the DOM");
  assert.equal(state.transcriptLoaded.has("a"), true);
});

test("actual Send/Stop control retains Stop for a recovered live runtime under failed health", () => {
  const actual = source.slice(source.indexOf("function sessionTurnBusy() {"), source.indexOf("async function cancelCurrentTurn()"));
  const nodes = new Map();
  for (const id of ["btn-send", "composer-gate-hint"]) nodes.set(id, { style: {}, classList: { toggle() {} } });
  const context = createContext({ state: { selectedSession: "a", sessions: [{ id: "a", live: true, status: "running" }],
    approvalModePending: null, approvalModeUnknown: new Set() }, $: id => nodes.get(id), turnActive: () => false,
    sessionIsStarting: () => false, durableOwner: { status: () => "running", gate: () => ({ canSend: false, reason: "sink failed" }) } });
  runInContext(actual, context); context.updateSendButton();
  assert.equal(nodes.get("btn-send").textContent, "Stop"); assert.equal(nodes.get("btn-send").disabled, false);
});

test("actual targeted patch changes a single committed body without rebuilding transcript", () => {
  const actual = source.slice(source.indexOf("function patchDurableTail("), source.indexOf("function scheduleDurableRender("));
  const body = {}, time = {};
  const block = { querySelector: selector => selector === ".t-body" ? body : time };
  const root = { querySelector: selector => selector.includes('"42"') ? block : null };
  const context = createContext({ $: () => root, state: { followTail: false }, renderMarkdown: text => `<p>${text}</p>`, shortTime: String });
  runInContext(actual, context);
  assert.equal(context.patchDurableTail({ seq: 42, role: "agent", body: "actual committed token", at: "now" }), true);
  assert.equal(body.innerHTML, "<p>actual committed token</p>");
  assert.equal(context.patchDurableTail({ seq: 43, role: "agent", body: "not visible" }), false);
});
