import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
const code = fs.readFileSync(new URL('./meaning-memory.js', import.meta.url), 'utf8');
const pure = vm.createContext({}); vm.runInContext(code, pure);
const qid = '11111111-1111-4111-8111-111111111111', cid = '22222222-2222-4222-8222-222222222222';
const authority = { toolsDispatched: false, approvalsGranted: false, memoryCommitted: false };
const distance = { totalVariation: 1, jensenShannonDistance: 1 };
const response = () => ({ schema: 'bomb-code/meaning-memory/v1', status: 'ready', authority, comparisonFingerprint: 'a'.repeat(64), query: { profileId: qid }, cards: [{ candidateProfileId: cid, readingPairs: [{ queryReadingId: 'q', candidateReadingId: 'c', sourceSenseDistance: distance, assertedConceptDistance: distance, contextPinDistance: null, senseSnapDictionaryDistance: distance, eventLocalDistance: distance, canonicalLinksAndReferencesEqual: false, geometry: { correspondenceCount: 1, rootCollisions: [{}], fineDistances: [] }, eventDifferences: [], multiplicity: {}, coverage: {}, uncertainty: {} }] }], unsentQuestions: [{ candidateProfileId: cid, text: 'Who approves <script> this?', basis: { queryReadingId: 'q', candidateReadingId: 'c' }, unsent: true }], overallRelevanceScore: null });
const catalog = { schema: 'bomb-code/meaning-memory/v1', status: 'ready', authority, profiles: [qid,cid].map((profileId,i) => ({ profileId, requestId: profileId, sentence: i ? 'The child rejects the artifact.' : 'The parent approves the child.', available: true, kind: 'passage', citationIds: ['citation'], threadId: 'thread-a' })), counts: { total: 2, unchecked: 0 }, nextOffset: null };
const dictionary = { schema: 'bomb-code/dictionary-shapes/v1', status: 'ready', resultCount: 1, hits: [{ senseId: 'sense1', lemma: '<img> bank', language: 'eng', definition: 'Financial institution', conceptIds: ['finance','candidate'], assertedConceptIds: ['finance'], concepts: [{ conceptId: 'finance', sharedConceptGloss: 'Financial institution' }] }] };
test('source picker uses only native asserted concept identities with attributed definitions', () => {
  const choices = pure.MeaningMemoryView.conceptsOf(dictionary);
  assert.equal(choices.length, 1); assert.equal(choices[0].id, 'finance'); assert.equal(choices[0].definition, 'Financial institution');
  assert.throws(() => pure.MeaningMemoryView.conceptsOf({ ...dictionary, hits: [{ ...dictionary.hits[0], assertedConceptIds: ['foreign'] }] }), /invalid/);
});
test('comparison boundary rejects authority, identity, confidence and nonfinite distances', () => {
  const req = pure.MeaningMemoryView.comparePayload(qid, new Set([cid]));
  assert.equal(pure.MeaningMemoryView.comparisonOf(response(), req).cards.length, 1);
  for (const edit of [v => v.authority = { ...authority, toolsDispatched: true }, v => v.cards[0].candidateProfileId = qid, v => v.overallRelevanceScore = 1, v => v.cards[0].readingPairs[0].sourceSenseDistance = { ...distance, totalVariation: Infinity }]) { const v = response(); edit(v); assert.throws(() => pure.MeaningMemoryView.comparisonOf(v, req)); }
  assert.throws(() => pure.MeaningMemoryView.comparePayload(qid, new Set([qid])), /different/);
  assert.throws(() => pure.MeaningMemoryView.comparePayload('../path', new Set([cid])));
  assert.equal(pure.MeaningMemoryView.distance(null), 'Unavailable');
});
function mount(reply, loadRecall = false) {
  class Element {
    constructor(tag = 'div') { this.tag = tag; this.children = []; this.handlers = {}; this.value = ''; this.textContent = ''; }
    appendChild(v) { this.children.push(v); return v; }
    replaceChildren(...vs) { this.children = vs; }
    addEventListener(type, fn) { this.handlers[type] = fn; }
    setAttribute(key,value) { this[key] = value; }
    get firstChild() { return this.children[0]; }
    set innerHTML(_) { throw new Error('Untrusted source must remain text'); }
  }
  const names = ['meaning-memory','meaning-dictionary-form','meaning-word','meaning-language','meaning-dictionary-search','meaning-source-search','meaning-refresh','meaning-import','meaning-compare','meaning-dictionary-prev','meaning-dictionary-next','meaning-profile-prev','meaning-profile-next','meaning-concepts','meaning-source-status','meaning-profile-status','meaning-query-profile','meaning-profiles','meaning-comparison-status','meaning-comparisons','joe-passage','joe-language'];
  if (loadRecall) names.push('recall-query','recall-topic','recall-source','recall-scope','recall-search','recall-index','recall-embed','recall-stop','recall-refresh','recall-prepare','recall-status','recall-coverage','recall-results','mem-scope');
  const ids = new Map(names.map(id => [id,new Element()])); ids.get('meaning-language').value = 'eng'; ids.get('meaning-word').value = 'bank';
  const events = {}, calls = [], drafted = [], displayed = [], prepared = [], state = { selectedSession: 'thread-a' }, filters = { query: 'Find it', topic: 'finance', source: 'history', scope: '' };
  const document = { getElementById: id => ids.get(id), createElement: tag => new Element(tag), addEventListener: (type, fn) => { (events[type] ||= []).push(fn); } };
  const fire = (type, detail) => (events[type] || []).forEach(fn => fn({ type, detail }));
  document.dispatchEvent = event => fire(event.type,event.detail);
  const invoke = async (command,args) => { calls.push({ command,args }); return reply(command,args); };
  const ctx = vm.createContext({ document, invoke, state, CustomEvent: class { constructor(type,options = {}) { this.type = type; Object.assign(this,options); } }, MemoryRecall: { filters: () => ({ ...filters }), invalidate: reason => fire('bomb-code:recall-invalidated',{ reason }), showSourceCandidates: value => displayed.push(value) }, WizardJoeGuide: { clearMemoryContext() {}, setMemoryContext: value => prepared.push(value), async draftMemoryQuestion(options) { await options.validate(); if (!options.isCurrent()) return false; drafted.push(options.question); return true; } } });
  if (loadRecall) vm.runInContext(fs.readFileSync(new URL('./memory-recall.js',import.meta.url),'utf8'),ctx);
  vm.runInContext(code,ctx);
  const all = el => [el,...el.children.flatMap(all)], tick = () => new Promise(r => setImmediate(r));
  return { ids, calls, drafted, displayed, prepared, state, filters, fire, all, tick };
}
async function choose(ui) {
  ui.ids.get('meaning-refresh').handlers.click(); await ui.tick();
  ui.ids.get('meaning-query-profile').value = qid; ui.ids.get('meaning-query-profile').handlers.change();
  const boxes = ui.all(ui.ids.get('meaning-profiles')).filter(n => n.type === 'checkbox'); boxes[1].checked = true; boxes[1].handlers.change();
  return boxes[1];
}
test('source lookup stays local, uses filters, and renders untrusted dictionary text safely', async () => {
  const source = { schema: 'bomb-code/source-concept-candidates/v1', status: 'ready', authority };
  const ui = mount((command,args) => command === 'word_shape_dictionary' ? dictionary : args.action === 'source_candidates' ? source : catalog);
  ui.ids.get('meaning-dictionary-form').handlers.submit({ preventDefault() {} }); await ui.tick();
  assert(ui.all(ui.ids.get('meaning-concepts')).some(n => n.textContent.includes('<img> bank')));
  const box = ui.all(ui.ids.get('meaning-concepts')).find(n => n.type === 'checkbox'); box.checked = true; box.handlers.change();
  ui.ids.get('meaning-source-search').handlers.click(); await ui.tick();
  assert.equal(ui.displayed.length, 1); const call = ui.calls[1]; assert.equal(call.command, 'meaning_memory'); assert.equal(call.args.action, 'source_candidates'); assert.equal(call.args.payload.conceptIds[0], 'finance'); assert.equal(call.args.payload.source, 'history'); assert.equal(call.args.payload.scope, '');
});
test('actual source cards prepare exact cited passage through existing Memory and Joe seams', async () => {
  const text = 'The bank approved the loan.';
  const source = { schema: 'bomb-code/source-concept-candidates/v1', status: 'ready', authority, sourceFresh: true, sourceIntegrity: true, referenceIntegrity: true, generation: 'g', recallBasis: { generation: 'g' }, scannedChunks: 1, candidateCount: 1, notReturnedCount: 0, hits: [{ chunkId: 'b'.repeat(64), text, title: 'Source', source: 'codex', start: 0, end: text.length, sourceConcept: { usageSelection: 'UNSELECTED', occurrences: [{ span: [4,8], alternativeSenseIds: ['river','finance'] }], distinctConceptSupport: ['finance'] } }] };
  const evidence = { schema: 'bomb-code/memory-recall/v1', ok: true, status: 'ready', sourceFresh: true, generation: 'g', receiptId: qid, question: text, context: { schema: 'bomb-code/recalled-evidence/v1', evidence: [{ text }] } };
  const ui = mount((command,args) => command === 'word_shape_dictionary' ? dictionary : command === 'memory_recall' ? evidence : source, true);
  ui.ids.get('meaning-dictionary-form').handlers.submit({ preventDefault() {} }); await ui.tick();
  const box = ui.all(ui.ids.get('meaning-concepts')).find(n => n.type === 'checkbox'); box.checked = true; box.handlers.change(); ui.ids.get('meaning-source-search').handlers.click(); await ui.tick();
  const cards = ui.all(ui.ids.get('recall-results')); assert(cards.some(n => n.textContent.includes('UNSELECTED')));
  cards.find(n => n.tag === 'button' && n.textContent === 'Prepare this passage in Joe').handlers.click(); await ui.tick();
  assert.equal(ui.prepared.length, 1); const call = ui.calls.at(-1); assert.equal(call.command, 'memory_recall'); assert.equal(call.args.action, 'evidence'); assert.equal(call.args.payload.query, text); assert.equal(call.args.payload.chunkIds[0], source.hits[0].chunkId);
  assert(ui.calls.every(c => ['word_shape_dictionary','meaning_memory','memory_recall'].includes(c.command)));
});
test('saved proposal import and current Joe autosave update catalog without provider calls', async () => {
  const ui = mount((_,args) => args.action === 'import' ? { schema: catalog.schema, status: 'ready', authority, imported: 1, existing: 0, withheld: [{ reason: 'Original proposal unavailable' }] } : catalog);
  ui.ids.get('meaning-import').handlers.click(); await ui.tick(); await ui.tick();
  assert.deepEqual(ui.calls.map(c => c.args.action), ['import','status']); assert.match(ui.ids.get('meaning-profile-status').textContent, /1 withheld/);
  ui.fire('bomb-code:joe-interpretation', { status: 'ready', result: { meaningProfile: { status: 'ready', profileId: qid, kind: 'passage' }, requestId: qid, sentence: 'Source passage', language: 'eng', threadId: 'thread-a' } }); await ui.tick();
  assert.equal(ui.ids.get('meaning-query-profile').value, qid); assert(ui.calls.every(c => c.command === 'meaning_memory' && ['status','import'].includes(c.args.action)));
});
test('comparison draft requires a fresh native fingerprint and remains unsent', async () => {
  const ui = mount((_,args) => args.action === 'status' ? catalog : args.action === 'compare' ? response() : { schema: catalog.schema, status: 'ready', authority, comparisonFingerprint: 'a'.repeat(64) });
  await choose(ui); ui.ids.get('meaning-compare').handlers.click(); await ui.tick();
  assert(ui.all(ui.ids.get('meaning-comparisons')).some(n => n.textContent.includes('same-root collisions')));
  const copy = ui.all(ui.ids.get('meaning-comparisons')).find(n => n.tag === 'button' && n.textContent === 'Add question to unsent message'); await copy.handlers.click();
  assert.equal(ui.drafted[0], response().unsentQuestions[0].text); assert.deepEqual(ui.calls.map(c => c.args.action), ['status','compare','validate']); assert(ui.calls.every(c => c.command === 'meaning_memory'));
});
for (const change of ['profile','thread-event','thread-silent','recall','topic-silent','joe-passage']) test(`late comparison is withheld after ${change}`, async () => {
  let resolve; const pending = new Promise(r => { resolve = r; }); const ui = mount((_,args) => args.action === 'status' ? catalog : pending);
  const box = await choose(ui); ui.ids.get('meaning-compare').handlers.click();
  if (change === 'profile') { box.checked = false; box.handlers.change(); }
  if (change.startsWith('thread')) { ui.state.selectedSession = 'thread-b'; if (change === 'thread-event') ui.fire('bomb-code:thread-selected'); }
  if (change === 'recall') ui.fire('bomb-code:recall-invalidated');
  if (change === 'topic-silent') ui.filters.topic = 'Changed';
  if (change === 'joe-passage') { ui.ids.get('joe-passage').value = 'Edited'; ui.ids.get('joe-passage').handlers.input(); }
  resolve(response()); await ui.tick(); assert.equal(ui.ids.get('meaning-comparisons').children.length, 0); assert.equal(ui.drafted.length, 0);
});
for (const change of ['profile','thread','fingerprint']) test(`pending question validation cannot draft after ${change}`, async () => {
  let resolve; const pending = new Promise(r => { resolve = r; }); const ui = mount((_,args) => args.action === 'status' ? catalog : args.action === 'compare' ? response() : pending);
  const box = await choose(ui); ui.ids.get('meaning-compare').handlers.click(); await ui.tick();
  const copy = ui.all(ui.ids.get('meaning-comparisons')).find(n => n.tag === 'button' && n.textContent === 'Add question to unsent message'); const copying = copy.handlers.click();
  if (change === 'profile') { box.checked = false; box.handlers.change(); }
  if (change === 'thread') ui.state.selectedSession = 'thread-b';
  resolve({ schema: catalog.schema, status: 'ready', authority, comparisonFingerprint: (change === 'fingerprint' ? 'b' : 'a').repeat(64) }); await copying; assert.equal(ui.drafted.length, 0);
});
test('late dictionary source choices do not survive edited words', async () => {
  let resolve; const ui = mount(() => new Promise(r => { resolve = r; })); ui.ids.get('meaning-dictionary-form').handlers.submit({ preventDefault() {} });
  ui.ids.get('meaning-word').value = 'shore'; ui.ids.get('meaning-word').handlers.input(); resolve(dictionary); await ui.tick(); assert.equal(ui.ids.get('meaning-concepts').children.length, 0);
});
