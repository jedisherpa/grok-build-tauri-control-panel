import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const context = vm.createContext({});
vm.runInContext(fs.readFileSync(new URL("./wizard-joe.js", import.meta.url), "utf8"), context);
const guide = context.WizardJoeView;
const response = () => ({ schema: "bomb-code/joe-result/v1", sentence: "Use 🌿 only if tests pass.", language: "eng", threadId: "thread-1", authority: { toolsDispatched: false, approvalsGranted: false, memoryCommitted: false }, interpretation: { binding: { readings: [{ id: "r1", frame: { unresolved: ["Which tests?"] } }] } }, clarifications: [{ question: "Which tests?" }] });

test("a result missing the explicit read-only receipt is rejected", () => {
  const result = response(); delete result.authority;
  assert.throws(() => guide.viewOf(result), /boundary receipt/);
  result.authority = { toolsDispatched: false, approvalsGranted: true, memoryCommitted: false };
  assert.throws(() => guide.viewOf(result), /boundary receipt/);
});
test("source readings remain intact, including unresolved items", () => {
  const result = response();
  assert.equal(guide.viewOf(result).readings, result.interpretation.binding.readings);
  assert.equal(guide.viewOf(result).readings[0].frame.unresolved[0], "Which tests?");
  result.schema = "unknown";
  assert.throws(() => guide.viewOf(result), /schema/);
});
test("drafting preserves the existing unsent composer and exact question", () => {
  const question = "Is <script> text literal? 🌿";
  assert.equal(guide.appendDraft("Existing unsent work", question), `Existing unsent work\n\n${question}`);
  assert.equal(guide.appendDraft("", question), question);
  assert.equal(guide.appendDraft("Keep this", " "), "Keep this");
});
test("stale input, language and thread scope cannot match a current review", () => {
  const result = response();
  assert.equal(guide.sameInput(result, result.sentence, "eng", "thread-1"), true);
  assert.equal(guide.sameInput(result, "Use 🌿 if tests pass.", "eng", "thread-1"), false);
  assert.equal(guide.sameInput(result, result.sentence, "spa", "thread-1"), false);
  assert.equal(guide.sameInput(result, result.sentence, "eng", "thread-2"), false);
  result.threadId = null;
  assert.equal(guide.sameInput(result, result.sentence, "eng", null), true);
});

function mount(result, options = {}) {
  let currentResult = result;
  const created = [], calls = [];
  class Element {
    constructor(tag = "div") { this.tag = tag; this.children = []; this.handlers = {}; this.value = ""; this.textContent = ""; created.push(this); }
    appendChild(child) { this.children.push(child); return child; }
    replaceChildren(...children) { this.children = children; }
    addEventListener(type, handler) { this.handlers[type] = handler; }
    dispatchEvent(event) { this.lastEvent = event; return this.handlers[event.type]?.(event); }
    focus() { this.focused = true; }
    set innerHTML(_) { throw new Error("Guide must not use an HTML sink"); }
  }
  const ids = new Map(["wizard-joe", "joe-passage", "joe-language", "joe-result-status", "joe-result", "joe-analyze", "joe-service-status", "joe-refresh-service", "joe-copy-composer", "joe-form", "prompt", "joe-compare", "joe-compare-label", "joe-cdiss-example"].map(id => [id, new Element()]));
  ids.get("joe-language").value = "eng";
  const scope = { selectedSession: "thread-1" };
  const document = { handlers: {}, getElementById: id => ids.get(id), createElement: tag => new Element(tag), addEventListener(type, handler) { this.handlers[type] = handler; }, dispatchEvent(event) { this.handlers[event.type]?.(event); } };
  const invoke = async (command, args) => { calls.push({ command, args }); if (command === "joe_analyze" || command === "joe_cdiss_example") return options.reply || currentResult; if (command === "joe_status") return { available: true, provider: "grok", model: "fixture" }; throw new Error(`Unexpected native command: ${command}`); };
  class TestEvent { constructor(type, options = {}) { this.type = type; Object.assign(this, options); } }
  const context = vm.createContext({ document, invoke, state: scope, CustomEvent: TestEvent, Event: TestEvent });
  vm.runInContext(fs.readFileSync(new URL("./wizard-joe.js", import.meta.url), "utf8"), context);
  return { ids, created, calls, scope, document, context, setResult(value) { currentResult = value; } };
}

function withContinuity(result) {
  result.cdiss = { status: "ready", state: { schema: "bomb-code/cdiss-state/v1", algorithmVersion: "bomb-code/cdiss-source-structure/v1", observation: { readingCount: 1, atomCount: 3, eventCount: 1, alternativeCount: 0, mappedMass: 2 / 3, unmappedMass: 1 / 3 }, continuity: { status: "fresh", reasons: [], sourceDistance: null, structureDistance: null, partitionChanged: null }, basis: {}, stateHash: "fixture-state", configDigest: "fixture-config" } };
  return result;
}

test("continuity distance display rejects nonfinite and out-of-range observations", () => {
  const result = withContinuity(response());
  assert.equal(guide.continuityView(result.cdiss).available, true);
  result.cdiss.state.continuity.sourceDistance = { totalVariation: 0.5, jensenShannonDistance: NaN };
  assert.equal(guide.continuityView(result.cdiss).available, false);
  result.cdiss.state.continuity.sourceDistance.jensenShannonDistance = 2;
  assert.equal(guide.continuityView(result.cdiss).available, false);
});

test("comparison is explicitly selected, scoped to one thread and absent from provider history", async () => {
  const first = withContinuity(response()); first.requestId = "first-review";
  const ui = mount(first); ui.ids.get("joe-passage").value = first.sentence;
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  assert.equal(ui.calls[0].args.compareRequestId, null);
  const second = withContinuity(response()); second.sentence = "Use 🌿 even if tests fail."; second.requestId = "second-review";
  ui.setResult(second); ui.ids.get("joe-passage").value = second.sentence;
  ui.ids.get("joe-passage").handlers.input();
  ui.ids.get("joe-compare").checked = true; ui.ids.get("joe-compare").handlers.change();
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  assert.equal(ui.calls[1].args.compareRequestId, "first-review");
  assert.equal(Object.hasOwn(ui.calls[1].args, "history"), false);
  ui.scope.selectedSession = "thread-2"; ui.document.handlers["bomb-code:thread-selected"]();
  assert.equal(ui.ids.get("joe-compare").checked, false);
  assert.equal(ui.ids.get("joe-compare").disabled, true);
  assert.equal(ui.ids.get("prompt").value, "");
});

test("the local example uses its separate zero-provider command and cannot become previous user context", async () => {
  const first = withContinuity(response()), second = withContinuity(response());
  const sample = { schema: "bomb-code/cdiss-example/v1", authority: first.authority, first, second };
  const ui = mount(sample); ui.ids.get("prompt").value = "Keep my unsent work";
  await ui.ids.get("joe-cdiss-example").handlers.click();
  assert.deepEqual(ui.calls.map(c => c.command), ["joe_cdiss_example"]);
  assert.match(ui.ids.get("joe-result-status").textContent, /Authored source-backed example/);
  assert.equal(ui.ids.get("prompt").value, "Keep my unsent work");
  assert.equal(ui.ids.get("joe-compare").disabled, true);
  ui.ids.get("joe-passage").value = "New passage"; ui.ids.get("joe-passage").handlers.input();
  assert.equal(ui.ids.get("joe-result").children.length, 0);
});

test("guide starts idle; explicit passage review renders untrusted text without executing it or sending work", async () => {
  const result = response();
  result.requestId = "fixture-request";
  result.clarifications[0].question = "Clarify <img src=x onerror=alert(1)>?";
  const ui = mount(result);
  assert.equal(ui.calls.length, 0);
  ui.ids.get("prompt").value = result.sentence;
  ui.ids.get("joe-copy-composer").handlers.click();
  assert.equal(ui.calls.length, 0);
  assert.equal(ui.ids.get("joe-passage").value, result.sentence);
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  assert.equal(ui.calls.length, 1);
  assert.equal(ui.calls[0].command, "joe_analyze");
  assert.equal(ui.calls[0].args.sentence, result.sentence);
  assert.ok(ui.created.some(node => node.textContent === result.clarifications[0].question));
  const draft = ui.created.find(node => node.tag === "button" && node.textContent === "Add question to unsent message");
  draft.handlers.click();
  assert.equal(ui.ids.get("prompt").value, `${result.sentence}\n\n${result.clarifications[0].question}`);
  assert.equal(ui.calls.length, 1);
  const visual = ui.ids.get("wizard-joe").lastEvent;
  assert.equal(visual.type, "bomb-code:joe-interpretation");
  assert.equal(visual.detail.result, result);
});

test("a retained review cannot draft into a newly selected thread and edited passages invalidate visuals", async () => {
  const result = response(), ui = mount(result);
  ui.ids.get("joe-passage").value = result.sentence;
  ui.ids.get("prompt").value = "New thread draft";
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  const draft = ui.created.find(node => node.tag === "button" && node.textContent === "Add question to unsent message");
  ui.scope.selectedSession = "thread-2";
  draft.handlers.click();
  assert.equal(ui.ids.get("prompt").value, "New thread draft");
  assert.match(ui.ids.get("joe-result-status").textContent, /another thread/);
  ui.ids.get("joe-passage").value = "Changed wording";
  ui.ids.get("joe-passage").handlers.input();
  assert.equal(ui.ids.get("joe-result").children.length, 0);
  assert.equal(ui.ids.get("wizard-joe").lastEvent.detail.status, "invalidated");
  draft.handlers.click();
  assert.equal(ui.ids.get("prompt").value, "New thread draft");
});

test("thread-selection seam clears retained reading and geometry without changing passage or draft", async () => {
  const result = response(), ui = mount(result);
  ui.ids.get("joe-passage").value = result.sentence;
  ui.ids.get("prompt").value = "Unsent work";
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  const draft = ui.created.find(node => node.tag === "button" && node.textContent === "Add question to unsent message");
  ui.scope.selectedSession = "thread-2";
  ui.document.dispatchEvent({ type: "bomb-code:thread-selected", detail: { threadId: "thread-2" } });
  assert.equal(ui.ids.get("joe-result").children.length, 0);
  assert.equal(ui.ids.get("joe-passage").value, result.sentence);
  assert.equal(ui.ids.get("prompt").value, "Unsent work");
  assert.equal(ui.ids.get("wizard-joe").lastEvent.detail.reason, "thread-changed");
  assert.equal(ui.ids.get("wizard-joe").lastEvent.detail.result, null);
  draft.handlers.click();
  assert.equal(ui.ids.get("prompt").value, "Unsent work");
  assert.equal(ui.calls.length, 1);
});

test("service can be rechecked on each opening or explicit refresh without model generation", async () => {
  const ui = mount(response());
  ui.ids.get("wizard-joe").open = true;
  ui.ids.get("wizard-joe").handlers.toggle();
  await new Promise(resolve => setImmediate(resolve));
  ui.ids.get("joe-refresh-service").handlers.click();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(ui.calls.map(call => call.command), ["joe_status", "joe_status"]);
});

test("actual fitted-position fields and full-lattice residual remain distinct and visible", async () => {
  const result = response();
  result.interpretation.binding.readings[0].e8_activations = [{ concept_id: "concept:example", source_mapping_asserted: true, placement: { status: "fitted", root_id: "e8-root:130", radius: 1, hierarchy_level: 0, position8: [0.1, 0.2], residual8: [0.01, 0.02] }, lattice_address: { geometry_version: "example/v1", coset: "integer-even", fine_position_scale: 8, position: [1, 2], residual: [0.3, 0.4], reconstruction: "fine = (lattice position + residual) / scale" } }];
  const ui = mount(result); ui.ids.get("joe-passage").value = result.sentence;
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  const visible = ui.created.map(n => n.textContent).join("\n");
  assert.match(visible, /fitted · source mapping asserted/);
  assert.match(visible, /Root anchor: e8-root:130 · radius: 1/);
  assert.match(visible, /Fine-position residual from root anchor: 0.01, 0.02/);
  assert.match(visible, /Lattice residual: 0.3, 0.4/);
});

test("adapter failure stage and validation reason remain inspectable without dumping provider prompts", async () => {
  const result = response(); result.error = null; result.status = "interpretation-unavailable";
  result.interpretation = { error: "Source sense selection was rejected", failed_stage: "select", execution: { calls: [{ stage: "selection", attempt: 1, generation: "returned", validation: "rejected", validation_error: "Unknown source sense id", prompt: "FULL PRIVATE PROVIDER PROMPT", response: "FULL PROVIDER RESPONSE" }] } };
  result.clarifications = [];
  const ui = mount(result); ui.ids.get("joe-passage").value = result.sentence;
  await ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  const visible = ui.created.map(n => n.textContent).join("\n");
  assert.match(visible, /Interpretation stopped at stage: select/);
  assert.match(visible, /Source sense selection was rejected/);
  assert.match(visible, /Validation error: Unknown source sense id/);
  assert.doesNotMatch(visible, /FULL PRIVATE PROVIDER PROMPT|FULL PROVIDER RESPONSE/);
  assert.match(ui.ids.get("joe-result-status").textContent, /unavailable/);
});


test("same-thread context change rejects a late result before rendering or geometry publication", async () => {
  const result = response(); let finish; const reply = new Promise(resolve => { finish = resolve; });
  const ui = mount(result, { reply }); let current = true;
  ui.context.WizardJoeGuide.setContextValidator(() => current);
  ui.context.WizardJoeGuide.setPassage(result.sentence, "Prepared locally");
  assert.equal(ui.calls.length, 0);
  const running = ui.ids.get("joe-form").handlers.submit({ preventDefault() {} });
  current = false; finish(result); await running;
  assert.equal(ui.ids.get("joe-result").children.length, 0);
  assert.match(ui.ids.get("joe-result-status").textContent, /context changed/);
  assert.notEqual(ui.ids.get("wizard-joe").lastEvent?.detail?.result, result);
});

test("context changes reject both immediate stale drafts and an analysis started before the observer tick", async () => {
  const result=response(), ui=mount(result); let current=true;
  ui.context.WizardJoeGuide.setContextValidator(() => current);
  ui.context.WizardJoeGuide.setPassage(result.sentence);
  await ui.ids.get("joe-form").handlers.submit({preventDefault(){}});
  current=false;
  const draft=ui.created.find(el=>el.textContent==="Add question to unsent message");
  assert.ok(draft); draft.handlers.click();
  assert.equal(ui.ids.get("prompt").value,"");
  const before=ui.calls.filter(c=>c.command==="joe_analyze").length;
  await ui.ids.get("joe-form").handlers.submit({preventDefault(){}});
  assert.equal(ui.calls.filter(c=>c.command==="joe_analyze").length,before);
  assert.match(ui.ids.get("joe-result-status").textContent,/context changed/);
});
