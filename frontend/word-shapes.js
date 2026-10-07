/* Read-only provenance glyphs. Native coordinates are never changed by this display. */
(() => {
  'use strict';
  const NS = 'http://www.w3.org/2000/svg';
  const LIMITS = Object.freeze({ readings: 64, occurrences: 512, alternatives: 512, page: 12, tokens: 12000, text: 4000, sources: 512 });
  const array = value => Array.isArray(value) ? value : [];
  const text = (value, max = LIMITS.text) => typeof value === 'string' ? value.slice(0, max) : typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value)) ? String(value) : '';
  const object = value => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  const valueText = value => {
    if (value == null) return 'unavailable';
    if (typeof value !== 'object') return text(value) || 'unavailable';
    let budget = 256; const seen = new Set();
    const preview = (v, depth) => {
      if (--budget < 0 || depth > 4) return '[preview limited]';
      if (v == null || typeof v === 'boolean') return v;
      if (typeof v === 'number') return Number.isFinite(v) ? v : '[nonfinite number]';
      if (typeof v !== 'object') return text(v, 400);
      if (seen.has(v)) return '[cyclic record]';
      seen.add(v);
      if (Array.isArray(v)) return v.slice(0, 32).map(item => preview(item, depth + 1));
      const result = {};
      Object.keys(v).slice(0, 32).forEach(key => { Object.defineProperty(result, text(key, 120), { value: preview(v[key], depth + 1), enumerable: true }); });
      return result;
    };
    try { return JSON.stringify(preview(value, 0)).slice(0, LIMITS.text); } catch (_) { return 'unavailable record'; }
  };
  const vec8 = value => Array.isArray(value) && value.length === 8 && value.every(v => typeof v === 'number' && Number.isFinite(v)) ? value : null;
  function el(parent, tag, content, className) {
    const node = document.createElement(tag);
    if (content != null) node.textContent = text(content);
    if (className) node.className = className;
    parent.appendChild(node); return node;
  }
  function svgEl(parent, tag, attrs, content) {
    const node = document.createElementNS(NS, tag);
    Object.entries(attrs || {}).forEach(([key, value]) => node.setAttribute(key, String(value)));
    if (content != null) node.textContent = text(content);
    parent.appendChild(node); return node;
  }
  function lazyDetails(parent, title, fill, open = false) {
    const d = el(parent, 'details', null, 'word-shape-details');
    el(d, 'summary', title); let filled = false;
    const load = () => { if (!filled && d.open) { filled = true; fill(d); } };
    d.addEventListener('toggle', load);
    if (open) { d.open = true; load(); }
    return d;
  }
  function paged(parent, values, cap, fill, noun) {
    const rows = array(values), total = Math.min(rows.length, cap);
    const container = el(parent, 'div', null, 'word-shape-list');
    const button = el(parent, 'button', '', 'word-shape-more'); button.type = 'button';
    let offset = 0;
    const more = () => {
      const end = Math.min(offset + LIMITS.page, total);
      for (; offset < end; offset++) fill(container, rows[offset], offset);
      button.hidden = offset >= total;
      button.textContent = `Show next ${Math.min(LIMITS.page, total - offset)} ${noun} (${offset}/${rows.length})`;
    };
    button.addEventListener('click', more); more();
    if (rows.length > cap) el(parent, 'p', `Display limited to ${cap} ${noun}; ${rows.length - cap} further records remain in the source packet.`, 'word-shape-notice');
  }
  function fields(parent, record, keys) {
    const d = object(record);
    keys.forEach(key => { if (d[key] != null) el(parent, 'p', `${key}: ${valueText(d[key])}`, 'word-shape-coordinate'); });
  }
  function sourceRecords(parent, title, records) {
    lazyDetails(parent, `${title} (${array(records).length})`, d => {
      if (!array(records).length) el(d, 'p', 'No attributed records in this packet.', 'word-shape-missing');
      paged(d, records, LIMITS.sources, (area, record) => el(area, 'pre', valueText(record), 'word-shape-source'), 'source records');
    });
  }
  function geometryModel(activation) {
    const a = object(activation), p = object(a.placement);
    return { root: text(p.root_id || p.rootId) || 'unavailable', position: vec8(p.position8), residual: vec8(p.residual8), status: text(p.status || a.placement_status) || 'unavailable' };
  }
  function e8Profile(parent, point) {
    const svg = svgEl(parent, 'svg', { viewBox: '0 0 160 160', role: 'img', 'aria-label': 'Eight-axis fine-position display profile; atan scaling, no semantic distance or E8 rotation' });
    svg.classList.add('word-shape-profile');
    let points = [];
    for (let i = 0; i < 8; i++) {
      const angle = i * Math.PI / 4 - Math.PI / 2;
      const dx = Math.cos(angle), dy = Math.sin(angle);
      svgEl(svg, 'line', { x1: 80, y1: 80, x2: 80 + 55 * dx, y2: 80 + 55 * dy, class: 'word-shape-axis' });
      svgEl(svg, 'text', { x: 80 + 66 * dx, y: 84 + 66 * dy, 'text-anchor': 'middle' }, String(i + 1));
      const radius = 28 + 24 * Math.atan(point[i]) / (Math.PI / 2);
      points.push(`${80 + radius * dx},${80 + radius * dy}`);
    }
    svgEl(svg, 'polygon', { points: points.join(' '), class: 'word-shape-native-profile' });
    el(parent, 'p', 'Eight-axis display profile: r = 28 + 24 atan(x)/(π/2). Native E8 address stays fixed; this profile is a display mapping.', 'word-shape-caption');
  }
  function nativeGeometry(parent, e8) {
    const g = object(e8), activations = array(g.activations);
    el(parent, 'p', `Native E8: ${text(g.status) || (activations.length ? 'source activations retained' : 'unavailable')}`);
    if (!activations.length) el(parent, 'p', 'No native fine position or root address in this packet.', 'word-shape-missing');
    paged(parent, activations, LIMITS.alternatives, (area, activation) => {
      const model = geometryModel(activation);
      const d = el(area, 'section', null, 'word-shape-native');
      el(d, 'p', `Concept ${text(activation?.concept_id) || 'unreported'} · root ${model.root} · ${model.status}`);
      fields(d, activation, ['sharedConceptGloss','sharedConceptGlossLanguage','definitionsByLanguage','placementStatus','unavailableReason']);
      if (model.position) { e8Profile(d, model.position); el(d, 'p', `Fine position: [${model.position.join(', ')}]`, 'word-shape-coordinate'); }
      else el(d, 'p', 'Fine position unavailable or malformed; no profile drawn.', 'word-shape-missing');
      if (model.residual) el(d, 'p', `Root residual: [${model.residual.join(', ')}]`, 'word-shape-coordinate');
      else el(d, 'p', 'Root residual unavailable or malformed.', 'word-shape-missing');
      fields(d, object(activation).placement, ['root_index','scaled_anchor8','root_ties','radius','hierarchy_level']);
      fields(d, activation, ['source_mapping_asserted','lattice_address']);
      sourceRecords(d, 'Native activation source record (bounded preview)', [activation]);
    }, 'activations');
  }
  function stagesFor(alternative, snap, sentenceAvailable) {
    const d = object(alternative.dictionary), c = object(alternative.crossLanguage);
    const definition = text(d.definition);
    const snapRecords = array(object(snap).centers);
    return [
      { name: 'Dictionary', state: definition ? 'definition retained' : 'definition unavailable', missing: !definition },
      { name: 'Cross-language', state: text(c.status) || (array(c.alignments).length ? 'attributed alignment' : 'alignment unavailable'), missing: !array(c.alignments).length && c.status !== 'ready' },
      { name: 'SenseSnap', state: snapRecords.length ? `${snapRecords.length} attributed centers` : alternative.selected === false ? 'unselected candidate' : 'meeting unavailable', missing: !snapRecords.length },
      { name: 'Sentence use', state: sentenceAvailable ? 'proposed role / scope' : 'unavailable: no sentence', missing: !sentenceAvailable }
    ];
  }
  function glyph(parent, alternative, snap, usage, sentenceAvailable) {
    const stages = stagesFor(alternative, snap, sentenceAvailable);
    const svg = svgEl(parent, 'svg', { viewBox: '0 0 600 112', role: 'img', 'aria-label': stages.map(s => `${s.name}: ${s.state}`).join('; ') });
    svg.classList.add('word-shape-chain');
    stages.forEach((stage, index) => {
      const x = 55 + 160 * index;
      if (index) {
        svgEl(svg, 'path', { d: `M ${x - 144} 35 L ${x - 21} 35 M ${x - 28} 30 L ${x - 21} 35 L ${x - 28} 40`, class: 'word-shape-edge' });
      }
      svgEl(svg, 'circle', { cx: x, cy: 35, r: 16, class: stage.missing ? 'word-shape-marker word-shape-marker-missing' : 'word-shape-marker' });
      svgEl(svg, 'text', { x, y: 40, 'text-anchor': 'middle' }, stage.missing ? '?' : String(index + 1));
      svgEl(svg, 'text', { x, y: 72, 'text-anchor': 'middle' }, stage.name);
      svgEl(svg, 'text', { x, y: 94, 'text-anchor': 'middle', class: 'word-shape-svg-status' }, stage.state);
    });
    if (sentenceAvailable) {
      array(object(usage).arrows).slice(0, 8).forEach(arrow => {
        const a = arrow?.angleDegrees;
        if (typeof a !== 'number' || !Number.isFinite(a) || a < 0 || a >= 360) return;
        const radians = a * Math.PI / 180, x = 535 + 29 * Math.cos(radians), y = 35 + 29 * Math.sin(radians);
        const dx = Math.cos(radians), dy = Math.sin(radians);
        svgEl(svg, 'path', { d: `M 535 35 L ${x} ${y} M ${x - 6 * dx + 3 * dy} ${y - 6 * dy - 3 * dx} L ${x} ${y} L ${x - 6 * dx - 3 * dy} ${y - 6 * dy + 3 * dx}`, class: 'word-shape-usage-arrow' });
      });
    }
    el(parent, 'p', 'Marker spacing and connecting lines show provenance stages; their lengths and angles are display coordinates, not calibrated semantic distances.', 'word-shape-caption');
  }
  function usageDetails(parent, occurrence) {
    const usage = object(occurrence.usageOrientation);
    el(parent, 'p', `Token position: ${valueText(occurrence.tokenPosition)} of ${valueText(occurrence.sentenceTokenCount)} · span: ${valueText(occurrence.span)}`, 'word-shape-coordinate');
    fields(parent, occurrence, ['tokenPositions','roleBindings']);
    el(parent, 'p', 'Usage arrow angles encode the supplied proposed grammar roles in a fixed display frame. They do not rotate the native E8 address or infer meaning.', 'word-shape-caption');
    array(usage.arrows).slice(0, LIMITS.sources).forEach(arrow => el(parent, 'p', `Role ${text(arrow?.role) || 'unknown'} · event ${text(arrow?.eventId) || 'unreported'} · angle ${typeof arrow?.angleDegrees === 'number' && Number.isFinite(arrow.angleDegrees) && arrow.angleDegrees >= 0 && arrow.angleDegrees < 360 ? `${arrow.angleDegrees}°` : 'unavailable'}`));
    if (!array(usage.arrows).length) el(parent, 'p', 'No role orientation supplied.', 'word-shape-missing');
    el(parent, 'p', 'Fixed grammar legend, clockwise from right: predicate 0°, agent 45°, recipient 90°, theme/patient 135°, experiencer 225°, instrument 270°, location 315°. Unrecognized roles have no angle.', 'word-shape-caption');
    array(usage.arrows).slice(0, LIMITS.sources).forEach(arrow => fields(parent, arrow, ['eventId','polarity','modality','cueSpans']));
    fields(parent, usage, ['legend','rule','basis','scope','eventLinks','references']);
  }
  function renderAlternative(parent, alternative, occurrence, sentenceAvailable, open = false) {
    const a = object(alternative), dictionary = object(a.dictionary), c = object(a.crossLanguage);
    const attributedSnap = { ...object(occurrence.senseSnap), centers: a.selected === true ? array(object(occurrence.senseSnap).centers).slice(0, LIMITS.sources).filter(center => center?.origin === 'dictionary-source' && array(center.source_sense_ids).includes(a.senseId)) : [] };
    const selected = a.selected === true ? 'selected proposal' : a.selected === false ? 'unselected candidate' : 'selection unreported';
    lazyDetails(parent, `${text(dictionary.lemma, 100) || 'Sense'} · ${text(a.senseId) || 'unreported'} · ${selected}`, d => {
      glyph(d, a, attributedSnap, occurrence.usageOrientation, sentenceAvailable);
      el(d, 'h5', 'Dictionary marker');
      fields(d, dictionary, ['lemma','form','language','pos','definition','definitionLanguage','definition_language','definitionStatus','sourceId','sourceRecordId','source_id','source_record_id','source','status']);
      if (!text(dictionary.definition)) el(d, 'p', 'Definition unavailable in the source packet.', 'word-shape-missing');
      fields(d, a, ['senseId','conceptIds']);
      sourceRecords(d, 'Dictionary evidence', dictionary.evidenceRefs || dictionary.evidence_refs);
      el(d, 'h5', 'Cross-language marker');
      fields(d, c, ['status','meaning','counterpartsStatus','counterpartCount','counterpartsTruncated','nextCounterpartOffset']);
      sourceRecords(d, 'Attributed alignments', c.alignments);
      sourceRecords(d, 'Cross-language counterparts', c.counterparts);
      el(d, 'h5', 'SenseSnap marker');
      sourceRecords(d, 'SenseSnap centers attributed to this selected sense', attributedSnap.centers);
      el(d, 'p', 'Only dictionary-source centers naming this selected sense are joined to this candidate. Other senses and independent context proposals are shown separately.', 'word-shape-caption');
      fields(d, occurrence.senseSnap, ['schema','implementation_hashes','status']);
      el(d, 'h5', 'Sentence-use marker');
      if (sentenceAvailable) usageDetails(d, occurrence);
      else el(d, 'p', 'Sentence use unavailable until a sentence is interpreted. No role or rotation is invented for dictionary records.', 'word-shape-missing');
      el(d, 'h5', 'Fixed native geometry'); nativeGeometry(d, a.e8);
    }, open);
  }
  function occurrenceCard(parent, occurrence, index) {
    const o = object(occurrence);
    lazyDetails(parent, `${index + 1}. ${text(o.surface, 180) || 'Unnamed occurrence'} · atom ${text(o.atomId, 120) || 'unreported'} · span ${valueText(o.span)}`, d => {
      usageDetails(d, o);
      if (!array(o.alternatives).length) { glyph(d, {}, { centers: [] }, o.usageOrientation, true); el(d, 'p', 'No retained dictionary sense candidate for this occurrence.', 'word-shape-missing'); }
      const independent = array(object(o.senseSnap).centers).slice(0, LIMITS.sources).filter(center => center?.origin !== 'dictionary-source');
      if (independent.length) { el(d, 'p', 'Independent SenseSnap context proposals do not assert dictionary mappings and are not connected to dictionary candidate chains.', 'word-shape-notice'); sourceRecords(d, 'Independent context proposals', independent); }
      if (array(o.alternatives).length > LIMITS.alternatives) el(d, 'p', `Candidate display limited to ${LIMITS.alternatives} of ${array(o.alternatives).length} records; remaining records stay in the source packet.`, 'word-shape-notice');
      const alternatives = array(o.alternatives).slice(0, LIMITS.alternatives).sort((a, b) => Number(b?.selected === true) - Number(a?.selected === true));
      paged(d, alternatives, LIMITS.alternatives, (area, alternative, i) => renderAlternative(area, alternative, o, true, i === 0), 'sense alternatives');
    });
  }
  function render(parent, attachment) {
    parent.replaceChildren(); const root = el(parent, 'section', null, 'word-shapes');
    if (attachment?.schema !== 'bomb-code/word-shapes/v1' || attachment?.status !== 'ready') {
      el(root, 'p', `Word shapes unavailable: ${text(attachment?.reason || attachment?.error) || 'no current source-validated attachment'}.`, 'word-shape-missing'); return false;
    }
    el(root, 'h4', 'Traceable word shapes');
    el(root, 'p', 'Each occurrence keeps its sense alternatives and source chain. Root collisions do not merge words or meanings. Interpretations remain proposals.', 'word-shape-notice');
    el(root, 'p', `Language: ${text(attachment.language) || 'unreported'} · ${array(attachment.readings).length} proposed readings`);
    el(root, 'p', text(attachment.sentence), 'word-shape-sentence');
    const coverage = object(attachment.tokenCoverage);
    lazyDetails(root, `All sentence tokens (${array(coverage.tokens).length} supplied) · covered and uncovered`, d => {
      fields(d, coverage, ['tokenizer','meaning','sentenceTokenCount','coveredTokenCount','uncoveredTokenCount','spanUnit','status']);
      el(d, 'p', 'Tokenization is heuristic; uncovered tokens have no retained atom mapping. Repeated tokens remain separate occurrences.', 'word-shape-caption');
      paged(d, coverage.tokens, LIMITS.tokens, (area, token, i) => {
        el(area, 'p', `${i + 1}. ${text(token?.surface, 200)} · span ${valueText(token?.span)} · ${token?.covered === true ? 'covered in every reading' : 'uncovered in one or more readings'}`, token?.covered === true ? 'word-shape-token' : 'word-shape-token word-shape-missing');
        fields(area, token, ['readings']);
      }, 'tokens');
    }, true);
    paged(root, attachment.readings, LIMITS.readings, (area, reading, index) => {
      const r = object(reading);
      lazyDetails(area, `Reading ${text(r.readingId) || String(index + 1)} · ${array(r.occurrences).length} occurrences`, d => {
        el(d, 'p', 'Open an occurrence to inspect its shape and alternatives.');
        paged(d, r.occurrences, LIMITS.occurrences, occurrenceCard, 'occurrences');
      }, index === 0);
    }, 'readings');
    lazyDetails(root, 'Coordinate basis and source pins', d => fields(d, attachment.basis, Object.keys(object(attachment.basis)).slice(0, 64)));
    return true;
  }
  function dictionaryAlternative(hit) {
    const h = object(hit), concepts = array(h.concepts);
    return { senseId: h.senseId, conceptIds: concepts.slice(0, LIMITS.alternatives).map(c => c?.conceptId || c?.concept_id), dictionary: h,
      crossLanguage: { status: array(h.alignments).length ? 'attributed alignment' : 'alignment unavailable', alignments: h.alignments, counterparts: h.counterparts, counterpartCount: h.counterpartCount, counterpartsTruncated: h.counterpartsTruncated, nextCounterpartOffset: h.nextCounterpartOffset },
      e8: { status: concepts.length ? 'source placements retained' : 'unavailable', activations: concepts.slice(0, LIMITS.alternatives).map(c => ({ ...object(c), concept_id: c?.conceptId || c?.concept_id, placement: c?.placement })) } };
  }
  function renderDictionary(parent, response) {
    parent.replaceChildren(); const root = el(parent, 'section', null, 'word-shapes');
    if (response?.schema !== 'bomb-code/dictionary-shapes/v1' || response?.status !== 'ready') {
      el(root, 'p', `Dictionary shapes unavailable: ${text(response?.reason || response?.error) || 'no verified dictionary response'}.`, 'word-shape-missing'); return false;
    }
    el(root, 'h4', 'Dictionary source shapes');
    el(root, 'p', 'Dictionary and cross-language records are source-backed. SenseSnap meeting and sentence use remain unavailable until an occurrence is interpreted.', 'word-shape-notice');
    el(root, 'p', 'Cross-language counterpart lists retain candidate links separately from jointly asserted source equivalences. These are not independently validated translations.', 'word-shape-caption');
    fields(root, response.coverage, Object.keys(object(response.coverage)).slice(0, 32));
    fields(root, response, ['query','queryKind','inverseNotice','language','resultCount','returnedCount','offset','limit','truncated','hasMore','nextOffset']);
    lazyDetails(root, 'Frozen dictionary source pins', d => fields(d, response.reference, Object.keys(object(response.reference)).slice(0, 32)));
    if (!array(response.hits).length) el(root, 'p', 'No senses in this page.');
    paged(root, response.hits, 128, (area, hit, i) => renderAlternative(area, dictionaryAlternative(hit), { senseSnap: {}, usageOrientation: {} }, false, i === 0), 'dictionary senses');
    return true;
  }
  globalThis.BombWordShapes = Object.freeze({ render, renderDictionary, geometryModel, limits: LIMITS });
})();
