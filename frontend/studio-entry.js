/* Local studio entry and workspace restoration. No session start and no provider calls. */
(function (global) {
  "use strict";

  const RESTORE_KEY = "c3:workspace-restoration:v1";
  const ENTRY_KEY = "c3:entry-preference:v1";
  const TOUR_KEY = "c3:tour-progress:v1";
  const MAX_BYTES = 4096;
  const LEGACY = Object.freeze(["bomb-code:panel-cubes:v1", "bomb-code:joe-corner", "bomb-code:joe-outcomes:v1", "bomb-code.pause-visual-motion"]);
  const ROUTES = Object.freeze({
    explore: Object.freeze({ view: "spatial", focus: "focus", openJoe: true }),
    collaborate: Object.freeze({ view: "spatial", focus: "focus", faces: true }),
    experiment: Object.freeze({ view: "builds" }),
    workspace: Object.freeze({ view: "spatial", focus: "focus" }),
    memory: Object.freeze({ view: "memory" }),
    settings: Object.freeze({ view: "settings" }),
    history: Object.freeze({ view: "history" }),
    builds: Object.freeze({ view: "builds" }),
  });

  const bytes = value => new TextEncoder().encode(String(value)).length;
  const cleanId = value => typeof value === "string" && value.length > 0 && value.length <= 512 && !/[\u0000-\u001f]/.test(value) ? value : null;

  function rect(value) {
    const x = value?.x, y = value?.y, width = value?.width, height = value?.height;
    if (![x, y, width, height].every(Number.isFinite) || width <= 0 || height <= 0 || width > 20000 || height > 20000 || Math.abs(x) > 20000 || Math.abs(y) > 20000) return null;
    return { x, y, width, height };
  }

  function parseRestoration(raw) {
    if (raw == null || raw === "") return { ok: true, empty: true };
    if (typeof raw !== "string") return { ok: false, reason: "invalid" };
    if (bytes(raw) > MAX_BYTES) return { ok: false, reason: "oversized", withheld: true };
    let data;
    try { data = JSON.parse(raw); } catch { return { ok: false, reason: "invalid" }; }
    if (!data || data.version !== 1 || Array.isArray(data)) return { ok: false, reason: "version" };
    if (["transcript", "credential", "token", "prompt", "secret"].some(key => Object.prototype.hasOwnProperty.call(data, key))) return { ok: false, reason: "forbidden-field" };
    if (data.threadId != null && !cleanId(data.threadId)) return { ok: false, reason: "thread" };
    if (data.projectId != null && !cleanId(data.projectId)) return { ok: false, reason: "project" };
    if (data.primary != null && !rect(data.primary)) return { ok: false, reason: "primary" };
    if (data.secondary != null && (!Array.isArray(data.secondary) || data.secondary.length > 8)) return { ok: false, reason: "secondary" };
    const secondary = (data.secondary || []).map(item => {
      const id = cleanId(item?.id), bounds = rect(item);
      return id && bounds ? { id, ...bounds } : null;
    });
    if (secondary.some(item => !item)) return { ok: false, reason: "secondary" };
    return { ok: true, value: { version: 1, threadId: data.threadId || null, projectId: data.projectId || null, primary: data.primary ? rect(data.primary) : null, secondary } };
  }

  function parsePreference(raw) {
    if (raw == null || raw === "") return { ok: true, empty: true };
    if (typeof raw !== "string" || bytes(raw) > 1024) return { ok: false, reason: bytes(String(raw)) > 1024 ? "oversized" : "invalid" };
    let data;
    try { data = JSON.parse(raw); } catch { return { ok: false, reason: "invalid" }; }
    if (!data || data.version !== 1) return { ok: false, reason: "version" };
    return { ok: true, value: { version: 1, seenIntroduction: data.seenIntroduction === true, upgradeDismissed: data.upgradeDismissed === true } };
  }

  function entryMode(preferenceRaw, keys = []) {
    const preference = parsePreference(preferenceRaw);
    const legacy = LEGACY.some(key => keys.includes(key));
    if (!preference.ok) return { mode: legacy ? "upgrade" : "work", invalid: true };
    if (preference.empty && legacy) return { mode: "upgrade", invalid: false };
    if (preference.empty) return { mode: "first", invalid: false };
    if (preference.value.seenIntroduction) return { mode: "returning", invalid: false };
    return { mode: legacy ? "upgrade" : "first", invalid: false };
  }

  function restorationDecision({ userTouched = false, bootstrapped = false, applied = false, record, sessions = [] } = {}) {
    if (!bootstrapped) return { restore: false, reason: "waiting" };
    if (applied) return { restore: false, reason: "done" };
    if (userTouched) return { restore: false, reason: "user" };
    if (!record?.ok || record.empty || !record.value) return { restore: false, reason: record?.withheld ? "withheld" : "absent" };
    const known = record.value.threadId ? sessions.some(session => session?.id === record.value.threadId) : false;
    if (record.value.threadId && !known) return { restore: true, selectThread: false, fallback: "project", layout: true };
    return { restore: true, selectThread: !!record.value.threadId, fallback: null, layout: true };
  }

  function clampRect(boundsRect, bounds) {
    const width = Math.max(1, Number(bounds?.width) || 1), height = Math.max(1, Number(bounds?.height) || 1);
    const nextWidth = Math.min(boundsRect.width, width), nextHeight = Math.min(boundsRect.height, height);
    return {
      x: Math.min(Math.max(0, boundsRect.x), Math.max(0, width - nextWidth)),
      y: Math.min(Math.max(0, boundsRect.y), Math.max(0, height - nextHeight)),
      width: nextWidth,
      height: nextHeight,
    };
  }

  function serializeRestoration(value) {
    const text = JSON.stringify({
      version: 1,
      projectId: cleanId(value?.projectId) || null,
      threadId: cleanId(value?.threadId) || null,
      primary: rect(value?.primary),
      secondary: (Array.isArray(value?.secondary) ? value.secondary : []).slice(0, 8).flatMap(item => {
        const id = cleanId(item?.id), bounds = rect(item);
        return id && bounds ? [{ id, ...bounds }] : [];
      }),
    });
    return bytes(text) > MAX_BYTES ? { ok: false, reason: "oversized", withheld: true } : { ok: true, text };
  }

  function attach(options = {}) {
    const doc = options.document || global.document;
    const win = doc.defaultView;
    const storage = options.storage || win.localStorage;
    const read = key => { try { return storage.getItem(key); } catch { return null; } };
    const write = (key, value) => { try { storage.setItem(key, value); return true; } catch { return false; } };
    const keys = () => { try { return Object.keys(storage); } catch { return []; } };
    let userTouched = false, applied = false, bootstrapped = false, restoring = false;
    const mode = entryMode(read(ENTRY_KEY), keys());
    const record = parseRestoration(read(RESTORE_KEY));
    const root = doc.createElement("section");
    root.className = "studio-invitation";
    root.id = "studio-invitation";
    root.setAttribute("role", "region");
    root.setAttribute("aria-label", "See Cubed studio");
    const mark = doc.createElement("p"); mark.className = "studio-mark"; mark.textContent = "See Cubed · C³";
    const title = doc.createElement("h2"); title.textContent = mode.mode === "upgrade" ? "See Cubed is your new studio" : "What would you like to make possible?";
    const copy = doc.createElement("p");
    copy.textContent = mode.mode === "upgrade"
      ? "Your projects, threads, and layout are still here. The introduction is optional."
      : "Creativity, collaboration, and consequence are ways into the same workspace.";
    const status = doc.createElement("p"); status.className = "studio-status"; status.setAttribute("role", "status");
    const choices = doc.createElement("div"); choices.className = "studio-choices";
    const tourBox = doc.createElement("div"); tourBox.className = "studio-tour"; tourBox.hidden = true;
    root.append(mark, title, copy, choices, tourBox, status);
    (options.parent || doc.body).appendChild(root);
    if (record.withheld) status.textContent = "Saved workspace layout was too large to restore. Your current work is unchanged.";
    function button(label, parent = choices) {
      const control = doc.createElement("button");
      control.type = "button";
      control.textContent = label;
      parent.appendChild(control);
      return control;
    }
    const explore = button("Explore an idea");
    const collaborate = button("Bring threads together");
    const experiment = button("Try it and see");
    const workspace = button("Open my workspace");
    const around = button("Show me around");
    workspace.className = "studio-direct";
    function remember(extra = {}) {
      write(ENTRY_KEY, JSON.stringify({ version: 1, seenIntroduction: true, upgradeDismissed: mode.mode !== "first", ...extra }));
    }
    function hide() { root.hidden = true; }
    function show(home = false) {
      root.hidden = false;
      title.textContent = home || mode.mode !== "first" ? "See Cubed is your new studio" : "What would you like to make possible?";
      around.focus();
    }
    async function go(name) {
      userTouched = true;
      const route = ROUTES[name];
      if (!route) return;
      remember();
      options.activateView?.(route.view);
      if (route.focus) options.setFocus?.(route.focus);
      if (route.openJoe) doc.dispatchEvent(new win.CustomEvent("bomb-code:open-joe"));
      if (route.faces) options.focusFaces?.();
      hide();
    }
    explore.addEventListener("click", () => go("explore"));
    collaborate.addEventListener("click", () => go("collaborate"));
    experiment.addEventListener("click", () => go("experiment"));
    workspace.addEventListener("click", () => go("workspace"));
    function showTour() {
      const tour = global.BombStudioTour;
      const checked = tour?.auditTour?.(tour.WELCOME);
      if (!checked?.ok) { status.textContent = "The introduction is unavailable. The workspace is ready."; return; }
      let index = 0;
      const paint = () => {
        const step = checked.steps[index];
        tourBox.hidden = false;
        tourBox.replaceChildren();
        const text = doc.createElement("p"); text.textContent = step.copy; tourBox.appendChild(text);
        const back = button("Back", tourBox), next = button("Next", tourBox), stop = button("Stop", tourBox);
        back.disabled = index === 0;
        tour.perform(tour.WELCOME.steps[index].action, {
          activateView: options.activateView,
          highlight: id => { root.dataset.highlight = id || ""; },
          parkJoe: () => options.parkJoe?.(),
        });
        back.addEventListener("click", () => { const moved = tour.move(tour.WELCOME, index, "back"); index = moved.index; paint(); });
        next.addEventListener("click", () => {
          const moved = tour.move(tour.WELCOME, index, "next");
          if (moved.status === "completed") { write(TOUR_KEY, JSON.stringify({ version: 1, tour: tour.WELCOME.id, step: "welcome", status: "completed" })); options.parkJoe?.(); tourBox.hidden = true; status.textContent = "Welcome complete. Help can show it again."; return; }
          index = moved.index; paint();
        });
        stop.addEventListener("click", () => { tour.move(tour.WELCOME, index, "stop"); write(TOUR_KEY, JSON.stringify({ version: 1, tour: tour.WELCOME.id, step: "welcome", status: "skipped" })); tourBox.hidden = true; options.parkJoe?.(); });
      };
      remember();
      paint();
    }
    around.addEventListener("click", showTour);
    root.addEventListener("keydown", event => { if (event.key === "Escape") { event.preventDefault(); go("workspace"); } });
    const home = doc.getElementById("studio-home");
    home?.addEventListener("click", () => show(true));
    if (mode.mode === "returning" || mode.mode === "work") hide();
    function noteUser() { if (!restoring) userTouched = true; }
    ["pointerdown", "keydown"].forEach(type => doc.addEventListener(type, event => {
      if (event.target?.closest?.("#thread-list, #prompt, .spatial-beacon, .spatial-primary-move, .spatial-resize-handle")) noteUser();
    }, true));
    function noteLayout() {
      if (restoring || (!applied && !userTouched)) return;
      const layout = options.layout?.();
      const work = options.work?.() || {};
      const saved = serializeRestoration({ ...work, primary: layout?.primary || null, secondary: layout?.secondary || [] });
      if (!saved.ok) { status.textContent = "Workspace layout was too large to save. Current work stays on screen."; return; }
      write(RESTORE_KEY, saved.text);
    }
    async function restore(sessions) {
      if (applied) return;
      bootstrapped = true;
      const decision = restorationDecision({ userTouched, bootstrapped, applied, record, sessions });
      applied = true;
      if (!decision.restore) return;
      restoring = true;
      try {
        if (decision.selectThread) await options.selectSession?.(record.value.threadId);
        else if (decision.fallback === "project") await options.selectSession?.(null);
        if (userTouched) return;
        const bounds = options.bounds?.() || { width: win.innerWidth || 960, height: win.innerHeight || 640 };
        const clamp = options.clamp || clampRect;
        if (record.value.primary) options.setWorkspaceRect?.(clamp(record.value.primary, bounds));
        (record.value.secondary || []).forEach(face => {
          if (sessions.some(session => session?.id === face.id)) options.placeFace?.(face.id, clamp(face, bounds));
        });
      } finally { restoring = false; }
      if (decision.restore && !userTouched) noteLayout();
    }
    doc.addEventListener("bomb-code:sessions-ready", event => { restore(event.detail?.sessions || options.sessions?.() || []); });
    return { element: root, show, hide, noteLayout, mode: mode.mode, restore, noteUser };
  }

  const api = Object.freeze({
    RESTORE_KEY, ENTRY_KEY, TOUR_KEY, LEGACY, ROUTES, parseRestoration, parsePreference, entryMode,
    restorationDecision, clampRect, serializeRestoration, attach,
  });
  global.BombStudioEntry = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
