/* Authored local tour. Steps may open a view, highlight, show a labelled fixture, or park Joe.
   They cannot invoke native execution, Analyze, Send, indexing, or approvals. */
(function (global) {
  "use strict";

  const ALLOWED = new Set(["open-view", "highlight", "prepare-fixture", "park-joe"]);
  const VIEWS = new Set(["spatial", "memory", "builds", "settings", "history", "chat"]);
  const FORBIDDEN = ["invoke", "command", "payload", "analyze", "approve", "send", "index", "build"];
  const WELCOME = Object.freeze({
    version: 1,
    id: "see-cubed-welcome",
    steps: Object.freeze([
      Object.freeze({
        id: "welcome",
        copy: "This is your studio. We can start with an idea or your existing work.",
        action: Object.freeze({ type: "open-view", view: "spatial", highlight: "studio-invitation" }),
      }),
    ]),
  });

  function audit(action) {
    if (!action || typeof action !== "object" || Array.isArray(action)) return { ok: false, reason: "missing" };
    if (FORBIDDEN.some(key => Object.prototype.hasOwnProperty.call(action, key))) return { ok: false, reason: "forbidden" };
    if (!ALLOWED.has(action.type)) return { ok: false, reason: "unlisted" };
    if (action.type === "open-view" && !VIEWS.has(action.view)) return { ok: false, reason: "view" };
    return { ok: true, action: action.type, view: action.view || null, highlight: action.highlight || null, fixture: action.fixture || null };
  }

  function auditTour(tour = WELCOME) {
    if (!tour || tour.version !== 1 || !Array.isArray(tour.steps) || !tour.steps.length) return { ok: false, reason: "tour" };
    const steps = tour.steps.map(step => {
      const checked = audit(step?.action);
      return checked.ok ? { ...checked, id: step.id, copy: step.copy } : checked;
    });
    if (steps.some(step => !step.ok)) return { ok: false, reason: "step", steps };
    return { ok: true, steps };
  }

  function move(tour, index, direction) {
    const checked = auditTour(tour);
    if (!checked.ok) return { ok: false, reason: checked.reason };
    const next = direction === "back" ? index - 1 : direction === "next" ? index + 1 : index;
    if (direction === "stop") return { ok: true, status: "skipped", index };
    if (next < 0) return { ok: true, status: "showing", index: 0, step: checked.steps[0] };
    if (next >= checked.steps.length) return { ok: true, status: "completed", index: checked.steps.length - 1, step: checked.steps.at(-1) };
    return { ok: true, status: "showing", index: next, step: checked.steps[next] };
  }

  function perform(action, context = {}) {
    const checked = audit(action);
    if (!checked.ok) return checked;
    if (checked.action === "open-view") context.activateView?.(checked.view);
    if (checked.highlight) context.highlight?.(checked.highlight);
    if (checked.action === "park-joe") context.parkJoe?.();
    if (checked.action === "prepare-fixture") context.showFixture?.(checked.fixture || "Sample · not your notes");
    return checked;
  }

  const api = Object.freeze({ WELCOME, ALLOWED, FORBIDDEN, audit, auditTour, move, perform });
  global.BombStudioTour = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
