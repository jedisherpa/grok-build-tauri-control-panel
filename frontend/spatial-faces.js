/* Multiple actual thread context faces; one original native coding surface.
   All commands delegate selection to the existing host. No tool dispatch. */
(function (global) {
  "use strict";
  const rows = value => Array.isArray(value) ? value : [];
  const lookup = (map, id) => map && typeof map.get === "function" ? map.get(id) : undefined;
  const string = value => typeof value === "string" ? value : "";
  const VISIBLE_ROLES = new Set(["user", "agent", "system", "approval", "error"]);
  function clip(value, limit) {
    const text = string(value);
    return text.length > limit ? `${text.slice(0, limit)}\n[Preview excerpt — focus this thread for its full conversation.]` : text;
  }
  function threadView(source, id, snapshot = {}) {
    const session = rows(source.sessions).find(row => row.id === id);
    if (!session) return null;
    const reported = rows(snapshot.sessions).find(row => row.id === id);
    const phase = reported?.phase;
    let status = reported?.savedOnly ? "Saved conversation" : {
      send: "Sending", think: "Agent working", tools: "Tool activity", reply: "Reply arriving", wait: "Waiting", done: "Turn completed", error: "Reported error", idle: "Idle", unknown: "Activity status unknown",
    }[phase] || "Activity status unavailable";
    if (Number.isInteger(reported?.toolsActive) && reported.toolsActive > 0) status += ` · ${reported.toolsActive} tools active`;
    if (Number.isInteger(reported?.pendingApprovals) && reported.pendingApprovals > 0) status += ` · ${reported.pendingApprovals} coding permissions pending`;
    const narration = rows(lookup(source.explainBySession, id));
    const transcript = rows(lookup(source.transcriptBySession, id)).filter(entry => VISIBLE_ROLES.has(entry.role));
    return {
      id, title: string(session.label) || `Thread ${id.slice(0, 8)}`,
      engine: string(session.backend) || "Engine unreported", project: string(session.projectRoot || session.project_root || session.cwd),
      status, narrativeCount: narration.length, transcriptCount: transcript.length,
      narration: narration.slice(-2).map(entry => ({ text: clip(entry.text, 6000), at: string(entry.at) })),
      transcript: transcript.slice(-10).map(entry => ({ role: entry.role, text: clip(entry.body, 8000), at: string(entry.at) })),
      cached: source.transcriptLoaded?.has?.(id) === true || transcript.length > 0,
    };
  }
  function boundedRect(rect, bounds) {
    const width = Math.max(40, Number(bounds.width) || 40), height = Math.max(40, Number(bounds.height) || 40);
    const pad = Math.min(12, width / 8, height / 8), top = Math.min(155, Math.max(pad, height - 120));
    const maxW = width - 2 * pad, maxH = Math.max(1, height - top - pad);
    const w = Math.min(maxW, Math.max(Math.min(320, maxW), Number(rect.width) || 320));
    const h = Math.min(maxH, Math.max(Math.min(230, maxH), Number(rect.height) || 300));
    return { x: Math.min(width - pad - w, Math.max(pad, Number(rect.x) || pad)), y: Math.min(height - pad - h, Math.max(top, Number(rect.y) || top)), width: w, height: h };
  }
  function overlaps(a, b, gap = 10) {
    return a.x < b.x + b.width + gap && a.x + a.width + gap > b.x && a.y < b.y + b.height + gap && a.y + a.height + gap > b.y;
  }
  function previewRect(bounds, occupied = []) {
    const base = boundedRect({ x: 12, y: 140, width: 320, height: 300 }, bounds);
    for (let y = base.y; y + base.height <= bounds.height - 10; y += 32) {
      for (let x = 12; x + base.width <= bounds.width - 10; x += 32) {
        const candidate = boundedRect({ ...base, x, y }, bounds);
        if (!occupied.some(rect => overlaps(candidate, rect))) return candidate;
      }
    }
    return boundedRect({ ...base, x: base.x + (occupied.length % 5) * 24, y: base.y + (occupied.length % 4) * 22 }, bounds);
  }
  function attach({ host, background, getState, selectSession, activateView }) {
    if (!host?.scene?.element || !host.scene.workspace) throw new Error("Native spatial working surface is unavailable.");
    const scene = host.scene, root = scene.element, doc = root.ownerDocument, win = doc.defaultView;
    const faces = new Map(), cleanup = [];
    let selected = getState().selectedSession || null, destroyed = false, choosing = false, z = 7, timer;
    const layer = doc.createElement("div"); layer.className = "spatial-faces-layer"; root.appendChild(layer);
    const toolbar = doc.createElement("div"); toolbar.className = "spatial-faces-toolbar"; root.appendChild(toolbar);
    const picker = doc.createElement("select"); picker.setAttribute("aria-label", "Thread to open in another face"); toolbar.appendChild(picker);
    const open = doc.createElement("button"); open.type = "button"; open.textContent = "Open face"; open.className = "spatial-control"; toolbar.appendChild(open);
    const notice = doc.createElement("span"); notice.className = "spatial-faces-notice"; notice.setAttribute("role", "status"); toolbar.appendChild(notice);
    const arrange = doc.createElement("button"); arrange.type = "button"; arrange.textContent = "Arrange faces"; arrange.className = "spatial-control"; toolbar.insertBefore(arrange, notice);
    const movePrimary = doc.createElement("button"); movePrimary.type = "button"; movePrimary.textContent = "Move working face"; movePrimary.className = "spatial-primary-move spatial-control"; movePrimary.title = "Drag to move the working face. Arrow keys move; Shift + arrows moves farther."; movePrimary.setAttribute("aria-label", "Move the original working face with arrow keys"); root.appendChild(movePrimary);
    root.classList.add("spatial-has-faces");
    function listen(target, type, callback, list = cleanup) { target.addEventListener(type, callback); list.push(() => target.removeEventListener(type, callback)); }
    const bounds = () => ({ width: root.clientWidth, height: root.clientHeight });
    const primaryRect = () => scene.getWorkspaceRect?.() || (() => { const a = root.getBoundingClientRect(), b = scene.workspace.getBoundingClientRect(); return { x: b.left - a.left, y: b.top - a.top, width: b.width, height: b.height }; })();
    function mask() {
      background?.setExclusionElements?.([...faces.values()].map(face => face.element).filter(Boolean), "secondary-faces");
      scene.setExclusionElements?.([...faces.values()].map(face => face.element).filter(Boolean), "secondary-faces");
      background?.refreshExclusions?.();
      scene.refreshExclusions?.();
      const rect = primaryRect(); Object.assign(movePrimary.style, { left: `${rect.x}px`, top: `${Math.max(155, rect.y)}px` });
    }
    function place(face) {
      face.rect = boundedRect(face.rect, bounds());
      if (face.element) Object.assign(face.element.style, { left: `${face.rect.x}px`, top: `${face.rect.y}px`, width: `${face.rect.width}px`, height: `${face.rect.height}px` });
      mask();
    }
    function addText(parent, tag, value, cls) { const node = doc.createElement(tag); node.textContent = value; if (cls) node.className = cls; parent.appendChild(node); return node; }
    function paint(face) {
      let snapshot = {}; try { snapshot = host.snapshot?.() || {}; } catch { /* Missing telemetry remains unknown. */ }
      const source = getState(), view = threadView(source, face.id, snapshot);
      if (!view || !face.element) return;
      const signature = JSON.stringify(view);
      if (face.signature === signature) return;
      face.signature = signature;
      face.title.textContent = view.title;
      face.meta.textContent = `${view.engine} · ${view.status}`;
      face.body.replaceChildren();
      addText(face.body, "p", "Read-only thread context · Focus to use the original coding controls.", "spatial-face-hint");
      if (view.project) addText(face.body, "p", view.project, "spatial-face-project");
      addText(face.body, "h3", "What's happening");
      if (!view.narration.length) addText(face.body, "p", "No cached explanation for this thread yet.", "spatial-face-hint");
      else {
        addText(face.body, "p", `Latest ${view.narration.length} of ${view.narrativeCount} cached explanation updates.`, "spatial-face-hint");
        view.narration.forEach(entry => addText(face.body, "p", entry.text, "spatial-face-text"));
      }
      addText(face.body, "h3", "Conversation & decisions");
      if (!view.transcript.length) addText(face.body, "p", view.cached ? "No visible conversation entries in the cached thread." : "Conversation is not loaded in this view. Focus this thread to load its original context.", "spatial-face-hint");
      else {
        addText(face.body, "p", `Latest ${view.transcript.length} of ${view.transcriptCount} cached conversation entries. Historical text does not grant permission.`, "spatial-face-hint");
        view.transcript.forEach(entry => { const section = doc.createElement("section"); addText(section, "p", entry.role === "user" ? "You" : entry.role === "agent" ? "Agent" : entry.role === "approval" ? "Recorded permission request · focus for native controls" : entry.role === "error" ? "Reported error" : "System", "spatial-face-role"); addText(section, "p", entry.text, "spatial-face-text"); face.body.appendChild(section); });
      }
      face.element.setAttribute("aria-label", `${view.title}, read-only thread context`);
    }
    function gesture(face, handle, resizing) {
      let drag = null;
      const listenFace = (type, callback) => listen(handle, type, callback, face.cleanup || cleanup);
      listenFace("pointerdown", event => {
        if (event.button !== 0) return;
        event.preventDefault(); handle.focus({ preventScroll: true }); face.element.style.zIndex = String(++z);
        drag = { id: event.pointerId, x: event.clientX, y: event.clientY, rect: { ...face.rect } };
        handle.setPointerCapture(event.pointerId);
      });
      listenFace("pointermove", event => {
        if (!drag || event.pointerId !== drag.id) return;
        const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
        face.rect = resizing ? { ...drag.rect, width: drag.rect.width + dx, height: drag.rect.height + dy } : { ...drag.rect, x: drag.rect.x + dx, y: drag.rect.y + dy };
        place(face);
      });
      const end = () => { drag = null; };
      listenFace("pointerup", end); listenFace("pointercancel", end); listenFace("lostpointercapture", end);
      listenFace("keydown", event => {
        if (!/^Arrow(Left|Right|Up|Down)$/.test(event.key) || event.metaKey || event.ctrlKey || event.altKey) return;
        event.preventDefault(); const step = event.shiftKey ? 32 : 12;
        const dx = event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0;
        const dy = event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0;
        face.rect = resizing ? { ...face.rect, width: face.rect.width + dx, height: face.rect.height + dy } : { ...face.rect, x: face.rect.x + dx, y: face.rect.y + dy };
        place(face);
      });
    }
    function removePreview(face) { if (face.element) { (face.cleanup || []).forEach(fn => fn()); face.cleanup = []; face.element.remove(); face.element = null; face.signature = null; } }
    function ensurePreview(face) {
      if (face.element || face.id === selected) return;
      const element = doc.createElement("article"); element.className = "spatial-thread-face"; element.dataset.threadId = face.id; face.element = element; layer.appendChild(element);
      face.cleanup = [];
      const header = doc.createElement("header"); header.className = "spatial-face-header"; element.appendChild(header);
      const move = doc.createElement("button"); move.type = "button"; move.className = "spatial-face-move"; move.title = "Drag to move. Arrow keys move; Shift + arrows moves farther."; move.setAttribute("aria-label", "Move this thread face with arrow keys"); header.appendChild(move);
      face.title = addText(move, "span", "Thread", "spatial-face-title"); face.meta = addText(move, "span", "", "spatial-face-meta");
      const focus = doc.createElement("button"); focus.type = "button"; focus.textContent = "Focus"; focus.className = "spatial-control"; header.appendChild(focus);
      const close = doc.createElement("button"); close.type = "button"; close.textContent = "×"; close.className = "spatial-control"; close.setAttribute("aria-label", "Close this context face, keeping its thread"); header.appendChild(close);
      face.body = doc.createElement("div"); face.body.className = "spatial-face-body"; element.appendChild(face.body);
      const resize = doc.createElement("button"); resize.type = "button"; resize.textContent = "↘"; resize.className = "spatial-face-resize"; resize.setAttribute("aria-label", "Resize this thread face with arrow keys"); resize.title = "Drag to resize. Arrow keys resize; Shift + arrows changes farther."; element.appendChild(resize);
      listen(focus, "click", () => promote(face.id), face.cleanup);
      listen(close, "click", () => { removePreview(face); faces.delete(face.id); mask(); notice.textContent = "Context face closed. The thread is retained."; }, face.cleanup);
      gesture(face, move, false); gesture(face, resize, true); place(face); paint(face);
    }
    async function promote(id) {
      if (choosing || !rows(getState().sessions).some(row => row.id === id)) return;
      choosing = true;
      try {
        await selectSession(id); activateView("spatial"); host.setView?.("focus");
        syncSelection();
        notice.textContent = "Focused thread owns the original composer. Other faces remain read-only.";
      } catch (error) { notice.textContent = `Could not focus this thread: ${String(error)}`; }
      finally { choosing = false; }
    }
    function syncSelection() {
      const next = getState().selectedSession || null;
      if (next === selected) return;
      const available = new Set(rows(getState().sessions).map(row => row.id));
      const old = selected;
      if (old && available.has(old)) {
        let previous = faces.get(old);
        if (!previous) { previous = { id: old, rect: primaryRect() }; faces.set(old, previous); }
        previous.rect = primaryRect();
      }
      const target = next && faces.get(next);
      selected = next;
      if (target) { removePreview(target); scene.setWorkspaceRect?.(target.rect); }
      else if (next && available.has(next)) {
        const occupied = [...faces.values()].map(face => face.rect);
        const rect = faces.size ? previewRect(bounds(), occupied) : primaryRect();
        faces.set(next, { id: next, rect }); scene.setWorkspaceRect?.(rect);
      }
      faces.forEach(face => { if (face.id !== selected) ensurePreview(face); }); mask();
    }
    function openFace(id) {
      if (!rows(getState().sessions).some(row => row.id === id)) { notice.textContent = "Choose an available thread."; return; }
      if (id === selected) { host.setView?.("focus"); notice.textContent = "This thread already owns the working face."; return; }
      let face = faces.get(id);
      if (!face) { face = { id, rect: previewRect(bounds(), [primaryRect(), ...[...faces.values()].map(row => row.rect)]) }; faces.set(id, face); }
      host.setView?.("focus"); ensurePreview(face); face.element.style.zIndex = String(++z); notice.textContent = "Opened actual cached thread context. Focus it to use coding controls.";
    }
    function arrangeFaces() {
      const b = bounds(), records = [faces.get(selected), ...[...faces.values()].filter(face => face.id !== selected)].filter(Boolean);
      if (!records.length) return;
      const columns = Math.min(records.length, Math.max(1, Math.floor((b.width - 24 + 12) / 332))), rowCount = Math.ceil(records.length / columns);
      const width = (b.width - 24 - 12 * (columns - 1)) / columns, height = (b.height - 155 - 12 - 12 * (rowCount - 1)) / rowCount;
      records.forEach((face, index) => { face.rect = boundedRect({ x: 12 + (index % columns) * (width + 12), y: 155 + Math.floor(index / columns) * (height + 12), width, height }, b); if (face.id === selected) scene.setWorkspaceRect?.(face.rect); else place(face); });
      mask(); notice.textContent = height >= 240 ? "Open faces arranged within the scene." : "This window is too small to tile every face. Enlarge it or close a context face; movement stays bounded.";
    }
    function refresh() {
      if (destroyed) return;
      syncSelection();
      const source = getState(), sessions = rows(source.sessions);
      const signature = sessions.map(session => `${session.id}:${session.label || ""}:${session.backend || ""}`).join("|");
      if (picker.dataset.signature !== signature) {
        const before = picker.value; picker.replaceChildren();
        sessions.forEach(session => { const option = doc.createElement("option"); option.value = session.id; option.textContent = `${session.label || `Thread ${session.id.slice(0, 8)}`} · ${session.backend || "engine unreported"}`; picker.appendChild(option); });
        picker.dataset.signature = signature;
        picker.value = sessions.some(session => session.id === before) ? before : sessions.find(session => session.id !== selected)?.id || sessions[0]?.id || "";
      }
      open.disabled = !sessions.length;
      const available = new Set(sessions.map(session => session.id));
      faces.forEach((face, id) => { if (!available.has(id)) { removePreview(face); faces.delete(id); } else if (id !== selected) { ensurePreview(face); paint(face); } });
      mask();
    }
    if (selected) faces.set(selected, { id: selected, rect: primaryRect() });
    listen(open, "click", () => openFace(picker.value));
    listen(arrange, "click", arrangeFaces);
    let moving = null;
    listen(movePrimary, "pointerdown", event => { if (event.button !== 0) return; event.preventDefault(); movePrimary.focus({ preventScroll: true }); moving = { id: event.pointerId, x: event.clientX, y: event.clientY, rect: primaryRect() }; movePrimary.setPointerCapture(event.pointerId); });
    listen(movePrimary, "pointermove", event => { if (moving?.id !== event.pointerId) return; scene.setWorkspaceRect?.(boundedRect({ ...moving.rect, x: moving.rect.x + event.clientX - moving.x, y: moving.rect.y + event.clientY - moving.y }, bounds())); mask(); });
    ["pointerup", "pointercancel", "lostpointercapture"].forEach(type => listen(movePrimary, type, () => { moving = null; }));
    listen(movePrimary, "keydown", event => { if (!/^Arrow(Left|Right|Up|Down)$/.test(event.key) || event.metaKey || event.ctrlKey || event.altKey) return; event.preventDefault(); const step = event.shiftKey ? 32 : 12, rect = primaryRect(); scene.setWorkspaceRect?.(boundedRect({ ...rect, x: rect.x + (event.key === "ArrowRight" ? step : event.key === "ArrowLeft" ? -step : 0), y: rect.y + (event.key === "ArrowDown" ? step : event.key === "ArrowUp" ? -step : 0) }, bounds())); mask(); });
    listen(doc, "bomb-code:thread-selected", refresh);
    listen(doc, "bomb-code:view-selected", () => { win.queueMicrotask(refresh); });
    const observer = new win.ResizeObserver(() => { faces.forEach(face => { if (face.id !== selected) place(face); }); mask(); }); observer.observe(root); cleanup.push(() => observer.disconnect());
    timer = win.setInterval(refresh, 700);
    refresh();
    return { refresh, openFace, promote, arrange: arrangeFaces, getState: () => ({ selected, openFaces: [...faces.keys()] }), destroy() { if (destroyed) return; destroyed = true; win.clearInterval(timer); faces.forEach(removePreview); cleanup.forEach(fn => fn()); background?.setExclusionElements?.([], "secondary-faces"); scene.setExclusionElements?.([], "secondary-faces"); root.classList.remove("spatial-has-faces"); layer.remove(); toolbar.remove(); movePrimary.remove(); } };
  }
  const api = Object.freeze({ attach, threadView, boundedRect, previewRect });
  global.BombSpatialFaces = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
