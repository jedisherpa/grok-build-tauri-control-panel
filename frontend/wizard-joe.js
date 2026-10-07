// Manual, read-only interpretation. Geometry receives typed provenance;
// guide buttons never send prompts or resolve native coding approvals.
(() => {
  "use strict";
  const list = (value) => Array.isArray(value) ? value : [];
  const text = (value) => typeof value === "string" ? value : "";
  function viewOf(result) {
    if (result?.schema !== "bomb-code/joe-result/v1") throw new Error("Unexpected guide response schema.");
    if (result.authority?.toolsDispatched !== false || result.authority?.approvalsGranted !== false || result.authority?.memoryCommitted !== false) throw new Error("Guide boundary receipt is missing or unexpected.");
    const binding = result.interpretation?.binding || {};
    return {
      readings: list(binding.readings),
      clarifications: list(result.clarifications).filter(q => text(q.question)),
      receipt: binding.receipt || {},
      execution: result.interpretation?.execution || {},
    };
  }
  function appendDraft(existing, question) {
    if (!text(question).trim()) return existing;
    return `${existing}${existing.trim() ? "\n\n" : ""}${question}`;
  }
  function sameInput(result, sentence, language, threadId) {
    return result?.sentence === sentence && result?.language === language && (result.threadId || null) === (threadId || null);
  }
  function continuityView(attachment) {
    const s = attachment?.state;
    if (attachment?.status !== "ready" || s?.schema !== "bomb-code/cdiss-state/v1" || s?.algorithmVersion !== "bomb-code/cdiss-source-structure/v1") return { available: false, reason: text(attachment?.reason) || "Continuity observation is unavailable for this review." };
    const c = s.continuity || {}, o = s.observation || {};
    if (![o.readingCount, o.atomCount, o.eventCount, o.alternativeCount, o.mappedMass, o.unmappedMass].every(Number.isFinite)) return { available: false, reason: "Continuity observation has invalid numeric fields." };
    const distances = [c.sourceDistance, c.structureDistance].filter(Boolean);
    if (distances.some(d => ![d.totalVariation, d.jensenShannonDistance].every(v => Number.isFinite(v) && v >= 0 && v <= 1))) return { available: false, reason: "Continuity distances are unavailable." };
    return { available: true, status: text(c.status), reasons: list(c.reasons).filter(v => typeof v === "string"), source: c.sourceDistance, structure: c.structureDistance, partitionChanged: c.partitionChanged, observation: o, state: s, ignoredReason: text(attachment.comparisonIgnoredReason) };
  }
  globalThis.WizardJoeView = Object.freeze({ viewOf, appendDraft, sameInput, continuityView });
  if (typeof document === "undefined") return;
  const byId = id => document.getElementById(id);
  const guide = byId("wizard-joe");
  if (!guide) return;
  const passage = byId("joe-passage"), language = byId("joe-language");
  const status = byId("joe-result-status"), output = byId("joe-result");
  const analyze = byId("joe-analyze");
  const compare = byId("joe-compare"), compareLabel = byId("joe-compare-label"), example = byId("joe-cdiss-example");
  const previousReviews = new Map();
  let busy = false, generation = 0, lastResult = null, showingExample = false;
  let memoryReview = null;
  const currentThread = () => typeof state !== "undefined" ? state.selectedSession || null : null;
  function updateComparison() {
    const prior = previousReviews.get(currentThread());
    if (compare) { compare.disabled = busy || !prior; if (!prior) compare.checked = false; }
    if (compareLabel) compareLabel.textContent = prior ? `Compare with previous review ${prior.slice(0, 8)} in this thread` : "Compare with a previous review in this thread when available";
  }
  let contextValid = () => true;
  function node(tag, content, cls) {
    const el = document.createElement(tag);
    if (content != null) el.textContent = String(content);
    if (cls) el.className = cls;
    return el;
  }
  function paragraph(parent, content, cls = "joe-notice") {
    if (content != null && String(content)) parent.appendChild(node("p", content, cls));
  }
  function details(parent, title) {
    const d = node("details", null, "joe-details");
    d.appendChild(node("summary", title)); parent.appendChild(d); return d;
  }
  function strings(parent, values, title) {
    const rows = list(values).filter(v => typeof v === "string" && v);
    if (!rows.length) return;
    paragraph(parent, title, "joe-label");
    const ul = node("ul"); rows.forEach(v => ul.appendChild(node("li", v))); parent.appendChild(ul);
  }
  const memoryArea = node("details", null, "joe-details");
  memoryArea.hidden = true; memoryArea.open = true; guide.appendChild(memoryArea);
  function clearMemoryContext(reason) {
    if (!memoryReview) return;
    memoryReview = null; memoryArea.hidden = true; memoryArea.replaceChildren();
    invalidate();
    if (reason) status.textContent = reason;
  }
  async function checkService() {
    try {
      const service = await invoke("joe_status");
      byId("joe-service-status").textContent = service.available
        ? `Manual interpretation · ${service.provider || "unreported provider"} · ${service.model || "unreported model"}. Model readings remain proposals.`
        : `Interpretation unavailable: ${service.reason || "Reference or provider is not ready."}`;
      analyze.disabled = !service.available || busy;
    } catch (error) {
      byId("joe-service-status").textContent = `Interpretation service could not be checked: ${String(error)}`;
      analyze.disabled = true;
    }
  }
  guide.addEventListener("toggle", () => { if (guide.open && !busy) checkService(); });
  byId("joe-refresh-service").addEventListener("click", () => { if (!busy) checkService(); });
  document.addEventListener("bomb-code:thread-selected", () => {
    clearMemoryContext("Thread changed. Prepare selected memory again for this thread.");
    generation++; lastResult = null; showingExample = false; output.replaceChildren();
    if (compare) compare.checked = false;
    updateComparison();
    status.textContent = "Thread changed. Your passage and unsent message are preserved; analyze again for this thread.";
    guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: "invalidated", reason: "thread-changed", result: null } }));
  });
  function invalidate() {
    if (memoryReview && memoryReview.question !== passage.value) {
      memoryReview = null; memoryArea.hidden = true; memoryArea.replaceChildren();
    }
    generation++;
    if (lastResult || showingExample) {
      lastResult = null; showingExample = false;
      output.replaceChildren();
      status.textContent = "Passage or language changed. Analyze again for a current reading.";
      guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: "invalidated", result: null } }));
    }
  }
  if (compare) compare.addEventListener("change", invalidate);
  passage.addEventListener("input", invalidate);
  language.addEventListener("input", invalidate);
  byId("joe-copy-composer").addEventListener("click", () => {
    clearMemoryContext();
    passage.value = byId("prompt").value;
    invalidate();
    status.textContent = "Copied the unsent message. Nothing has been sent.";
    passage.focus();
  });
  globalThis.WizardJoeGuide = Object.freeze({
    setContextValidator(validate) { contextValid = typeof validate === "function" ? validate : () => true; },
    setPassage(value, message) { clearMemoryContext(); invalidate(); passage.value = value; status.textContent = message || "Passage prepared locally."; },
    clearMemoryContext,
    setMemoryContext(prepared) {
      if (!text(prepared?.receiptId) || !text(prepared.question) || prepared.context?.schema !== "bomb-code/recalled-evidence/v1" || !list(prepared.context.evidence).length) throw new Error("Prepared recall context is invalid.");
      clearMemoryContext(); invalidate(); passage.value = prepared.question;
      memoryReview = { receiptId: prepared.receiptId, question: prepared.question };
      if (compare) compare.checked = false;
      memoryArea.replaceChildren(node("summary", "Selected memory context · included only when you Analyze"));
      paragraph(memoryArea, `Topic: ${text(prepared.context.topic) || "Not narrowed by topic"}. ${prepared.context.notice}`);
      prepared.context.evidence.forEach(item => {
        const source = details(memoryArea, `${text(item.title) || text(item.source)} · ${text(item.role) || "note"}`);
        paragraph(source, item.text, "joe-source-text");
        paragraph(source, `Citation: ${item.citationId} · source: ${item.source} · thread: ${item.threadId || "saved note"} · message: ${item.messageId || item.noteId} · span: [${item.span?.start}, ${item.span?.end}) · coverage: ${item.coverage}`);
      });
      memoryArea.hidden = false; memoryArea.open = true; guide.open = true;
      status.textContent = "Question and selected historical context prepared locally. Analyze sends both to the displayed provider; sources are checked again first.";
    },
    invalidateContext(reason) {
      clearMemoryContext();
      generation++; lastResult = null; showingExample = false; output.replaceChildren();
      status.textContent = reason;
      guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: "invalidated", reason: "context-changed", result: null } }));
    }
  });
  function renderAtom(parent, record) {
    const atom = record.atom || {};
    const chosen = list(record.selected_source_bindings);
    const candidates = list(record.all_source_candidates);
    const area = details(parent, `${text(atom.surface) || atom.id || "Atom"} · ${chosen.length} selected source senses · ${candidates.length} candidates retained`);
    if (record.source_grounding_unavailable) paragraph(area, "No selected source grounding. This does not establish that your intended meaning is absent.");
    paragraph(area, record.selection_reason);
    if (Array.isArray(atom.span)) paragraph(area, `Exact span: ${atom.span.join("–")} · Python Unicode character offsets, half-open.`);
    chosen.forEach(candidate => {
      const sense = candidate.sense || {};
      const d = details(area, `${sense.id || "Source sense"} · ${candidate.binding_kind || "source binding"}`);
      paragraph(d, sense.definition || sense.gloss, "joe-source-text");
      strings(d, sense.examples, "Native examples");
      paragraph(d, `Language: ${sense.language || "unreported"} · source: ${sense.source_id || sense.source || "see receipt"}`);
      strings(d, candidate.concept_ids, "Source concept IDs");
    });
    const centers = list(record.sense_snap?.centers);
    centers.forEach(center => {
      const d = details(area, `${center.meeting_id || "Center"} · ${center.origin || "unreported origin"}`);
      paragraph(d, center.source_mapping_asserted ? "Existing asserted source mapping." : "No new dictionary equivalence asserted.");
      paragraph(d, center.authority);
      paragraph(d, `Complete relation ring: ${center.ring_total ?? "unreported"} entries · bounded preview is not the full ring.`);
      if (center.origin === "context-pin" && !center.display) paragraph(d, "Context center has no fitted display point.");
    });
    if (candidates.length) {
      const retained = details(area, "Inspect retained source alternatives");
      candidates.forEach(candidate => {
        const sense = candidate.sense || {};
        const d = details(retained, `${sense.id || "Source candidate"} · ${candidate.binding_kind || "candidate"}`);
        paragraph(d, sense.definition || sense.gloss, "joe-source-text");
        strings(d, sense.examples, "Native examples");
      });
    }
  }
  function render(result) {
    const view = viewOf(result);
    output.replaceChildren();
    paragraph(output, `Model-proposed reading · ${result.provider || "unreported provider"} · ${result.model || "unreported model"}.`, "joe-label");
    if (result.error) paragraph(output, result.error, "joe-error");
    if (result.interpretation?.error) paragraph(output, result.interpretation.error, "joe-error");
    if (result.interpretation?.failed_stage) paragraph(output, `Interpretation stopped at stage: ${result.interpretation.failed_stage}`, "joe-error");
    paragraph(output, "This passage review does not determine which requirements or questions are missing from the whole project.");
    renderContinuity(output, result.cdiss);
    view.clarifications.forEach(question => {
      const section = node("section", null, "joe-question");
      paragraph(section, question.question, "joe-question-text");
      paragraph(section, question.reason);
      paragraph(section, `Origin: ${question.origin || "guide-proposed"} · readings: ${list(question.readingIds).join(", ") || "unreported"} · atoms: ${list(question.atomIds).join(", ") || "unreported"}`);
      const button = node("button", "Add question to unsent message", "btn ghost"); button.type = "button";
      button.addEventListener("click", async () => {
        if (lastResult !== result || !sameInput(result, passage.value, language.value.trim(), result.threadId)) return;
        if ((typeof state !== "undefined" ? state.selectedSession || null : null) !== (result.threadId || null)) { status.textContent = "This review belongs to another thread. Select that thread or analyze again before drafting."; return; }
        if (!contextValid(result.sentence, result.threadId || null)) { status.textContent = "Thread context changed. Prepare a current review before drafting this question."; return; }
        if (result.memoryEvidence?.receiptId) {
          try { await invoke("memory_recall", { action: "validate", payload: { receiptId: result.memoryEvidence.receiptId } }); }
          catch (error) { clearMemoryContext("Selected evidence changed. Prepare a current review before drafting."); return; }
          if (lastResult !== result || !sameInput(result, passage.value, language.value.trim(), currentThread()) || !contextValid(result.sentence, result.threadId || null)) return;
        }
        const composer = byId("prompt");
        composer.value = appendDraft(composer.value, question.question);
        composer.dispatchEvent(new Event("input", { bubbles: true }));
        status.textContent = "Question added to your unsent message. Review it before sending.";
        composer.focus();
      }); section.appendChild(button); output.appendChild(section);
    });
    if (!view.clarifications.length) paragraph(output, "No clarification proposal returned for this passage. This is not evidence that the project is complete.");
    view.readings.forEach(reading => {
      const frame = reading.frame || {};
      const area = details(output, `${reading.id || "Reading"}: ${text(frame.summary) || "Inspect retained interpretation"}`);
      paragraph(area, frame.reason);
      strings(area, frame.unresolved, "Unresolved in this proposed reading");
      strings(area, reading.selection_uncertainty, "Selection uncertainty");
      list(frame.events).forEach(event => paragraph(area, `Event ${event.id || ""}: ${event.predicate || ""} · ${event.polarity || "unknown polarity"} · ${event.modality || "unknown modality"} · ${list(event.roles).map(role => `${role.role}: ${role.atom_id}`).join(", ")}`));
      list(reading.bound_usage?.atoms).forEach(atom => renderAtom(area, atom));
      list(reading.e8_activations).forEach(activation => {
        const d = details(area, `E8 representation: ${activation.concept_id || "concept"}`);
        const placement = activation.placement || {};
        paragraph(d, `${placement.status || activation.placement_status || "placement unreported"} · ${activation.source_mapping_asserted ? "source mapping asserted" : "candidate mapping retained"}`);
        paragraph(d, `Root anchor: ${placement.root_id || "unavailable"} · radius: ${placement.radius ?? "unavailable"} · hierarchy level: ${placement.hierarchy_level ?? "unreported"}`);
        const point = placement.position8;
        if (Array.isArray(point)) paragraph(d, `Fine position: ${point.join(", ")}`, "joe-coordinate");
        if (Array.isArray(placement.residual8)) paragraph(d, `Fine-position residual from root anchor: ${placement.residual8.join(", ")}`, "joe-coordinate");
        if (activation.lattice_address) {
          const address = activation.lattice_address;
          paragraph(d, `Full lattice address · ${address.geometry_version || "unreported convention"} · ${address.coset || "unreported coset"} · scale: ${address.fine_position_scale ?? "unreported"}`, "joe-coordinate");
          if (Array.isArray(address.position)) paragraph(d, `Lattice position: ${address.position.join(", ")}`, "joe-coordinate");
          if (Array.isArray(address.residual)) paragraph(d, `Lattice residual: ${address.residual.join(", ")}`, "joe-coordinate");
          paragraph(d, address.reconstruction, "joe-coordinate");
          paragraph(details(d, "Complete lattice address record"), JSON.stringify(address), "joe-coordinate");
        }
      });
    });
    const receipt = details(output, "Receipt and boundaries");
    paragraph(receipt, `Request: ${result.requestId || "unreported"}`, "joe-coordinate");
    paragraph(receipt, `Thread snapshot: ${result.threadId || "no selected thread"}`, "joe-coordinate");
    paragraph(receipt, `Private receipt: ${result.receiptPath || "not available"}`, "joe-coordinate");
    const calls = list(view.execution.calls);
    paragraph(receipt, `Provider attempts recorded: ${calls.length}`);
    calls.forEach(call => {
      paragraph(receipt, `${call.stage || "unreported stage"} · attempt ${(call.attempt ?? 0) + 1} · ${call.generation || "unreported outcome"} · ${call.provider || "unreported provider"} / ${call.model || "unreported model"}`, "joe-coordinate");
      if (call.validation) paragraph(receipt, `Validation: ${call.validation}`, "joe-notice");
      if (call.validation_error) paragraph(receipt, `Validation error: ${typeof call.validation_error === "object" ? JSON.stringify(call.validation_error) : call.validation_error}`, "joe-error");
    });
    Object.entries(view.receipt).forEach(([key, value]) => paragraph(receipt, `${key}: ${typeof value === "object" ? JSON.stringify(value) : value}`, "joe-coordinate"));
    paragraph(receipt, "Read-only interpretation: no coding tools dispatched, no approval granted, no memory committed.");
  }
  function renderContinuity(parent, attachment) {
    const view = continuityView(attachment);
    const area = details(parent, "Context continuity · CDISS");
    area.open = true;
    if (!view.available) { paragraph(area, view.reason); return; }
    const o = view.observation;
    paragraph(area, view.status === "compared" ? "Compared with the explicitly chosen previous review." : view.status === "reset-incompatible" ? "Started fresh because the previous review uses a different context or reference." : "Fresh observation of this proposed reading.", "joe-label");
    paragraph(area, `${o.readingCount} readings · ${o.atomCount} atom occurrences · ${o.eventCount} events · ${o.alternativeCount} selected alternatives retained.`);
    paragraph(area, `Fitted E8 allocation: ${o.mappedMass.toFixed(3)} · no fitted E8 position: ${o.unmappedMass.toFixed(3)}. Includes separately typed context pins; allocation is descriptive, not confidence.`);
    if (view.source) paragraph(area, `Source / context identity change: TV ${view.source.totalVariation.toFixed(3)} · Jensen–Shannon distance ${view.source.jensenShannonDistance.toFixed(3)}. Includes retained unmapped occurrences.`);
    if (view.structure) paragraph(area, `Role / polarity / modality change: TV ${view.structure.totalVariation.toFixed(3)} · Jensen–Shannon distance ${view.structure.jensenShannonDistance.toFixed(3)}.`);
    if (view.partitionChanged != null) paragraph(area, view.partitionChanged ? "Occurrence or alternative grouping changed." : "Occurrence and alternative grouping retained.");
    strings(area, view.reasons, "Comparison notes");
    paragraph(area, view.ignoredReason);
    paragraph(area, "The current reading and accumulated context remain separate. E8 positions are retained unchanged. These distances do not grant approval or measure completion.");
    const provenance = details(area, "Inspect continuity provenance");
    paragraph(provenance, `Algorithm: ${view.state.algorithmVersion}`, "joe-coordinate");
    paragraph(provenance, `State: ${view.state.stateHash}`, "joe-coordinate");
    paragraph(provenance, `Configuration: ${view.state.configDigest}`, "joe-coordinate");
    paragraph(provenance, `Previous state: ${view.state.previousStateHash || "none"}`, "joe-coordinate");
    paragraph(provenance, `Source model: ${view.state.basis?.modelSnapshotHash || "unreported"}`, "joe-coordinate");
  }
  if (example) example.addEventListener("click", async () => {
    if (busy) return;
    invalidate(); const current = ++generation;
    busy = true; example.disabled = true; analyze.disabled = true; updateComparison();
    status.textContent = "Preparing the authored local example. No provider call is made.";
    try {
      const sample = await invoke("joe_cdiss_example");
      if (current !== generation) return;
      if (sample?.schema !== "bomb-code/cdiss-example/v1" || sample.authority?.toolsDispatched !== false || sample.authority?.approvalsGranted !== false || sample.authority?.memoryCommitted !== false) throw new Error("Unexpected example boundary receipt");
      viewOf(sample.first); render(sample.second); showingExample = true;
      paragraph(output, `Example baseline: ${sample.first.sentence}`);
      paragraph(output, `Example current: ${sample.second.sentence}`);
      status.textContent = "Authored source-backed example: approved versus did not approve. This tests local wiring; it is not a live interpretation of your passage.";
    } catch (error) { output.replaceChildren(); status.textContent = `Local comparison unavailable: ${String(error)}`; }
    finally { busy = false; example.disabled = false; analyze.disabled = false; updateComparison(); }
  });
  byId("joe-form").addEventListener("submit", async event => {
    event.preventDefault();
    if (busy) return;
    const sentence = passage.value, lang = language.value.trim();
    if (!sentence.trim()) { status.textContent = "Enter an exact passage to analyze."; passage.focus(); return; }
    if (!/^[a-z]{3}$/.test(lang)) { status.textContent = "Enter a three-letter lowercase language code supported by the source snapshot, such as eng."; language.focus(); return; }
    const threadId = typeof state !== "undefined" ? state.selectedSession || null : null;
    if (!contextValid(sentence, threadId)) { status.textContent = "Thread context changed. Prepare a current review before analyzing."; return; }
    const compareRequestId = compare?.checked ? previousReviews.get(threadId) || null : null;
    const memoryEvidenceId = memoryReview?.question === sentence ? memoryReview.receiptId : null;
    invalidate();
    const current = ++generation;
    busy = true; analyze.disabled = true; analyze.textContent = "Analyzing…";
    if (example) example.disabled = true;
    updateComparison();
    lastResult = null; output.replaceChildren();
    status.textContent = "Preparing source candidates and a model-proposed reading. No coding tools are running for this review.";
    try {
      const result = await invoke("joe_analyze", { sentence, language: lang, threadId, compareRequestId, memoryEvidenceId });
      if (current !== generation || !contextValid(sentence, threadId) || !sameInput(result, passage.value, language.value.trim(), typeof state !== "undefined" ? state.selectedSession || null : null)) { status.textContent = "Input or thread context changed while the review was running. Prepare or analyze again for a current reading."; return; }
      if (memoryEvidenceId) {
        if (result.memoryEvidence?.receiptId !== memoryEvidenceId) throw new Error("The returned memory receipt does not match this review.");
        await invoke("memory_recall", { action: "validate", payload: { receiptId: memoryEvidenceId } });
        if (current !== generation || !contextValid(sentence, threadId) || !sameInput(result, passage.value, language.value.trim(), currentThread())) return;
      }
      render(result); lastResult = result;
      if (threadId && continuityView(result.cdiss).available && typeof result.requestId === "string") {
        previousReviews.delete(threadId); previousReviews.set(threadId, result.requestId);
        if (previousReviews.size > 32) previousReviews.delete(previousReviews.keys().next().value);
      }
      status.textContent = result.status === "interpretation-unavailable" ? "Interpretation unavailable. Its failure receipt is retained." : "Review ready. Meanings and questions remain proposals.";
      guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: result.status, result } }));
    } catch (error) {
      if (memoryEvidenceId) clearMemoryContext();
      output.replaceChildren(); lastResult = null;
      status.textContent = `Passage review unavailable: ${String(error)}`;
    } finally { busy = false; analyze.disabled = false; analyze.textContent = "Analyze passage"; if (example) example.disabled = false; updateComparison(); }
  });
})();
