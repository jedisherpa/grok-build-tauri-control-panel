/* Native sidebar controls become independent presentation cubes.
   Move original elements; never clone controls or dispatch coding commands. */
(function (global) {
  "use strict";
  const STORAGE = "bomb-code:panel-cubes:v3";
  function bounded(rect, bounds, collapsed = false) {
    const width = Math.max(160, bounds.width), height = Math.max(120, bounds.height);
    const w = Math.min(width - 36, Math.max(148, Number(rect.width) || 190));
    const h = Math.min(height - 68, Math.max(collapsed ? 34 : 56, Number(rect.height) || 160));
    return { x: Math.min(width - w - 24, Math.max(12, Number(rect.x) || 12)), y: Math.min(height - h - 12, Math.max(48, Number(rect.y) || 48)), width: w, height: h };
  }
  function attach({ background, scene, document: doc = global.document }) {
    const win = doc.defaultView, panels = [], cleanup = [];
    let z = 10, destroyed = false, saved = {};
    try { saved = JSON.parse(win.localStorage.getItem(STORAGE) || "{}") || {}; } catch { /* Layout is optional. */ }
    const layer = doc.createElement("div"); layer.className = "spatial-panel-cubes"; doc.body.appendChild(layer);
    doc.documentElement.classList.add("has-panel-cubes");
    const bounds = () => ({ width: win.innerWidth, height: win.innerHeight });
    function listen(node, type, fn) { node.addEventListener(type, fn); cleanup.push(() => node.removeEventListener(type, fn)); }
    function save() { try { win.localStorage.setItem(STORAGE, JSON.stringify(Object.fromEntries(panels.map(p => [p.id, { ...p.rect, expanded: p.expanded, expandedHeight: p.expandedHeight }])))); } catch { /* Private or full storage does not disable controls. */ } }
    function mask() {
      const elements = panels.map(p => p.element);
      background?.setExclusionElements?.(elements, "panel-cubes");
      scene?.setExclusionElements?.(elements, "panel-cubes");
      background?.refreshExclusions?.(); scene?.refreshExclusions?.();
    }
    function place(p) {
      p.rect = bounded(p.rect, bounds(), !p.expanded);
      Object.assign(p.element.style, { left: `${p.rect.x}px`, top: `${p.rect.y}px`, width: `${p.rect.width}px`, height: `${p.rect.height}px` });
      mask();
    }
    /** Right-rail cubes should stay docked to the window's right edge (play1 #16/#26). */
    function dockRight(p) {
      const b = bounds();
      p.rect = { ...p.rect, x: Math.max(12, b.width - p.rect.width - 24) };
      place(p);
    }
    function isRightRail(id) {
      return ["now", "agents", "tools", "details", "view", "log", "preview"].includes(id);
    }
    function expand(p, expanded, persist = true) {
      if (p.expanded === expanded) { p.body.hidden = !expanded; if (expanded && p.rect.height <= 34) { p.rect.height = Math.max(56, p.expandedHeight || 140); place(p); } return; }
      if (!expanded && p.rect.height > 34) p.expandedHeight = p.rect.height;
      p.expanded = expanded; p.body.hidden = !expanded;
      p.rect.height = expanded ? Math.max(56, p.expandedHeight || 160) : 34;
      p.element.dataset.expanded = String(expanded);
      if (!p.nativeToggle) { p.fold.setAttribute("aria-expanded", String(expanded)); p.fold.textContent = expanded ? "−" : "+"; }
      place(p); if (persist) save();
    }
    function gesture(p, handle, resize) {
      let drag;
      listen(handle, "pointerdown", e => { if (e.button !== 0) return; e.preventDefault(); handle.focus({ preventScroll: true }); drag = { id: e.pointerId, x: e.clientX, y: e.clientY, rect: { ...p.rect } }; handle.setPointerCapture(e.pointerId); p.element.style.zIndex = String(++z); });
      listen(handle, "pointermove", e => { if (drag?.id !== e.pointerId) return; const dx = e.clientX - drag.x, dy = e.clientY - drag.y; p.rect = resize ? { ...drag.rect, width: drag.rect.width + dx, height: drag.rect.height + dy } : { ...drag.rect, x: drag.rect.x + dx, y: drag.rect.y + dy }; place(p); });
      ["pointerup", "pointercancel", "lostpointercapture"].forEach(type => listen(handle, type, () => { if (drag) { drag = null; if (p.expanded) p.expandedHeight = p.rect.height; save(); } }));
      listen(handle, "keydown", e => { if (!/^Arrow(Left|Right|Up|Down)$/.test(e.key) || e.metaKey || e.ctrlKey || e.altKey) return; e.preventDefault(); const step = e.shiftKey ? 32 : 8, dx = e.key === "ArrowRight" ? step : e.key === "ArrowLeft" ? -step : 0, dy = e.key === "ArrowDown" ? step : e.key === "ArrowUp" ? -step : 0; p.rect = resize ? { ...p.rect, width: p.rect.width + dx, height: p.rect.height + dy } : { ...p.rect, x: p.rect.x + dx, y: p.rect.y + dy }; p.element.style.zIndex = String(++z); place(p); if (p.expanded) p.expandedHeight = p.rect.height; save(); });
    }
    function cube(id, title, content, rect, toggle = null, extras = []) {
      if (!content) return;
      const anchor = doc.createComment(`original ${id}`); content.before(anchor);
      const element = doc.createElement("section"); element.className = "spatial-panel-cube"; element.dataset.cube = id; element.setAttribute("aria-label", `${title} cube`); layer.appendChild(element);
      const header = doc.createElement("header"); header.className = "spatial-panel-cube-header"; element.appendChild(header);
      const move = doc.createElement("button"); move.type = "button"; move.className = "spatial-panel-cube-move"; move.textContent = title; move.setAttribute("aria-label", `Move ${title} cube`); move.title = "Drag to move · Arrow keys move · Shift moves farther"; header.appendChild(move);
      const nativeToggle = toggle !== null, fold = toggle || doc.createElement("button"), toggleAnchor = toggle ? doc.createComment(`original ${id} toggle`) : null;
      if (toggle) toggle.before(toggleAnchor);
      fold.type = "button"; fold.classList.add("spatial-panel-cube-fold"); fold.setAttribute("aria-label", `Expand or fold ${title} cube`); header.appendChild(fold);
      const body = doc.createElement("div"); body.className = "spatial-panel-cube-body"; element.appendChild(body); body.appendChild(content); const extraAnchors = extras.filter(Boolean).map(node => { const marker = doc.createComment(`original ${id} accessory`); node.before(marker); header.appendChild(node); return [node, marker]; });
      const resize = doc.createElement("button"); resize.type = "button"; resize.className = "spatial-panel-cube-resize"; resize.textContent = "⌟"; resize.setAttribute("aria-label", `Resize ${title} cube`); resize.title = "Drag to resize · Arrow keys resize · Shift changes farther"; element.appendChild(resize);
      const stored = saved[id], valid = stored && ["x", "y", "width", "height"].every(key => Number.isFinite(stored[key]));
      const p = { id, element, body, fold, nativeToggle, extraAnchors, anchor, content, toggleAnchor, defaultRect: { ...rect }, expanded: true, expandedHeight: valid ? stored.expandedHeight : rect.height === 34 ? 140 : rect.height, rect: valid ? stored : rect };
      panels.push(p); gesture(p, move, false); gesture(p, resize, true);
      if (toggle) {
        const sync = () => expand(p, toggle.getAttribute("aria-expanded") === "true", false);
        const observer = new win.MutationObserver(sync); observer.observe(toggle, { attributes: true, attributeFilter: ["aria-expanded"] }); cleanup.push(() => observer.disconnect()); sync();
      } else {
        fold.textContent = "−"; fold.setAttribute("aria-expanded", "true"); listen(fold, "click", () => expand(p, !p.expanded)); if (valid && stored.expanded === false) expand(p, false, false);
      }
      place(p); return p;
    }
    const left = doc.querySelector(".col-left"), right = doc.querySelector(".col-right");
    const brand = left?.querySelector(".brand");
    if (brand) { brand.classList.add("spatial-floating-brand"); doc.body.appendChild(brand); }
    const available = win.innerHeight - 76, leftScale = Math.min(1, (available - 36) / 740);
    cube("navigation", "Navigate", left?.querySelector(".nav-block"), { x: 12, y: 64, width: 180, height: 260 * leftScale });
    cube("projects", "Projects & threads", left?.querySelector(".threads-section"), { x: 22, y: 82 + 260 * leftScale, width: 180, height: 330 * leftScale });
    cube("services", "Services", left?.querySelector(".col-footer"), { x: 12, y: 100 + 590 * leftScale, width: 190, height: 150 * leftScale });
    const specs = [
      ["now", "Now", "#now-panel", null, 100], ["agents", "Agents", "#agent-list", "#agents-toggle", 100],
      ["tools", "Tools", "#tool-list", "#tools-toggle", 86], ["details", "Tool details", "#technical-transcript", null, 216],
      ["view", "View", "#view-options", "#view-toggle", 34], ["log", "Log", "#event-feed", "#log-toggle", 34],
      ["preview", "Live preview", ".dev-dock", null, 144],
    ];
    specs.forEach(spec => { if (spec[4] === 34 && doc.querySelector(spec[3])?.getAttribute("aria-expanded") === "true") spec[4] = 140; });
    const collapsedCount = specs.filter(spec => spec[4] === 34).length;
    const weight = specs.reduce((sum, spec) => sum + (spec[4] === 34 ? 0 : spec[4] - 56), 0);
    const extra = Math.max(0, available - 96 - collapsedCount * 34 - (7 - collapsedCount) * 56); let y = 64;
    specs.forEach(([id, title, selector, toggleId, h], i) => {
      const child = right?.querySelector(selector), content = child?.closest(".right-block") || child;
      const height = h === 34 ? 34 : 56 + Math.min(weight, extra) * (h - 56) / weight;
      cube(id, title, content, { x: win.innerWidth - 254 + (i % 2 ? 8 : 0), y, width: 214, height }, toggleId ? doc.querySelector(toggleId) : null, id === "now" ? [doc.getElementById("now-elapsed"), doc.getElementById("btn-refresh"), doc.getElementById("activity-bomb")] : []);
      y += height + 16;
    });
    /** Keep cubes out of the sticky composer band (round4 Play B ~960×670). */
    function clampAboveComposer(p) {
      const b = bounds();
      const reserve = Math.min(160, Math.max(120, Math.floor(b.height * 0.18)));
      const maxBottom = Math.max(40, b.height - reserve);
      if (p.rect.y + p.rect.height > maxBottom) {
        p.rect = {
          ...p.rect,
          y: Math.max(12, maxBottom - p.rect.height),
          height: Math.min(p.rect.height, Math.max(34, maxBottom - 12)),
        };
      }
    }
    function reflowOnResize() {
      // Enlarge/shrink: re-dock right rail against current bounds so cubes do
      // not stay at the old small-window coordinates (Play B B09b).
      const b = bounds();
      panels.forEach(p => {
        if (isRightRail(p.id)) {
          p.rect = {
            ...p.rect,
            x: Math.max(12, b.width - p.rect.width - 24),
          };
        }
        clampAboveComposer(p);
        place(p);
      });
      save();
    }
    const reset = doc.createElement("button"); reset.type = "button"; reset.className = "spatial-cubes-reset"; reset.textContent = "Reset cubes"; reset.title = "Restore the small cubes around your working surface"; layer.appendChild(reset);
    const defaults = new Map(panels.map(p => [p.id, { ...p.defaultRect }]));
    listen(reset, "click", () => {
      // Recompute from original defaults against the *current* window size so
      // Reset is visible after maximize/enlarge (play1 #15/#26).
      const b = bounds();
      panels.forEach(p => {
        const d = defaults.get(p.id);
        let rect = { ...d, height: p.expanded ? d.height : 34 };
        if (isRightRail(p.id)) rect.x = Math.max(12, b.width - rect.width - 24);
        p.rect = rect;
        p.expandedHeight = Math.max(76, d.height);
        clampAboveComposer(p);
        place(p);
      });
      try { win.localStorage.removeItem(STORAGE); } catch { /* ignore */ }
      save();
    });
    listen(win, "resize", reflowOnResize);
    listen(doc, "bomb-code:view-selected", () => win.queueMicrotask(mask));
    const observer = new win.ResizeObserver(mask); panels.forEach(p => observer.observe(p.element)); cleanup.push(() => observer.disconnect());
    return { elements: () => panels.map(p => p.element), destroy() { if (destroyed) return; destroyed = true; cleanup.forEach(fn => fn()); panels.forEach(p => { p.extraAnchors.forEach(([node, marker]) => marker.replaceWith(node)); if (p.toggleAnchor) p.toggleAnchor.replaceWith(p.fold); p.anchor.replaceWith(p.content); }); if (brand) { brand.classList.remove("spatial-floating-brand"); left.prepend(brand); } layer.remove(); doc.documentElement.classList.remove("has-panel-cubes"); background?.setExclusionElements?.([], "panel-cubes"); scene?.setExclusionElements?.([], "panel-cubes"); } };
  }
  const api = Object.freeze({ attach, bounded }); global.BombSpatialPanels = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
