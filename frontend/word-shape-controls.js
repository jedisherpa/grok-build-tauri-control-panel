/* Explicit local source lookups and historical reading inspection. */
(() => {
  "use strict";
  if (typeof document === "undefined") return;
  const el = id => document.getElementById(id);
  const form = el("word-dictionary-form");
  if (!form) return;
  const query = el("word-dictionary-query"), language = el("word-dictionary-language");
  const status = el("word-dictionary-status"), output = el("word-dictionary-result");
  const search = el("word-dictionary-search"), prev = el("word-dictionary-prev"), next = el("word-dictionary-next");
  const replayId = el("word-replay-id"), replayButton = el("word-replay-show");
  const replayStatus = el("word-replay-status"), replayOutput = el("word-replay-result");
  let generation = 0, replayGeneration = 0, offset = 0, count = 0, busy = false;
  const thread = () => typeof state !== "undefined" ? state.selectedSession || null : null;
  function controls() {
    search.disabled = busy; prev.disabled = busy || offset === 0;
    next.disabled = busy || offset + 20 >= count; replayButton.disabled = busy;
  }
  function clearDictionary() {
    generation++; offset = 0; count = 0; output.replaceChildren();
    status.textContent = "Dictionary query changed. Search for current source records."; controls();
  }
  function clearReplay() {
    replayGeneration++; replayOutput.replaceChildren();
    replayStatus.textContent = "Saved reading changed. Inspect the original review for this thread.";
  }
  async function lookup(page) {
    if (busy) return;
    const current = ++generation, text = query.value, lang = language.value;
    busy = true; controls(); output.replaceChildren(); status.textContent = "Checking frozen dictionary sources locally…";
    try {
      const result = await invoke("word_shape_dictionary", { payload: {action:"query",query:text,language:lang,offset:page,limit:20} });
      if (current !== generation || text !== query.value || lang !== language.value) return;
      if (result?.schema !== "bomb-code/dictionary-shapes/v1" || result.status !== "ready" || !Number.isSafeInteger(result.resultCount) || result.resultCount < 0 || !Array.isArray(result.hits) || result.hits.length > 20) throw new Error("Dictionary response is invalid.");
      offset = page; count = result.resultCount;
      globalThis.BombWordShapes.renderDictionary(output, result);
      status.textContent = `${count.toLocaleString()} matching senses · showing ${count ? offset + 1 : 0}–${Math.min(count, offset + result.hits.length)}. Source records and missing mappings remain distinct.`;
    } catch (error) {
      if (current === generation) { count = 0; offset = 0; output.replaceChildren(); status.textContent = `Dictionary unavailable: ${String(error)}`; }
    } finally { busy = false; controls(); }
  }
  form.addEventListener("submit", event => { event.preventDefault(); lookup(0); });
  query.addEventListener("input", clearDictionary); language.addEventListener("change", clearDictionary);
  prev.addEventListener("click", () => lookup(Math.max(0, offset - 20)));
  next.addEventListener("click", () => { if (offset + 20 < count) lookup(offset + 20); });
  replayId.addEventListener("input", clearReplay);
  for (const id of ["joe-passage", "joe-language"]) el(id)?.addEventListener("input", clearReplay);
  document.addEventListener("bomb-code:thread-selected", clearReplay);
  document.addEventListener("bomb-code:joe-context-changed", clearReplay);
  document.addEventListener("bomb-code:joe-interpretation", event => { if (event.detail?.status === "invalidated") clearReplay(); });
  replayButton.addEventListener("click", async () => {
    if (busy) return;
    const id = replayId.value.trim(), selected = thread(), current = ++replayGeneration;
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id) || !selected) {
      replayOutput.replaceChildren(); replayStatus.textContent = "Enter a saved review UUID and select its original Bomb Code thread."; return;
    }
    busy = true; controls(); replayOutput.replaceChildren(); replayStatus.textContent = "Checking the saved reading and its source pins locally…";
    try {
      const result = await invoke("joe_word_shape_replay", {requestId:id,threadId:selected});
      if (current !== replayGeneration || selected !== thread() || id !== replayId.value.trim()) return;
      if (result?.schema !== "bomb-code/word-shape-replay/v1" || result.threadId !== selected || result.requestId !== id || result.authority?.toolsDispatched !== false || result.authority?.approvalsGranted !== false || result.authority?.memoryCommitted !== false || result.wordShapes?.status !== "ready") throw new Error("Saved reading boundary is invalid.");
      if (result.memoryReceiptId) {
        await invoke("memory_recall", {action:"validate",payload:{receiptId:result.memoryReceiptId}});
        if (current !== replayGeneration || selected !== thread() || id !== replayId.value.trim()) return;
      }
      globalThis.BombWordShapes.render(replayOutput, result.wordShapes);
      replayStatus.textContent = `Saved review ${id.slice(0,8)} · ${result.notice}`;
    } catch (error) {
      if (current === replayGeneration) { replayOutput.replaceChildren(); replayStatus.textContent = `Saved reading unavailable: ${String(error)}`; }
    } finally { busy = false; controls(); }
  });
  controls();
})();
