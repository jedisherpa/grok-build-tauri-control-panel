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
  globalThis.WizardJoeView = Object.freeze({ viewOf, appendDraft, sameInput });
  if (typeof document === "undefined") return;
  const byId = id => document.getElementById(id);
  const guide = byId("wizard-joe");
  if (!guide) return;
  const passage = byId("joe-passage"), language = byId("joe-language");
  const status = byId("joe-result-status"), output = byId("joe-result");
  const analyze = byId("joe-analyze");
  let busy = false, generation = 0, lastResult = null;
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
    generation++; lastResult = null; output.replaceChildren();
    status.textContent = "Thread changed. Your passage and unsent message are preserved; analyze again for this thread.";
    guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: "invalidated", reason: "thread-changed", result: null } }));
  });
  function invalidate() {
    generation++;
    if (lastResult) {
      lastResult = null;
      output.replaceChildren();
      status.textContent = "Passage or language changed. Analyze again for a current reading.";
      guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: "invalidated", result: null } }));
    }
  }
  passage.addEventListener("input", invalidate);
  language.addEventListener("input", invalidate);
  byId("joe-copy-composer").addEventListener("click", () => {
    passage.value = byId("prompt").value;
    invalidate();
    status.textContent = "Copied the unsent message. Nothing has been sent.";
    passage.focus();
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
    view.clarifications.forEach(question => {
      const section = node("section", null, "joe-question");
      paragraph(section, question.question, "joe-question-text");
      paragraph(section, question.reason);
      paragraph(section, `Origin: ${question.origin || "guide-proposed"} · readings: ${list(question.readingIds).join(", ") || "unreported"} · atoms: ${list(question.atomIds).join(", ") || "unreported"}`);
      const button = node("button", "Add question to unsent message", "btn ghost"); button.type = "button";
      button.addEventListener("click", () => {
        if (lastResult !== result || !sameInput(result, passage.value, language.value.trim(), result.threadId)) return;
        if ((typeof state !== "undefined" ? state.selectedSession || null : null) !== (result.threadId || null)) { status.textContent = "This review belongs to another thread. Select that thread or analyze again before drafting."; return; }
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
  byId("joe-form").addEventListener("submit", async event => {
    event.preventDefault();
    if (busy) return;
    const sentence = passage.value, lang = language.value.trim();
    if (!sentence.trim()) { status.textContent = "Enter an exact passage to analyze."; passage.focus(); return; }
    if (!/^[a-z]{3}$/.test(lang)) { status.textContent = "Enter a three-letter lowercase language code supported by the source snapshot, such as eng."; language.focus(); return; }
    const threadId = typeof state !== "undefined" ? state.selectedSession || null : null;
    invalidate();
    const current = ++generation;
    busy = true; analyze.disabled = true; analyze.textContent = "Analyzing…";
    lastResult = null; output.replaceChildren();
    status.textContent = "Preparing source candidates and a model-proposed reading. No coding tools are running for this review.";
    try {
      const result = await invoke("joe_analyze", { sentence, language: lang, threadId });
      if (current !== generation || !sameInput(result, passage.value, language.value.trim(), typeof state !== "undefined" ? state.selectedSession || null : null)) { status.textContent = "Input or selected thread changed while the review was running. Analyze again for a current reading."; return; }
      render(result); lastResult = result;
      status.textContent = result.status === "interpretation-unavailable" ? "Interpretation unavailable. Its failure receipt is retained." : "Review ready. Meanings and questions remain proposals.";
      guide.dispatchEvent(new CustomEvent("bomb-code:joe-interpretation", { bubbles: true, detail: { schema: "bomb-code/joe-visual-state/v1", status: result.status, result } }));
    } catch (error) {
      output.replaceChildren(); lastResult = null;
      status.textContent = `Passage review unavailable: ${String(error)}`;
    } finally { busy = false; analyze.disabled = false; analyze.textContent = "Analyze passage"; }
  });
})();
