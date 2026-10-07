import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
const code = fs.readFileSync(new URL('./memory-recall.js', import.meta.url), 'utf8');
const pure = vm.createContext({}); vm.runInContext(code, pure);
const hit = { chunkId: 'a'.repeat(64), text: '<script>ignored()</script>', title: 'Geometry', source: 'codex', kind: 'history', role: 'user', start: 0, end: 26 };
const ready = { schema: 'bomb-code/memory-recall/v1', ok: true, status: 'ready', sourceFresh: true, generation: 'g', chunkCount: 2, vectorCount: 1, vectorStatus: 'partial', hits: [hit] };
test('evidence selection uses current IDs and generation, never client excerpts', () => {
  const payload = pure.MemoryRecallView.selectionPayload(ready, new Set([hit.chunkId]), 'Clarify geometry', 'math');
  assert.equal(payload.generation, 'g'); assert.equal(payload.chunkIds[0], hit.chunkId);
  assert.equal(payload.text, undefined); assert.equal(payload.evidence, undefined);
  assert.throws(() => pure.MemoryRecallView.selectionPayload({ ...ready, sourceFresh: false }, new Set([hit.chunkId]), 'q', ''), /stale/);
  assert.throws(() => pure.MemoryRecallView.selectionPayload(ready, new Set(['foreign']), 'q', ''), /current excerpts/);
  assert.match(pure.MemoryRecallView.coverageText(ready), /partial/);
});
test('source alias candidates retain current generation and UNSELECTED alternatives', () => {
  const value = { status: 'ready', sourceFresh: true, sourceIntegrity: true, referenceIntegrity: true, generation: 'g', recallBasis: { generation: 'g' }, scannedChunks: 2, candidateCount: 1, notReturnedCount: 0, hits: [{ ...hit, sourceConcept: { usageSelection: 'UNSELECTED', occurrences: [{ alternativeSenseIds: ['river','finance'] }], distinctConceptSupport: ['river'] } }] };
  const result = pure.MemoryRecallView.sourceResult(value); assert.equal(result.hits[0].sourceAlias.occurrences[0].alternativeSenseIds.length, 2);
  assert.throws(() => pure.MemoryRecallView.sourceResult({ ...value, generation: 'different' }), /integrity/);
  assert.throws(() => pure.MemoryRecallView.sourceResult({ ...value, sourceFresh: false }), /integrity/);
});
function mount(reply) {
  class Element {
    constructor(tag = 'div') { this.tag = tag; this.children = []; this.handlers = {}; this.value = ''; this.textContent = ''; }
    appendChild(value) { this.children.push(value); return value; }
    replaceChildren(...values) { this.children = values; }
    setAttribute(key, value) { this[key] = value; }
    addEventListener(type, fn) { this.handlers[type] = fn; }
    get firstChild() { return this.children[0]; }
    set innerHTML(value) { throw new Error('Untrusted source must use textContent'); }
  }
  const ids = new Map(['recall-query','recall-topic','recall-source','recall-scope','recall-search','recall-index','recall-embed','recall-stop','recall-refresh','recall-prepare','recall-status','recall-coverage','recall-results','mem-scope'].map(id => [id,new Element()]));
  const events = {}, calls = [], prepared = [], state = { selectedSession: 'thread-a' };
  const document = { getElementById: id => ids.get(id), createElement: tag => new Element(tag), addEventListener: (type,fn) => { events[type] = fn; }, dispatchEvent: event => events[event.type]?.(event) };
  const invoke = async (command,args) => { calls.push({command,args}); return reply(args.action); };
  const ctx = vm.createContext({ document, invoke, state, CustomEvent: class { constructor(type) { this.type = type; } }, WizardJoeGuide: { clearMemoryContext() {}, setMemoryContext: value => prepared.push(value) } });
  vm.runInContext(code,ctx);
  const tick = () => new Promise(resolve => setImmediate(resolve));
  const all = node => [node,...node.children.flatMap(all)];
  return { ids,events,calls,prepared,state,tick,all,ctx };
}
async function searched(ui) { ui.ids.get('recall-query').value = 'Clarify geometry'; ui.ids.get('recall-search').handlers.click(); await ui.tick(); const checkbox = ui.all(ui.ids.get('recall-results')).find(x => x.type === 'checkbox'); checkbox.checked = true; checkbox.handlers.change(); return checkbox; }
test('Analyze this passage prepares only its exact cited excerpt and current topic, with no provider call', async () => {
  const ui = mount(action => action === 'evidence' ? { ...ready, receiptId: 'r', question: hit.text, context: {} } : ready);
  await searched(ui); ui.ids.get('recall-topic').value = 'geometry';
  ui.all(ui.ids.get('recall-results')).find(x => x.tag === 'button' && x.textContent === 'Analyze this passage').handlers.click(); await ui.tick();
  const call = ui.calls[1]; assert.equal(call.args.action, 'evidence'); assert.equal(call.args.payload.query, hit.text); assert.equal(call.args.payload.topic, 'geometry'); assert.deepEqual(Array.from(call.args.payload.chunkIds), [hit.chunkId]); assert.equal(ui.prepared.length, 1); assert(ui.calls.every(c => c.command === 'memory_recall'));
});
test('search and preparation never dispatch an LLM or coding prompt; untrusted text stays text', async () => {
  const ui = mount(action => action === 'evidence' ? { ...ready, receiptId: 'r', question: 'Clarify geometry', context: {} } : ready);
  await searched(ui); ui.ids.get('recall-prepare').handlers.click(); await ui.tick();
  assert.equal(ui.prepared.length,1);
  assert.deepEqual(ui.calls.map(x => x.command), ['memory_recall','memory_recall']);
  assert.deepEqual(ui.calls.map(x => x.args.action), ['search','evidence']);
  assert(ui.all(ui.ids.get('recall-results')).some(x => x.textContent === hit.text));
});
for (const change of ['selection','thread-event','thread-without-event','query']) {
  test(`late evidence is withheld after ${change} changes`, async () => {
    let resolve; const pending = new Promise(r => { resolve = r; });
    const ui = mount(action => action === 'evidence' ? pending : ready);
    const checkbox = await searched(ui); ui.ids.get('recall-prepare').handlers.click();
    if (change === 'selection') { checkbox.checked = false; checkbox.handlers.change(); }
    if (change.startsWith('thread')) { ui.state.selectedSession = 'thread-b'; if (change === 'thread-event') ui.events['bomb-code:thread-selected'](); }
    if (change === 'query') { ui.ids.get('recall-query').value = 'Different question'; ui.ids.get('recall-query').handlers.input(); }
    resolve({ ...ready, receiptId: 'r', question: 'Clarify geometry', context: {} }); await ui.tick();
    assert.equal(ui.prepared.length,0);
  });
}
test('local vector indexing resumes bounded batches until complete without a provider command', async () => {
  let count = 0;
  const ui = mount(() => ({ ...ready, embedded: 128, pendingVectors: ++count === 1 ? 128 : 0 }));
  await ui.ids.get('recall-embed').handlers.click();
  assert.equal(ui.calls.length, 2);
  assert(ui.calls.every(call => call.command === 'memory_recall' && call.args.action === 'embed_batch' && call.args.payload.maxDocuments === 128));
  assert.equal(ui.ids.get('recall-stop').disabled, true);
});
for (const reason of ['pause','query-change','embedding-error']) {
  test(`vector indexing retains the current bounded batch and stops on ${reason}`, async () => {
    let resolve;
    const ui = mount(() => new Promise(r => { resolve = r; }));
    const run = ui.ids.get('recall-embed').handlers.click();
    if (reason === 'pause') ui.ids.get('recall-stop').handlers.click();
    if (reason === 'query-change') ui.ids.get('recall-query').handlers.input();
    resolve({ ...ready, embedded: 8, pendingVectors: 128, ...(reason === 'embedding-error' ? { embeddingError: 'Model changed' } : {}) });
    await run;
    assert.equal(ui.calls.length, 1);
    if (reason === 'pause') assert.match(ui.ids.get('recall-status').textContent, /Paused/);
    if (reason === 'embedding-error') assert.match(ui.ids.get('recall-status').textContent, /Model changed/);
  });
}
for (const reason of ['no-decrease','model-change','generation-change','basis-change']) {
  test(`full indexing stops if coverage or identity changes: ${reason}`, async () => {
    let count = 0;
    const ui = mount(() => {
      count++;
      return { ...ready, chunkCount: 1024, embedded: 128, pendingVectors: count === 1 ? 256 : reason === 'no-decrease' ? 256 : 128,
        generation: count > 1 && reason === 'generation-change' ? 'changed' : 'g',
        model: { digest: count > 1 && reason === 'model-change' ? 'changed' : 'original' },
        embeddingBasis: { fingerprint: count > 1 && reason === 'basis-change' ? 'changed' : 'original' } };
    });
    await ui.ids.get('recall-embed').handlers.click();
    assert.equal(ui.calls.length, 2);
    assert.match(ui.ids.get('recall-status').textContent, /Indexing stopped/);
  });
}
