import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
const code = fs.readFileSync(new URL('./word-shapes.js', import.meta.url), 'utf8');
function mount() {
  class Element {
    constructor(tag = 'div') { this.tag = tag; this.children = []; this.handlers = {}; this.attrs = {}; this.textContent = ''; this.className = ''; this.open = false; this.classList = { add: name => { this.className += ` ${name}`; } }; }
    appendChild(value) { this.children.push(value); return value; }
    replaceChildren(...values) { this.children = values; }
    setAttribute(key, value) { this.attrs[key] = value; }
    addEventListener(type, fn) { this.handlers[type] = fn; }
    set innerHTML(_) { throw new Error('No markup interpolation allowed'); }
  }
  const document = { createElement: tag => new Element(tag), createElementNS: (_, tag) => new Element(tag) };
  const context = vm.createContext({ document, invoke: () => { throw new Error('No provider or native command from renderer'); }, fetch: () => { throw new Error('No network from renderer'); } });
  vm.runInContext(code, context);
  const root = new Element();
  const all = node => [node, ...node.children.flatMap(all)];
  const contents = () => all(root).map(node => node.textContent).join('\n');
  const open = node => { node.open = true; node.handlers.toggle?.(); };
  const findDetails = needle => all(root).find(node => node.tag === 'details' && node.children[0]?.textContent.includes(needle));
  return { api: context.BombWordShapes, root, all, contents, open, findDetails };
}
const activation = { concept_id: 'finance', placement: { status: 'fitted', root_id: 'e8-root:2', root_index: 1, position8: [1,0,0,0,0,0,0,0], residual8: [0.1,0,0,0,0,0,0,0], scaled_anchor8: [0.9,0,0,0,0,0,0,0] } };
const alternative = senseId => ({ senseId, selected: true, conceptIds: ['finance'], dictionary: { lemma: 'bank', language: 'en', definition: '<img src=x onerror=evil()> a financial institution', definitionStatus: 'source-definition', source_id: 'dictionary-source', source_record_id: 'record-1', evidence_refs: ['proof-1'] }, crossLanguage: { status: 'source-alignment', alignments: [{ source_id: 'alignment-source', sense_id: 'de:bank', confidence: 'source-assertion' }], counterpartsStatus: 'not-in-native-packet' }, e8: { status: 'native-placement', activations: [activation] } });
const occurrence = (atomId = 'a', span = [0,4]) => ({ atomId, surface: 'bank', span, tokenPosition: span[0] === 0 ? 1 : 3, tokenPositions: [1], sentenceTokenCount: 3, roleBindings: [{ role: 'agent', eventId: 'event-1' }], alternatives: [alternative('sense-one'), alternative('sense-two')], senseSnap: { centers: [{ origin: 'dictionary-source', source_sense_ids: ['sense-one'], concept_id: 'finance', source_id: 'snap-source' }] }, usageOrientation: { arrows: [{ role: 'agent', eventId: 'event-1', angleDegrees: 45, polarity: 'negative', modality: 'conditional', cueSpans: [[5,10]] }], eventLinks: [{ kind: 'condition', to: 'e2' }], references: [], basis: 'declared-fixed-role-frame', nativeGeometryRotated: false } });
const ready = () => ({ schema: 'bomb-code/word-shapes/v1', status: 'ready', sentence: 'bank beside bank', language: 'en', basis: { manifestSha256: 'abc' }, tokenCoverage: { sentenceTokenCount: 3, coveredTokenCount: 2, uncoveredTokenCount: 1, spanUnit: 'unicode-code-points', tokens: [{ surface: 'bank', span: [0,4], covered: true }, { surface: 'beside', span: [5,11], covered: false }, { surface: 'bank', span: [12,16], covered: true }] }, readings: [{ readingId: 'r1', occurrences: [occurrence('first',[0,4]), occurrence('second',[12,16])] }] });
test('every token including uncovered and repeated words is visible as separate occurrence', () => {
  const ui = mount(); assert.equal(ui.api.render(ui.root, ready()), true);
  assert.match(ui.contents(), /2\. beside · span \[5,11\] · uncovered/);
  assert(ui.findDetails('atom first')); assert(ui.findDetails('atom second'));
  assert.match(ui.contents(), /uncoveredTokenCount: 1/);
  assert.match(ui.contents(), /Tokenization is heuristic/);
});
test('lazy shape keeps source text inert and shows four markers and unchanged native geometry', () => {
  const ui = mount(); ui.api.render(ui.root, ready());
  assert(!ui.contents().includes('financial institution'));
  ui.open(ui.findDetails('atom first'));
  assert.match(ui.contents(), /<img src=x onerror=evil\(\)> a financial institution/);
  assert.equal(ui.all(ui.root).filter(node => node.tag === 'img').length, 0);
  assert.match(ui.contents(), /root e8-root:2/);
  assert.match(ui.contents(), /Fine position: \[1, 0, 0, 0, 0, 0, 0, 0\]/);
  assert.match(ui.contents(), /Root residual: \[0\.1, 0, 0, 0, 0, 0, 0, 0\]/);
  const glyph = ui.all(ui.root).find(node => node.tag === 'svg' && node.attrs['aria-label']?.includes('Dictionary'));
  assert(glyph); assert.match(glyph.attrs['aria-label'], /Cross-language.*SenseSnap.*Sentence use/);
  assert.match(ui.contents(), /display coordinates, not calibrated semantic distances/);
  ui.open(ui.findDetails('Attributed alignments'));
  assert.match(ui.contents(), /alignment-source/);
  ui.open(ui.findDetails('Dictionary evidence'));
  assert.match(ui.contents(), /proof-1/);
});
test('colliding root positions retain both candidate sense identities without merging', () => {
  const ui = mount(); ui.api.render(ui.root, ready()); ui.open(ui.findDetails('atom first')); ui.open(ui.findDetails('sense-two'));
  assert(ui.findDetails('sense-one')); assert(ui.findDetails('sense-two'));
  assert.equal(ui.all(ui.root).filter(node => node.textContent.includes('root e8-root:2')).length, 2);
  assert.match(ui.contents(), /Root collisions do not merge/);
});
test('role direction and scope stay declared grammar metadata, no native E8 rotation', () => {
  const ui = mount(); ui.api.render(ui.root, ready()); ui.open(ui.findDetails('atom first'));
  assert.match(ui.contents(), /Role agent · event event-1 · angle 45°/);
  assert.match(ui.contents(), /Fixed grammar legend, clockwise from right/);
  assert.match(ui.contents(), /polarity: negative/); assert.match(ui.contents(), /modality: conditional/);
  assert.match(ui.contents(), /cueSpans/); assert.match(ui.contents(), /They do not rotate the native E8 address or infer meaning/);
  assert.equal(ui.all(ui.root).filter(node => node.attrs.class === 'word-shape-usage-arrow').length, 1);
});
for (const bad of [[1,2], [NaN,0,0,0,0,0,0,0], [Infinity,0,0,0,0,0,0,0], ['1',0,0,0,0,0,0,0]]) {
  test(`malformed native vectors are withheld (${String(bad[0])}, length ${bad.length})`, () => {
    const ui = mount(), packet = ready(); packet.readings[0].occurrences[0].alternatives = [alternative('bad')];
    packet.readings[0].occurrences[0].alternatives[0].e8.activations = [{ placement: { root_id: 'e8-root:2', position8: bad, residual8: bad } }];
    ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first'));
    assert.match(ui.contents(), /Fine position unavailable or malformed/);
    assert.equal(ui.all(ui.root).filter(node => node.className.includes('word-shape-profile')).length, 0);
  });
}
test('missing definition, alignment, meeting and geometry keep visible gaps', () => {
  const ui = mount(), packet = ready(); packet.readings[0].occurrences[0].alternatives = [{ senseId: 'missing', dictionary: {}, crossLanguage: {}, e8: {} }]; packet.readings[0].occurrences[0].senseSnap = {};
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first'));
  const glyph = ui.all(ui.root).find(node => node.attrs['aria-label']?.includes('Dictionary'));
  assert.match(glyph.attrs['aria-label'], /definition unavailable.*alignment unavailable.*meeting unavailable/);
  assert.match(ui.contents(), /No native fine position or root address/);
});
test('unknown role angle has no drawn orientation', () => {
  const ui = mount(), packet = ready(); packet.readings[0].occurrences[0].usageOrientation.arrows = [{ role: 'unrecognized', eventId: 'e', angleDegrees: null }];
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first'));
  assert.match(ui.contents(), /Role unrecognized · event e · angle unavailable/);
  assert.equal(ui.all(ui.root).filter(node => node.attrs.class === 'word-shape-usage-arrow').length, 0);
});
test('large repeated data is paged, capped and lazily expanded', () => {
  const ui = mount(), packet = ready(); packet.readings[0].occurrences = Array.from({ length: 10000 }, (_, i) => occurrence(`a${i}`));
  packet.tokenCoverage.tokens = Array.from({ length: 20000 }, () => ({ surface: 'bank', span: [0,4], covered: true }));
  ui.api.render(ui.root, packet);
  assert(ui.all(ui.root).length < 160);
  assert.equal(ui.all(ui.root).filter(node => node.tag === 'details' && node.children[0]?.textContent.includes('· atom')).length, 12);
  assert.match(ui.contents(), /Display limited to 512 occurrences/);
  assert.match(ui.contents(), /Display limited to 12000 tokens/);
  const button = ui.all(ui.root).find(node => node.tag === 'button' && node.textContent.includes('occurrences'));
  button.handlers.click();
  assert.equal(ui.all(ui.root).filter(node => node.tag === 'details' && node.children[0]?.textContent.includes('· atom')).length, 24);
});
test('dictionary source browser retains missing usage and does not invent SenseSnap or sentence roles', () => {
  const ui = mount(); const hit = { senseId: 'dictionary-one', language: 'de', lemma: 'Bank', definition: 'Sitzmöbel', definitionLanguage: 'de', definitionStatus: 'sense-local', sourceId: 'source', sourceRecordId: 'record', concepts: [{ conceptId: 'bench', placement: activation.placement }], alignments: [], counterpartCount: 0, counterparts: [] };
  assert.equal(ui.api.renderDictionary(ui.root, { schema: 'bomb-code/dictionary-shapes/v1', status: 'ready', coverage: { senseCount: 34801 }, hits: [hit] }), true);
  assert.match(ui.contents(), /Sentence use unavailable until a sentence is interpreted/);
  assert.match(ui.contents(), /SenseSnap meeting and sentence use remain unavailable/);
  assert.match(ui.contents(), /Sitzmöbel/); assert.match(ui.contents(), /sourceRecordId: record/);
  assert.equal(ui.all(ui.root).filter(node => node.attrs.class === 'word-shape-usage-arrow').length, 0);
});
test('invalid/stale attachment replaces previous shape with clear unavailable state', () => {
  const ui = mount(); ui.api.render(ui.root, ready()); assert.equal(ui.api.render(ui.root, { schema: 'bomb-code/word-shapes/v1', status: 'stale', reason: 'source changed' }), false);
  assert.match(ui.contents(), /Word shapes unavailable: source changed/); assert(!ui.contents().includes('atom first'));
  assert.equal(ui.api.renderDictionary(ui.root, null), false); assert.match(ui.contents(), /Dictionary shapes unavailable/);
});
test('hostile cyclic source previews remain bounded and inert', () => {
  const ui = mount(), packet = ready(); const record = { content: '<script>bad()</script>' }; record.self = record;
  packet.readings[0].occurrences[0].alternatives[0].crossLanguage.alignments = [record];
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first')); ui.open(ui.findDetails('Attributed alignments'));
  assert.match(ui.contents(), /<script>bad\(\)<\/script>/); assert.match(ui.contents(), /cyclic record/);
  assert.equal(ui.all(ui.root).filter(node => node.tag === 'script').length, 0);
});

test('SenseSnap provenance is scoped to selected sense; context pins are independent', () => {
  const ui = mount(), packet = ready();
  const atom = packet.readings[0].occurrences[0];
  atom.alternatives[1].selected = false;
  atom.senseSnap.centers.push({ origin: 'context-pin', source_mapping_asserted: false, meeting_id: 'pin-only', source_sense_ids: [] });
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first')); ui.open(ui.findDetails('sense-two'));
  const second = ui.findDetails('sense-two');
  const glyph = ui.all(second).find(node => node.attrs['aria-label']?.includes('Dictionary'));
  assert.match(glyph.attrs['aria-label'], /SenseSnap: unselected candidate/);
  assert(!ui.all(second).some(node => node.textContent.includes('snap-source')));
  assert(!ui.all(second).some(node => node.textContent.includes('pin-only')));
  assert.match(ui.contents(), /Independent SenseSnap context proposals do not assert dictionary mappings/);
  ui.open(ui.findDetails('Independent context proposals'));
  assert.match(ui.contents(), /pin-only/);
});
test('a selected late source candidate is displayed first without changing source order', () => {
  const ui = mount(), packet = ready(); const atom = packet.readings[0].occurrences[0];
  atom.alternatives = Array.from({ length: 512 }, (_, i) => ({ ...alternative(`sense-${i}`), selected: i === 511 }));
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first'));
  assert(ui.findDetails('sense-511'));
  assert.equal(atom.alternatives[0].senseId, 'sense-0');
});

test('empty dictionary candidate inventory does not connect independent context to missing source markers', () => {
  const ui = mount(), packet = ready();
  const atom = packet.readings[0].occurrences[0]; atom.alternatives = [];
  atom.senseSnap.centers = [{ origin: 'context-pin', meeting_id: 'independent-pin', source_mapping_asserted: false }];
  packet.tokenCoverage.meaning = 'covered means full atom span; source choice and E8 fit are separate';
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first'));
  const glyph = ui.all(ui.root).find(node => node.attrs['aria-label']?.includes('Dictionary'));
  assert.match(glyph.attrs['aria-label'], /SenseSnap: meeting unavailable/);
  assert.match(ui.contents(), /covered means full atom span; source choice and E8 fit are separate/);
  assert(ui.findDetails('Independent context proposals'));
});
test('native snake-case definition language remains visible', () => {
  const ui = mount(), packet = ready();
  packet.readings[0].occurrences[0].alternatives[0].dictionary.definition_language = 'eng';
  ui.api.render(ui.root, packet); ui.open(ui.findDetails('atom first'));
  assert.match(ui.contents(), /definition_language: eng/);
});
