// Local source candidates and retained interpretation comparisons. No provider dispatch.
(() => {
  'use strict';
  const SCHEMA = 'bomb-code/meaning-memory/v1';
  const list = value => Array.isArray(value) ? value : [];
  const str = (value, maximum = 3000) => typeof value === 'string' ? value.slice(0, maximum) : '';
  const uuid = value => typeof value === 'string' && /^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(value);
  function boundary(value, schema = SCHEMA) {
    if (value?.schema !== schema || value.status !== 'ready' || ['toolsDispatched','approvalsGranted','memoryCommitted'].some(key => value.authority?.[key] !== false)) throw new Error(value?.reason || value?.error || 'Meaning-memory response is unavailable.');
    return value;
  }
  function conceptsOf(value) {
    if (value?.schema !== 'bomb-code/dictionary-shapes/v1' || value.status !== 'ready' || !Number.isSafeInteger(value.resultCount) || value.resultCount < 0 || !Array.isArray(value.hits) || value.hits.length > 20) throw new Error('Dictionary source response is invalid.');
    const choices = new Map();
    for (const hit of value.hits) for (const id of list(hit.assertedConceptIds)) {
      if (typeof id !== 'string' || !list(hit.conceptIds).includes(id)) throw new Error('Asserted source concept is invalid.');
      if (choices.has(id)) continue;
      const concept = list(hit.concepts).find(c => c.conceptId === id);
      choices.set(id, { id, lemma: str(hit.lemma), language: str(hit.language), definition: str(hit.definition) || 'Definition unavailable in this source', gloss: str(concept?.sharedConceptGloss || concept?.label), senseId: str(hit.senseId) });
    }
    return [...choices.values()];
  }
  function comparePayload(queryProfileId, candidates) {
    const ids = [...candidates];
    if (!uuid(queryProfileId) || !ids.length || ids.length > 3 || new Set(ids).size !== ids.length || ids.some(id => !uuid(id) || id === queryProfileId)) throw new Error('Choose a query and one to three different current profiles.');
    return { queryProfileId, candidateProfileIds: ids };
  }
  function distance(value) {
    if (value == null) return 'Unavailable';
    if (![value.totalVariation, value.jensenShannonDistance].every(n => Number.isFinite(n) && n >= 0 && n <= 1)) throw new Error('Comparison distance is invalid.');
    return `TV ${value.totalVariation.toFixed(3)} · √JS ${value.jensenShannonDistance.toFixed(3)}`;
  }
  function comparisonOf(value, request) {
    boundary(value);
    if (value.overallRelevanceScore !== null || typeof value.comparisonFingerprint !== 'string' || !/^[0-9a-f]{64}$/i.test(value.comparisonFingerprint) || value.query?.profileId !== request.queryProfileId || !Array.isArray(value.cards) || value.cards.length !== request.candidateProfileIds.length) throw new Error('Comparison identity or provenance is invalid.');
    const ids = value.cards.map(c => c.candidateProfileId);
    if (new Set(ids).size !== ids.length || ids.some(id => !request.candidateProfileIds.includes(id))) throw new Error('Comparison candidates changed.');
    let pairs = 0;
    for (const card of value.cards) {
      if (!Array.isArray(card.readingPairs)) throw new Error('Retained reading pairs are unavailable.');
      pairs += card.readingPairs.length;
      if (pairs > 4096) throw new Error('Comparison reading budget exceeded.');
      for (const pair of card.readingPairs) for (const key of ['sourceSenseDistance','assertedConceptDistance','senseSnapDictionaryDistance','contextPinDistance','eventLocalDistance']) distance(pair[key]);
    }
    if (!Array.isArray(value.unsentQuestions) || value.unsentQuestions.length > 48 || value.unsentQuestions.some(q => q.unsent !== true || !str(q.text) || q.text.length > 4000 || !ids.includes(q.candidateProfileId))) throw new Error('Unsent question budget or provenance is invalid.');
    return value;
  }
  function preview(value) {
    let remaining = 96;
    function visit(v, depth) {
      if (--remaining < 0 || depth > 4) return '[preview limited]';
      if (v == null || typeof v === 'boolean') return v;
      if (typeof v === 'number') return Number.isFinite(v) ? v : 'unavailable';
      if (typeof v !== 'object') return str(v, 300);
      if (Array.isArray(v)) return v.slice(0, 12).map(x => visit(x, depth + 1));
      return Object.fromEntries(Object.entries(v).slice(0, 16).map(([k,x]) => [k,visit(x,depth+1)]));
    }
    return JSON.stringify(visit(value, 0), null, 2).slice(0, 6000);
  }
  globalThis.MeaningMemoryView = Object.freeze({ conceptsOf, comparePayload, comparisonOf, distance, preview });
  if (typeof document === 'undefined' || !document.getElementById('meaning-memory')) return;
  const $ = id => document.getElementById(id);
  const thread = () => typeof state !== 'undefined' ? state.selectedSession || null : null;
  let revision = 0, busy = false, dictionaryOffset = 0, dictionaryCount = 0, profileOffset = 0, nextProfileOffset = null;
  let latestJoe = null, comparison = null, queuedCatalog = false, importNotice = '';
  const selectedConcepts = new Set(), selectedProfiles = new Set(), profiles = new Map();
  const controls = ['meaning-dictionary-search','meaning-source-search','meaning-refresh','meaning-import','meaning-compare','meaning-dictionary-prev','meaning-dictionary-next','meaning-profile-prev','meaning-profile-next'];
  function node(tag, text, cls) { const el = document.createElement(tag); if (text != null) el.textContent = String(text); if (cls) el.className = cls; return el; }
  function paragraph(parent, text, cls = 'muted') { parent.appendChild(node('p', text, cls)); }
  function buttons() {
    controls.forEach(id => { $(id).disabled = busy; });
    $('meaning-source-search').disabled = busy || !selectedConcepts.size;
    $('meaning-compare').disabled = busy || !uuid($('meaning-query-profile').value) || !selectedProfiles.size || selectedProfiles.has($('meaning-query-profile').value);
    $('meaning-dictionary-prev').disabled = busy || dictionaryOffset === 0;
    $('meaning-dictionary-next').disabled = busy || dictionaryOffset + 20 >= dictionaryCount;
    $('meaning-profile-prev').disabled = busy || profileOffset === 0;
    $('meaning-profile-next').disabled = busy || nextProfileOffset == null;
  }
  function invalidate(reason) { revision++; comparison = null; $('meaning-comparisons').replaceChildren(); if (reason) $('meaning-comparison-status').textContent = reason; buttons(); }
  const snapshot = () => JSON.stringify([thread(), $('meaning-word').value, $('meaning-language').value, globalThis.MemoryRecall?.filters(), $('meaning-query-profile').value, [...selectedProfiles], [...selectedConcepts], $('joe-passage')?.value, $('joe-language')?.value]);
  async function perform(action, payload, accept, area = 'meaning-profile-status') {
    if (busy) return;
    const current = ++revision, selectedThread = thread(), inputs = snapshot(); busy = true; buttons();
    $(area).textContent = 'Checking source-bound memory locally…';
    try {
      const result = boundary(await invoke('meaning_memory', { action, payload }), action === 'source_candidates' ? 'bomb-code/source-concept-candidates/v1' : SCHEMA);
      if (current !== revision || selectedThread !== thread() || inputs !== snapshot()) return;
      accept(result);
    } catch (error) { if (current === revision && selectedThread === thread()) { invalidate(); $(area).textContent = `Unavailable: ${String(error)}`; } }
    finally { busy = false; buttons(); if (queuedCatalog) { queuedCatalog = false; catalog(0); } }
  }
  function clearConcepts() {
    selectedConcepts.clear(); dictionaryOffset = 0; dictionaryCount = 0; $('meaning-concepts').replaceChildren();
    invalidate('Dictionary input changed. Choose current source concepts.');
    globalThis.MemoryRecall?.invalidate('Source meaning changed. Search for current passages.');
  }
  async function lookup(offset = 0) {
    if (busy) return;
    const query = $('meaning-word').value.trim(), language = $('meaning-language').value;
    if (!query || query.length > 256) { $('meaning-source-status').textContent = 'Enter a word or definition of at most 256 characters.'; return; }
    selectedConcepts.clear(); $('meaning-concepts').replaceChildren(); globalThis.MemoryRecall?.invalidate('Source meanings are changing.');
    const current = ++revision, selectedThread = thread(), inputs = snapshot(); busy = true; buttons();
    try {
      const result = await invoke('word_shape_dictionary', { payload: { action: 'query', query, language, offset, limit: 20 } });
      if (current !== revision || selectedThread !== thread() || inputs !== snapshot()) return;
      const choices = conceptsOf(result); dictionaryOffset = offset; dictionaryCount = result.resultCount;
      for (const choice of choices.slice(0, 40)) {
        const label = node('label', null, 'meaning-choice'), box = node('input'); box.type = 'checkbox';
        box.addEventListener('change', () => {
          if (box.checked && selectedConcepts.size >= 8) { box.checked = false; $('meaning-source-status').textContent = 'Choose at most eight source concepts.'; return; }
          if (box.checked) selectedConcepts.add(choice.id); else selectedConcepts.delete(choice.id);
          invalidate('Selected source concepts changed. Search their forms again.'); globalThis.MemoryRecall?.invalidate('Selected source concepts changed.');
        });
        label.appendChild(box); label.appendChild(node('span', `${choice.lemma} (${choice.language}): ${choice.definition}${choice.gloss && choice.gloss !== choice.definition ? ` · ${choice.gloss}` : ''}`)); $('meaning-concepts').appendChild(label);
      }
      $('meaning-source-status').textContent = `${result.resultCount} source senses found; ${choices.length} distinct asserted concepts on this page. ${choices.length > 40 ? 'First 40 choices displayed; narrow the wording for additional choices. ' : ''}${choices.length ? 'Choose up to eight concepts to find their encoded forms.' : 'No asserted equivalent concept is available on this page. Other source alternatives remain in the dictionary viewer.'}`;
    } catch (error) { if (current === revision && selectedThread === thread()) { dictionaryCount = 0; $('meaning-source-status').textContent = `Dictionary unavailable: ${String(error)}`; } }
    finally { busy = false; buttons(); if (queuedCatalog) { queuedCatalog = false; catalog(0); } }
  }
  function catalog(offset = 0) {
    invalidate('Profile coverage is changing. Compare the current selection again.');
    return perform('status', { offset, limit: 20 }, value => {
      if (!Array.isArray(value.profiles) || value.profiles.length > 20 || !Number.isSafeInteger(value.counts?.total) || value.counts.total < 0 || (value.nextOffset !== null && (!Number.isSafeInteger(value.nextOffset) || value.nextOffset <= offset))) throw new Error('Profile coverage response is invalid.');
      profileOffset = offset; nextProfileOffset = value.nextOffset;
      const previousQuery = $('meaning-query-profile').value;
      const retained = new Set([...selectedProfiles, previousQuery, latestJoe?.profileId]);
      for (const id of profiles.keys()) if (!retained.has(id)) profiles.delete(id);
      for (const profile of value.profiles) { if (!uuid(profile.profileId) || typeof profile.available !== 'boolean' || typeof profile.sentence !== 'string') throw new Error('Profile identity is invalid.'); profiles.set(profile.profileId, profile); }
      const picker = $('meaning-query-profile'); picker.replaceChildren(node('option', 'Choose a saved proposal or analyze a passage in Joe')); picker.firstChild.value = '';
      for (const profile of profiles.values()) {
        const option = node('option', `${profile.profileId === latestJoe?.profileId ? 'Current Joe review · ' : ''}${profile.kind || 'proposal'} · ${str(profile.sentence, 120)}`); option.value = profile.profileId; option.disabled = !profile.available; picker.appendChild(option);
      }
      picker.value = profiles.get(previousQuery)?.available ? previousQuery : profiles.get(latestJoe?.profileId)?.available ? latestJoe.profileId : '';
      $('meaning-profiles').replaceChildren();
      for (const profile of value.profiles) {
        const card = node('section', null, 'meaning-profile'), label = node('label', null, 'meaning-choice'), box = node('input'); box.type = 'checkbox'; box.checked = selectedProfiles.has(profile.profileId); box.disabled = !profile.available;
        box.addEventListener('change', () => {
          if (box.checked && selectedProfiles.size >= 3) { box.checked = false; $('meaning-comparison-status').textContent = 'Compare at most three candidate profiles at once.'; return; }
          if (box.checked) selectedProfiles.add(profile.profileId); else selectedProfiles.delete(profile.profileId);
          invalidate('Comparison profile choices changed. Compare again.');
        });
        label.appendChild(box); label.appendChild(node('span', `${profile.kind || 'proposal'} · ${list(profile.citationIds).length} citations · ${profile.available ? 'source checks current' : 'withheld'}`)); card.appendChild(label);
        paragraph(card, str(profile.sentence), 'meaning-passage'); paragraph(card, profile.available ? `Saved ${profile.createdAt || 'time unavailable'} · thread ${profile.threadId || 'unscoped'}` : str(profile.reason) || 'Source checks are unavailable.');
        const inspect = node('details'); inspect.appendChild(node('summary', 'Inspect proposal identity and citations')); paragraph(inspect, `Request ${profile.requestId || profile.profileId} · citations ${list(profile.citationIds).join(', ') || 'none'} · model-proposed interpretation`); card.appendChild(inspect); $('meaning-profiles').appendChild(card);
        const shapes = node('button', 'Inspect saved word shapes in Joe', 'btn ghost'); shapes.type = 'button'; shapes.disabled = !profile.available || !profile.threadId || profile.threadId !== thread();
        shapes.addEventListener('click', () => {
          if (profile.threadId !== thread() || !profile.available || !$('word-replay-id') || !$('word-replay-show')) return;
          $('word-replay-id').value = profile.requestId || profile.profileId; $('word-replay-id').dispatchEvent(new Event('input', { bubbles: true }));
          document.dispatchEvent(new CustomEvent('bomb-code:open-joe')); $('word-replay-show').click();
        }); card.appendChild(shapes);
        if (shapes.disabled) paragraph(card, 'Select the original thread to inspect this saved proposal in the existing word-shape viewer.');
      }
      const checked = value.profiles.filter(p => p.available).length;
      $('meaning-profile-status').textContent = `${importNotice}${value.counts.total} stored proposals; ${checked}/${value.profiles.length} on this page pass current source checks. ${value.counts.unchecked ?? Math.max(0, value.counts.total - value.profiles.length)} remain unchecked by this page. Passage/question counts describe stored subjects; they do not measure meaning accuracy. All other history remains unannotated.`;
      buttons();
    });
  }
  function evidence(parent, title, value) {
    const d = node('details'); d.appendChild(node('summary', title)); let loaded = false;
    d.addEventListener('toggle', () => { if (d.open && !loaded) { loaded = true; d.appendChild(node('pre', preview(value), 'meaning-evidence')); } }); parent.appendChild(d);
  }
  function renderComparison(value, request, savedRevision, inputs, selectedThread) {
    const output = $('meaning-comparisons'); output.replaceChildren();
    const current = () => comparison === value && revision === savedRevision && inputs === snapshot() && selectedThread === thread();
    for (const card of value.cards) {
      const section = node('section', null, 'meaning-comparison'); paragraph(section, str(profiles.get(card.candidateProfileId)?.sentence) || 'Compared saved proposal', 'meaning-passage');
      let shown = 0; const rows = node('div'); section.appendChild(rows);
      const more = node('button', 'Inspect reading pairs', 'btn ghost'); more.type = 'button';
      const append = () => {
        if (!current()) return;
        const end = Math.min(shown + 8, card.readingPairs.length, 128);
        for (; shown < end; shown++) {
          const pair = card.readingPairs[shown], d = node('details'); d.appendChild(node('summary', `Reading ${pair.queryReadingId} compared with ${pair.candidateReadingId}`));
          const signals = node('ul', null, 'meaning-signals');
          for (const [name,key] of [['Selected source senses','sourceSenseDistance'],['Asserted source concepts','assertedConceptDistance'],['Dictionary SenseSnap centers','senseSnapDictionaryDistance'],['Scoped context pins','contextPinDistance'],['Predicate and attached roles / polarity / modality','eventLocalDistance']]) signals.appendChild(node('li', `${name}: ${distance(pair[key])}`));
          d.appendChild(signals); paragraph(d, 'Distances describe retained proposal distributions. Inspect event attachments and source evidence to interpret each difference.');
          paragraph(d, `Event links and references: ${pair.canonicalLinksAndReferencesEqual ? 'same canonical signatures' : 'different signatures'}. Graph isomorphism remains unestablished.${pair.signatureCollisions || pair.atomIdentityCollisions ? ' Repeated event or occurrence signatures are present; inspect original endpoint attachments.' : ''}`);
          const queryReading = list(value.queryReadings).find(r => r.readingId === pair.queryReadingId);
          const candidateReading = list(list(value.candidateReadings).find(p => p.profileId === card.candidateProfileId)?.readings).find(r => r.readingId === pair.candidateReadingId);
          for (const [label,frame,reading] of [['Query',pair.queryFrame,queryReading],['Comparison',pair.candidateFrame,candidateReading]]) {
            const surfaces = new Map(list(reading?.occurrences).map(a => [a.atomId,str(a.surface,100)]));
            for (const event of list(frame?.events).slice(0, 8)) paragraph(d, `${label} ${surfaces.get(event.predicate) || event.predicate}: ${list(event.roles).slice(0,12).map(role => `${role.role}=${surfaces.get(role.atom_id) || role.atom_id}`).join(', ')} · ${event.polarity} · ${event.modality}`);
          }
          evidence(d, 'Selected dictionary definitions and occurrence spans', { query: queryReading, comparison: candidateReading });
          evidence(d, 'Scoped context and source-center provenance', { centers: pair.contextCenters, policy: pair.contextPolicy });
          paragraph(d, `Fine geometry: ${pair.geometry?.correspondenceCount ?? 0} correspondences; ${Array.isArray(pair.geometry?.rootCollisions) ? pair.geometry.rootCollisions.length : pair.geometry?.rootCollisions ?? 0} same-root collisions. Root sharing does not select a meaning.`);
          evidence(d, 'Event roles, polarity and modality evidence', pair.eventDifferences); evidence(d, 'Fine positions and root residual comparisons', pair.geometry); evidence(d, 'Coverage, occurrence multiplicity and uncertainty', { coverage: pair.coverage, multiplicity: pair.multiplicity, uncertainty: pair.uncertainty }); rows.appendChild(d);
        }
        more.hidden = shown >= card.readingPairs.length || shown >= 128; more.textContent = `Inspect next reading pairs (${shown}/${card.readingPairs.length})`;
      };
      more.addEventListener('click', append); section.appendChild(more); append();
      if (card.readingPairs.length > 128) paragraph(section, 'Display is bounded to 128 reading pairs. Complete retained comparisons remain in the native result; narrow the profile selection for inspection.');
      evidence(section, 'Original profile and citation provenance', card.provenance); output.appendChild(section);
    }
    const drafts = node('section', null, 'meaning-questions'); drafts.appendChild(node('h4', 'Unsent clarification drafts'));
    for (const question of value.unsentQuestions) {
      const d = node('section', null, 'meaning-question'); paragraph(d, question.text, 'meaning-passage'); evidence(d, 'Original reading / occurrence / event basis', question.basis);
      const copy = node('button', 'Add question to unsent message', 'btn ghost'); copy.type = 'button';
      copy.addEventListener('click', async () => {
        if (!current() || copy.disabled) return;
        copy.disabled = true;
        try {
          const copied = await globalThis.WizardJoeGuide?.draftMemoryQuestion({ question: question.text, threadId: selectedThread, isCurrent: current, validate: async () => {
            const checked = boundary(await invoke('meaning_memory', { action: 'validate', payload: { ...request, comparisonFingerprint: value.comparisonFingerprint } }));
            if (checked.comparisonFingerprint !== value.comparisonFingerprint || !current()) throw new Error('Comparison source identity changed.');
          } });
          if (current()) $('meaning-comparison-status').textContent = copied ? 'Question added to the unsent message. Review it before sending.' : 'Draft withheld. Prepare a current comparison for this thread.';
        } catch (error) { if (current()) invalidate(`Draft withheld: ${String(error)}`); }
        finally { copy.disabled = !current(); }
      }); d.appendChild(copy); drafts.appendChild(d);
    }
    if (!value.unsentQuestions.length) paragraph(drafts, 'No draft was produced from the retained differences. This does not establish that all questions are answered.'); output.appendChild(drafts);
  }
  $('meaning-dictionary-form').addEventListener('submit', event => { event.preventDefault(); lookup(0); });
  $('meaning-word').addEventListener('input', clearConcepts); $('meaning-language').addEventListener('change', clearConcepts);
  $('meaning-dictionary-prev').addEventListener('click', () => lookup(Math.max(0, dictionaryOffset - 20))); $('meaning-dictionary-next').addEventListener('click', () => { if (dictionaryOffset + 20 < dictionaryCount) lookup(dictionaryOffset + 20); });
  $('meaning-source-search').addEventListener('click', () => {
    if (!selectedConcepts.size || busy) return;
    globalThis.MemoryRecall?.invalidate('Searching encoded source forms locally…');
    const filters = globalThis.MemoryRecall?.filters() || {};
    perform('source_candidates', { conceptIds: [...selectedConcepts], source: filters.source || '', scope: filters.scope || '', limit: 12 }, value => {
      globalThis.MemoryRecall.showSourceCandidates(value); $('meaning-source-status').textContent = 'Unselected source candidates appear in the cited recall cards above. Prepare one passage in Joe to analyze its sentence use.';
    }, 'meaning-source-status');
  });
  $('meaning-refresh').addEventListener('click', () => catalog(0));
  $('meaning-import').addEventListener('click', () => { invalidate('Importing saved proposals changes profile coverage.'); perform('import', {}, value => { importNotice = `Last import: ${value.imported ?? 0} imported; ${value.existing ?? 0} already present; ${list(value.withheld).length} withheld. ${list(value.withheld).slice(0,3).map(row => str(row.reason,200)).join('; ')}${list(value.withheld).length > 3 ? ' (first three reasons shown)' : ''} Original saved proposals are preserved. `; $('meaning-profile-status').textContent = importNotice; queuedCatalog = true; }); });
  $('meaning-profile-prev').addEventListener('click', () => catalog(Math.max(0, profileOffset - 20))); $('meaning-profile-next').addEventListener('click', () => { if (nextProfileOffset != null) catalog(nextProfileOffset); });
  $('meaning-query-profile').addEventListener('change', () => invalidate('Query profile changed. Compare the current choice again.'));
  $('meaning-compare').addEventListener('click', () => {
    let request; try { request = comparePayload($('meaning-query-profile').value, selectedProfiles); } catch (error) { $('meaning-comparison-status').textContent = error.message; return; }
    invalidate('Comparison pending.');
    perform('compare', request, value => {
      comparison = comparisonOf(value, request); renderComparison(value, request, revision, snapshot(), thread());
      $('meaning-comparison-status').textContent = `${value.cards.length} profiles compared through their retained readings. Separate signals and proposed questions are ready for inspection; no overall relevance score is assigned.`;
    }, 'meaning-comparison-status');
  });
  document.addEventListener('bomb-code:recall-invalidated', event => invalidate(event.detail?.reason || 'Recall input or sources changed.'));
  document.addEventListener('bomb-code:thread-selected', () => { latestJoe = null; invalidate('Thread changed. Compare again before drafting.'); });
  document.addEventListener('bomb-code:joe-context-changed', () => invalidate('Joe context changed. Compare current profiles again.'));
  for (const id of ['joe-passage','joe-language']) $(id)?.addEventListener('input', () => { latestJoe = null; invalidate('Joe passage changed. Compare again.'); });
  document.addEventListener('bomb-code:joe-interpretation', event => {
    invalidate('Joe review changed. Compare current source-bound profiles.');
    const result = event.detail?.result;
    latestJoe = result?.meaningProfile?.status === 'ready' && uuid(result.meaningProfile.profileId) ? { profileId: result.meaningProfile.profileId, requestId: result.requestId } : null;
    if (latestJoe) profiles.set(latestJoe.profileId, { profileId: latestJoe.profileId, requestId: result.requestId, sentence: result.sentence, language: result.language, threadId: result.threadId, kind: result.meaningProfile.kind, citationIds: list(result.memoryEvidence?.citationIds), available: true });
    if (latestJoe) { if (busy) queuedCatalog = true; else catalog(0); }
  });
  buttons();
})();
