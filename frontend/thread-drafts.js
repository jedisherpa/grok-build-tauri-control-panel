// Unsent text belongs to the selected thread. This store never sends or persists it.
(function (global) {
  function createStore() {
    const drafts = new Map();
    return { switchThread(previous, next, text) {
      if (previous === next) return String(text ?? "");
      drafts.set(previous || null, String(text ?? ""));
      return drafts.get(next || null) || "";
    } };
  }
  const api = { createStore }; global.BombThreadDrafts = api;
  if (typeof module !== "undefined") module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
