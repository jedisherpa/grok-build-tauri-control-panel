// Cited retrieval is local. Selected evidence reaches the LLM only via Analyze.
(() => {
  'use strict';
  function coverageText(result) {
    const total = result.chunkCount || 0, vectors = result.vectorCount || 0;
    return `${total.toLocaleString()} indexed excerpts · ${vectors.toLocaleString()} local vectors · ${result.vectorStatus || 'unavailable'}. ${result.sourceFresh ? 'Sources current.' : 'Build or refresh the index before retrieving.'}`;
  }
  function selectionPayload(result, selected, query, topic) {
    if (!result?.sourceFresh || result.status !== 'ready' || !result.generation) throw new Error('Recall sources are stale; refresh the index.');
    const ids = [...selected];
    if (!ids.length || ids.length > 8 || ids.some(id => !result.hits.some(h => h.chunkId === id))) throw new Error('Select one to eight current excerpts.');
    if (!query.trim()) throw new Error('Enter the question you want Joe to clarify.');
    return { generation: result.generation, chunkIds: ids, query: query.trim(), topic: topic.trim() };
  }
  function sourceResult(value) {
    if (value?.status !== 'ready' || value.sourceFresh !== true || value.sourceIntegrity !== true || value.referenceIntegrity !== true || !value.recallBasis?.generation || value.generation !== value.recallBasis.generation || !Array.isArray(value.hits) || value.hits.length > 12 || ![value.scannedChunks,value.candidateCount,value.notReturnedCount].every(n => Number.isSafeInteger(n) && n >= 0)) throw new Error('Source lookup integrity is unavailable.');
    const hits = value.hits.map(hit => {
      if (!hit.chunkId || typeof hit.text !== 'string' || hit.sourceConcept?.usageSelection !== 'UNSELECTED' || !Array.isArray(hit.sourceConcept.occurrences) || !Array.isArray(hit.sourceConcept.distinctConceptSupport)) throw new Error('Source candidate citation is invalid.');
      return { ...hit, sourceAlias: hit.sourceConcept };
    });
    return { status: 'ready', sourceFresh: true, generation: value.recallBasis.generation, hits, sourceLookup: value };
  }
  globalThis.MemoryRecallView = Object.freeze({ coverageText, selectionPayload, sourceResult });
  if (typeof document === 'undefined') return;
  const $ = id => document.getElementById(id);
  if (!$('recall-query')) return;
  let result = null, selected = new Set(), busy = false, revision = 0, indexing = false, stopIndexing = false;
  const buttons = ['recall-search', 'recall-index', 'recall-embed', 'recall-refresh', 'recall-prepare'];
  const status = $('recall-status'), results = $('recall-results');
  const values = () => ({ query: $('recall-query').value.trim(), topic: $('recall-topic').value.trim(), source: $('recall-source').value, scope: $('recall-scope').value, limit: 12 });
  function node(tag, content, cls) { const el = document.createElement(tag); if (content != null) el.textContent = String(content); if (cls) el.className = cls; return el; }
  function updateButtons() { buttons.forEach(id => { $(id).disabled = busy || indexing || (id === 'recall-prepare' && (!selected.size || selected.size > 8 || !result)); }); $('recall-stop').disabled = !indexing || stopIndexing; }
  function announce(reason) { document.dispatchEvent(new CustomEvent('bomb-code:recall-invalidated', { detail: { reason } })); }
  function invalidate(message) { revision++; result = null; selected.clear(); results.replaceChildren(); updateButtons(); globalThis.WizardJoeGuide?.clearMemoryContext(message); announce(message); if (message) status.textContent = message; }
  function showCoverage(value) {
    status.textContent = coverageText(value);
    const excluded = Object.entries(value.exclusions || {}).map(([key, count]) => `${key}: ${count}`).join('; ');
    $('recall-coverage').textContent = `${value.historyChunks || 0} conversation excerpts; ${value.noteChunks || 0} saved-note excerpts; ${value.pendingVectors || 0} pending vectors. Derived storage: ${Math.ceil((value.databaseBytes || 0) / 1048576)} MiB of ${Math.floor((value.databaseBudgetBytes || 0) / 1048576)} MiB. Model: ${value.model?.name || 'unavailable'}; digest: ${value.model?.digest || 'unavailable'}; dimension: ${value.model?.dimension || 'not established'}. Exclusions: ${excluded || 'none recorded'}. ${JSON.stringify(value.coverage || {})}. Vector coverage can be partial; a rank is not confidence or factual verification. Imported branches may have no parent-message metadata.`;
  }
  async function operation(action, payload = {}) {
    if (busy) return;
    if (action === 'search') { announce('Recall retrieval changed.'); globalThis.WizardJoeGuide?.clearMemoryContext('Recall retrieval changed. Prepare current context again.'); }
    if (action === 'index' || action === 'embed_batch') announce('The recall index is changing.');
    const current = revision, thread = typeof state !== 'undefined' ? state.selectedSession || null : null; busy = true; updateButtons();
    status.textContent = action === 'index' ? 'Building a private local index; original histories and notes are preserved…' : action === 'embed_batch' ? 'Adding local Nomic vectors; completed batches are retained…' : 'Checking local recall…';
    try {
      const value = await invoke('memory_recall', { action, payload });
      if (current !== revision) return;
      if (thread !== (typeof state !== 'undefined' ? state.selectedSession || null : null)) { invalidate('Thread changed. Prepare current selected context again.'); return; }
      if (value.schema !== 'bomb-code/memory-recall/v1' || value.ok !== true) throw new Error(value.error || 'Unexpected recall response.');
      showCoverage(value);
      if (action === 'search') {
        if (value.status !== 'ready') throw new Error('Build or refresh the local index before searching.');
        result = value; selected.clear(); render();
        status.textContent = `${value.hits.length} ranked excerpts. ${coverageText(value)}${value.vectorError ? ` Semantic search unavailable: ${value.vectorError}.` : ''}`;
      } else if (action === 'evidence') {
        if (!value.receiptId || !value.context || !value.question) throw new Error('Prepared source receipt is missing.');
        globalThis.WizardJoeGuide?.setMemoryContext(value);
        document.dispatchEvent(new CustomEvent('bomb-code:open-joe'));
        status.textContent = 'Selected context prepared locally in Joe. Inspect it, then choose Analyze.';
      } else if (action === 'index' || action === 'embed_batch') {
        result = null; selected.clear(); results.replaceChildren();
        globalThis.WizardJoeGuide?.clearMemoryContext('The recall index changed. Prepare current context again.');
        announce('The recall index changed.');
        if (action === 'embed_batch') status.textContent += ` Added ${value.embedded || 0}; ${value.remaining ?? value.pendingVectors ?? 0} pending.`;
        if (value.embeddingError) status.textContent += ` ${value.embeddingError}`;
      } else if (!value.sourceFresh || (result && result.generation !== value.generation)) invalidate('Recall sources or index generation changed. Prepare current context again.');
      return value;
    } catch (error) {
      if (current === revision) { result = null; selected.clear(); results.replaceChildren(); globalThis.WizardJoeGuide?.clearMemoryContext('Recall is unavailable. Prepare current context again.'); status.textContent = `Recall unavailable: ${String(error)}`; }
    } finally { busy = false; updateButtons(); }
  }
  function render() {
    results.replaceChildren();
    if (!result.hits.length) { results.appendChild(node('p', 'No matching excerpts. Try different wording or broader topic/source filters.', 'muted')); return; }
    result.hits.forEach(hit => {
      const card = node('section', null, 'mem-card recall-card');
      const label = node('label', null, 'recall-select');
      const checkbox = node('input'); checkbox.type = 'checkbox'; checkbox.setAttribute('aria-label', `Select ${hit.title || hit.source} excerpt`);
      checkbox.addEventListener('change', () => { revision++; if (checkbox.checked) selected.add(hit.chunkId); else selected.delete(hit.chunkId); globalThis.WizardJoeGuide?.clearMemoryContext('Selected memory changed. Prepare the current selection again.'); announce('Selected memory changed.'); updateButtons(); });
      label.appendChild(checkbox); label.appendChild(node('span', hit.title || hit.scope || hit.source)); card.appendChild(label);
      card.appendChild(node('p', `${hit.source} · ${hit.role || 'note'} · ${hit.at || 'time unavailable'}${hit.sourceAlias ? ' · source alias candidate · UNSELECTED' : ` · keyword rank ${hit.keywordRank || '—'} · vector rank ${hit.vectorRank || '—'}`}`, 'muted'));
      card.appendChild(node('p', hit.text, 'recall-text'));
      if (hit.sourceAlias) card.appendChild(node('p', `${hit.sourceAlias.distinctConceptSupport.length} queried source concepts matched. ${hit.sourceAlias.occurrences.length} lexical occurrences retained; their senses, context and sentence roles remain unselected.`, 'muted'));
      if (hit.sourceAlias) {
        const aliases = node('details'); aliases.appendChild(node('summary', 'Inspect unselected lexical occurrences')); let filled = false;
        aliases.addEventListener('toggle', () => {
          if (!aliases.open || filled) return; filled = true;
          hit.sourceAlias.occurrences.slice(0, 24).forEach(occurrence => {
            const area = node('section'); area.appendChild(node('p', `${occurrence.surface || occurrence.normalizedForm} · excerpt span [${occurrence.span?.join(', ')}) · UNSELECTED`));
            (occurrence.alternativeSenseIds || []).slice(0, 24).forEach(id => {
              const source = result?.sourceLookup?.sourceRecords?.[id], sense = source?.sense;
              area.appendChild(node('p', sense ? `${sense.lemma} (${sense.language}): ${sense.definition || 'Definition unavailable'} · source sense ${id}` : `Source sense ${id} · inspect its dictionary record`));
            });
            if ((occurrence.alternativeSenseIds || []).length > 24) area.appendChild(node('p', 'First 24 alternatives displayed; complete alternatives remain in the source result.'));
            aliases.appendChild(area);
          });
          if (hit.sourceAlias.occurrences.length > 24) aliases.appendChild(node('p', 'First 24 occurrences displayed; all occurrence spans remain in the source result.'));
        }); card.appendChild(aliases);
      }
      const passage = node('button', 'Prepare this passage in Joe', 'btn ghost'); passage.type = 'button';
      passage.addEventListener('click', () => {
        if (busy || indexing) return;
        try {
          const payload = selectionPayload(result, new Set([hit.chunkId]), hit.text, values().topic);
          revision++; globalThis.WizardJoeGuide?.clearMemoryContext('A different passage was selected.'); announce('Selected passage changed.');
          operation('evidence', payload);
        } catch (error) { status.textContent = error.message; }
      }); card.appendChild(passage);
      const provenance = node('details'); provenance.appendChild(node('summary', 'Inspect source citation'));
      provenance.appendChild(node('p', `Thread: ${hit.threadId || 'saved note'} · message: ${hit.messageId || hit.noteId || 'unavailable'} · scope: ${hit.scope || 'historical conversation'} · Unicode span [${hit.start}, ${hit.end}) · coverage: ${hit.coverage || 'unreported'}`));
      provenance.appendChild(node('p', `Source SHA-256: ${hit.messageSha256} · excerpt SHA-256: ${hit.excerptSha256} · citation: ${hit.chunkId}`, 'recall-hash'));
      card.appendChild(provenance); results.appendChild(card);
    });
  }
  ['recall-query','recall-topic','recall-source','recall-scope'].forEach(id => $(id).addEventListener(id.includes('source') || id.includes('scope') ? 'change' : 'input', () => invalidate('Recall question or filters changed. Prepare selected context again.')));
  document.addEventListener('bomb-code:thread-selected', () => invalidate('Thread changed. Search again before preparing selected context.'));
  $('recall-search').addEventListener('click', () => operation('search', values()));
  $('recall-query').addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); operation('search', values()); } });
  $('recall-index').addEventListener('click', () => operation('index'));
  $('recall-embed').addEventListener('click', async () => {
    if (busy || indexing) return;
    indexing = true; stopIndexing = false; updateButtons();
    const startingRevision = revision;
    let basis = null, pending = null, batches = 0, batchLimit = 2000;
    try {
      while (!stopIndexing && startingRevision === revision && batches < batchLimit) {
        const value = await operation('embed_batch', { query: values().query, maxDocuments: 128 });
        batches++;
        if (!value || value.embeddingError || !value.embedded || !value.pendingVectors) break;
        const currentBasis = JSON.stringify([value.generation, value.model?.digest, value.embeddingBasis?.fingerprint]);
        if (basis !== null && (basis !== currentBasis || value.pendingVectors >= pending)) {
          status.textContent += ' Indexing stopped because the model/index changed or pending coverage did not decrease. Check coverage before resuming.';
          break;
        }
        if (basis === null) batchLimit = Math.min(2000, Math.max(1, Math.ceil((value.chunkCount || 0) / 128) + 1));
        basis = currentBasis; pending = value.pendingVectors;
      }
      if (batches >= batchLimit && pending > 0) status.textContent += ' Indexing reached its bounded batch limit. Check coverage before resuming.';
      if (stopIndexing && startingRevision === revision) status.textContent += ' Paused; completed vectors are retained. Choose Index local vectors to resume.';
    } finally { indexing = false; updateButtons(); }
  });
  $('recall-stop').addEventListener('click', () => { stopIndexing = true; updateButtons(); status.textContent += ' Pause requested; the current bounded batch will finish.'; });
  $('recall-refresh').addEventListener('click', () => operation('status'));
  $('recall-prepare').addEventListener('click', () => { try { operation('evidence', selectionPayload(result, selected, values().query, values().topic)); } catch (error) { status.textContent = error.message; } });
  globalThis.MemoryRecall = Object.freeze({
    invalidate,
    filters: values,
    showSourceCandidates(value) {
      if (busy || indexing) throw new Error('Wait for the current recall operation to finish.');
      const next = sourceResult(value);
      revision++; result = next; selected.clear(); globalThis.WizardJoeGuide?.clearMemoryContext('Source candidates changed. Prepare current context again.');
      render(); updateButtons();
      status.textContent = `${next.hits.length} unselected source alias candidates from ${value.scannedChunks.toLocaleString()} scanned excerpts. ${value.candidateCount.toLocaleString()} candidates in total; ${value.notReturnedCount.toLocaleString()} outside this page. Sentence interpretation is available only after explicit Analyze.`;
    },
    refreshStatus() {
      const prev = $('recall-scope').value;
      $('recall-scope').replaceChildren(node('option', 'All saved-note scopes'));
      $('recall-scope').firstChild.value = '';
      Array.from($('mem-scope')?.options || []).forEach(option => { const item = node('option', option.textContent); item.value = option.value; $('recall-scope').appendChild(item); });
      $('recall-scope').value = prev;
      if (!busy) operation('status');
    }
  });
})();
