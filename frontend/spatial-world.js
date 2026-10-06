/* Source-canonical E8 scene. This module renders snapshots and navigation only.
   It has no Tauri calls, provider calls, coding commands, or approval actions. */
(function (global) {
  "use strict";

  const PLANE_SHA = "fd8ac2058aee11bfb68ed69dec7aa424db56db465e8e61724cfdbda89e58472a";
  const ASSET_SHA = "f124fe8ff3bb6706a2013a72b2f90bc589c446e080c05085481a113fa77edd21";
  const STALE_MS = 25000;
  const PALETTE = ["#b7e3ce", "#c4b5dd", "#eed8b5"];
  const ACTIVE = new Set(["send", "think", "tools", "reply"]);
  const PHASES = new Set(["idle", "send", "think", "tools", "reply", "wait", "done", "error", "unknown", "disconnected"]);
  const PHASE_LABEL = { idle: "Idle", send: "Sending", think: "Agent working", tools: "Tools running", reply: "Reply arriving", wait: "Waiting for you", done: "Turn ended", error: "Turn error", unknown: "Status unknown", disconnected: "Disconnected" };
  const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));
  const dot = (a, b) => a.reduce((sum, x, i) => sum + x * b[i], 0);
  const count = value => Number.isSafeInteger(value) && value >= 0 ? value : null;
  const text = (value, max = 160) => typeof value === "string" ? value.slice(0, max) : "";

  function validateScaffold(data) {
    if (!data || data.schema !== "bomb-code/e8-reference-scaffold/v1" || data.sourcePlaneSha256 !== PLANE_SHA) throw new Error("E8 reference convention differs");
    if (!Array.isArray(data.roots) || data.roots.length !== 240 || !Array.isArray(data.edges) || data.edges.length !== 6720) throw new Error("Expected 240 roots and 6,720 edges");
    const unique = new Set();
    data.roots.forEach((root, i) => {
      const v = root.position8;
      if (root.id !== `e8-root:${i + 1}` || root.index !== i || !Array.isArray(v) || v.length !== 8 || v.some(x => !Number.isFinite(x))) throw new Error("Invalid canonical root ID or coordinates");
      const sparse = v.filter(x => Math.abs(x) === 1).length === 2 && v.filter(x => x === 0).length === 6;
      const dense = v.every(x => Math.abs(x) === 0.5) && v.filter(x => x < 0).length % 2 === 0;
      if ((!sparse && !dense) || dot(v, v) !== 2 || unique.has(v.join(","))) throw new Error("Root is outside the canonical E8 set");
      unique.add(v.join(","));
      if (i) {
        const previous = data.roots[i - 1].position8;
        const first = v.findIndex((x, j) => x !== previous[j]);
        if (first < 0 || v[first] < previous[first]) throw new Error("Root IDs must use source lexicographic order");
      }
    });
    const edges = new Set(), degrees = new Array(240).fill(0);
    data.edges.forEach(edge => {
      if (!Array.isArray(edge) || edge.length !== 2) throw new Error("Invalid edge");
      const [a, b] = edge;
      if (!Number.isInteger(a) || !Number.isInteger(b) || a < 0 || b >= 240 || a >= b || edges.has(`${a}:${b}`) || dot(data.roots[a].position8, data.roots[b].position8) !== 1) throw new Error("Edge is not 8D nearest-neighbor adjacency");
      edges.add(`${a}:${b}`); degrees[a]++; degrees[b]++;
    });
    if (degrees.some(n => n !== 56)) throw new Error("E8 degree differs");
    const q = data.projectionBasisQ;
    if (!Array.isArray(q) || q.length !== 8 || q.some(row => !Array.isArray(row) || row.length !== 8 || row.some(x => !Number.isFinite(x)))) throw new Error("Invalid projection basis");
    q.forEach((row, i) => q.forEach((other, j) => { if (Math.abs(dot(row, other) - (i === j ? 1 : 0)) > 1e-10) throw new Error("Projection basis is not orthonormal"); }));
    return data;
  }

  function rotate8(vector, rotations = []) {
    const out = vector.slice();
    rotations.forEach(([a, b, angle]) => {
      if (!Number.isInteger(a) || !Number.isInteger(b) || a < 0 || b >= 8 || a >= b || !Number.isFinite(angle)) throw new Error("Invalid 8D rotation");
      const c = Math.cos(angle), s = Math.sin(angle), x = out[a], y = out[b];
      out[a] = c * x - s * y; out[b] = s * x + c * y;
    });
    return out;
  }

  function projectVector(vector, q, plane = 0, yaw = 0, tilt = 0) {
    if (!Number.isInteger(plane) || plane < 0 || plane > 3) throw new Error("Unknown source plane");
    const v = rotate8(vector, [[0, 2, yaw], [1, 3, tilt]]);
    return { x: dot(v, q[2 * plane]), y: dot(v, q[2 * plane + 1]), z: dot(v, q[(2 * plane + 2) % 8]) };
  }

  function stableIndex(id, maximum = 240) {
    let h = 2166136261;
    for (const ch of String(id)) { h ^= ch.codePointAt(0); h = Math.imul(h, 16777619); }
    return (h >>> 0) % maximum;
  }

  function signalTime(value) {
    if (typeof value === "number") return Number.isFinite(value) && value > 0 ? value : null;
    if (typeof value !== "string" || !value.trim()) return null;
    const t = Date.parse(value); return Number.isFinite(t) && t > 0 ? t : null;
  }

  function normalizeSnapshots(rows, now = Date.now()) {
    const ids = new Set();
    return (Array.isArray(rows) ? rows : []).filter(row => row && typeof row.id === "string" && row.id && !ids.has(row.id) && ids.add(row.id)).map(row => {
      const phase = PHASES.has(row.phase) ? row.phase : "unknown";
      const at = signalTime(row.lastSignalAt);
      const ageMs = at !== null && at <= now + 5000 ? Math.max(0, now - at) : null;
      const live = row.live === true ? true : row.live === false ? false : null;
      const approvals = count(row.pendingApprovals), tools = count(row.toolsActive);
      const saved = row.savedOnly === true || live === false;
      const running = !saved && row.sessionClosed !== true && live === true && ACTIVE.has(phase);
      const fresh = ageMs !== null && ageMs <= STALE_MS;
      let label = saved ? "Saved history" : PHASE_LABEL[phase];
      if (!saved && live === null) label = "Live status unknown";
      if (!saved && approvals > 0) label = `${approvals} ${approvals === 1 ? "request" : "requests"} for approval`;
      else if (running && !fresh) label = ageMs === null ? "Active reported · signal time unknown" : `No recent signal · ${formatAge(ageMs)}`;
      else if (!saved && live === true && phase === "tools" && tools !== null) label = `${tools} ${tools === 1 ? "tool" : "tools"} running`;
      const sessionClosed = row.sessionClosed === true, turnEndedAt = signalTime(row.turnEndedAt);
      const qualifier = !saved && sessionClosed ? turnEndedAt !== null ? "Session closed · turn ended" : "Session closed" : "";
      return { id: row.id, title: text(row.title) || `Thread ${row.id.slice(0, 8)}`, engine: text(row.engine, 48) || "Engine unreported", project: text(row.project) || "Threads", phase, live, saved, pendingApprovals: approvals, toolsActive: tools, ageMs, fresh, running, animated: running && fresh, sessionClosed, turnEndedAt, qualifier, label };
    });
  }

  function formatAge(ms) { const s = Math.floor(ms / 1000); return s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${Math.floor(s / 3600)}h`; }
  function smooth(t) { const x = clamp(t, 0, 1); return x * x * (3 - 2 * x); }

  function normalizeExclusions(rects, width, height) {
    return (Array.isArray(rects) ? rects : []).flatMap(r => {
      const x = r?.x ?? r?.left, y = r?.y ?? r?.top, w = r?.width ?? (r?.right - x), h = r?.height ?? (r?.bottom - y);
      if (![x,y,w,h].every(Number.isFinite) || w <= 0 || h <= 0) return [];
      const left = clamp(x,0,width), top = clamp(y,0,height), right = clamp(x+w,0,width), bottom = clamp(y+h,0,height);
      return right > left && bottom > top ? [{x:left,y:top,width:right-left,height:bottom-top}] : [];
    });
  }

  function boundWorkspaceRect(rect, bounds) {
    const margin = bounds.margin ?? 16, top = Math.min(bounds.top ?? 125,Math.max(0,bounds.height-80)), bottom = bounds.bottom ?? 60;
    const maxWidth = Math.max(1,bounds.width-2*margin), maxHeight = Math.max(1,bounds.height-top-bottom);
    const width = clamp(rect.width,Math.min(320,maxWidth),maxWidth), height = clamp(rect.height,Math.min(240,maxHeight),maxHeight);
    return {x:clamp(rect.x,margin,Math.max(margin,bounds.width-margin-width)),y:clamp(rect.y,top,Math.max(top,bounds.height-bottom-height)),width,height};
  }

  // A decorative, bounded route, independent of every work or provider signal.
  function peripheralRoute(seconds, width, height) {
    const top = Math.min(height - 24, Math.max(210, height * 0.34)), bottom = height - 18;
    const path = [[width * .10, bottom], [width * .90, bottom], [width * .93, top], [width * .15, top], [width * .07, (top + bottom) / 2]];
    const lengths = path.map((a, i) => { const b = path[(i + 1) % path.length]; return Math.hypot(b[0] - a[0], b[1] - a[1]); });
    const total = lengths.reduce((a, b) => a + b, 0);
    let distance = ((Math.max(0, seconds) * 64) % total);
    for (let i = 0; i < path.length; i++) {
      if (distance <= lengths[i] || i === path.length - 1) { const a = path[i], b = path[(i + 1) % path.length], t = distance / lengths[i]; return { x: a[0] + (b[0] - a[0]) * t, y: a[1] + (b[1] - a[1]) * t, totalLength: total }; }
      distance -= lengths[i];
    }
  }

  function attach(container, options = {}) {
    if (!container || typeof container.appendChild !== "function") throw new Error("A DOM container is required");
    const doc = container.ownerDocument, win = doc.defaultView;
    const cleanup = [], snapshots = new Map();
    const exclusionElements = new Map(), exclusionRects = new Map(), primaryExclusionOwner = {};
    let exclusionObserver = null, workspaceRect = null, resizeDrag = null;
    let destroyed = false, scaffold = null, raw = {}, sessions = [], selectedId = options.selectedSessionId || null;
    let view = options.view === "focus" && (selectedId || options.allowEmptyFocus || options.contentElement) ? "focus" : options.view === "overview" || !selectedId ? "overview" : "focus";
    let yaw = 0, tilt = 0, plane = 0, width = 1, height = 1, dpr = 1, raf = null, ticker = null;
    let aperture = view === "focus" ? 1 : 0, transition = null, userPaused = options.paused === true;
    let sprite = null, spriteImage = null, spriteLoaded = false, spriteGeneration = 0, spriteTime = 0, lastFrame = null;
    let drag = null, lastPointerX = 0, lastPointerY = 0, showAll = false, telemetryAvailable = true;
    const reduced = win.matchMedia("(prefers-reduced-motion: reduce)");

    function element(tag, cls, value, parent = root) { const e = doc.createElement(tag); if (cls) e.className = cls; if (value) e.textContent = value; parent.appendChild(e); return e; }
    function listen(target, event, fn, opts) { target.addEventListener(event, fn, opts); cleanup.push(() => target.removeEventListener(event, fn, opts)); }
    const root = doc.createElement("section"); root.className = options.backgroundOnly ? "spatial-world spatial-background-only" : "spatial-world"; if (options.renderScaffold === false) root.classList.add("spatial-shared-background"); root.dataset.view = view; root.setAttribute("aria-label", options.backgroundOnly ? "Source E8 background" : "Spatial thread overview"); container.appendChild(root);
    const spriteCanvas = element("canvas", "spatial-joe-canvas"); spriteCanvas.setAttribute("aria-hidden", "true");
    const canvas = element("canvas", "spatial-lattice-canvas"); canvas.tabIndex = 0; canvas.setAttribute("role", "img"); canvas.setAttribute("aria-label", "E8 scaffold. Drag or use arrow keys to rotate in Overview; Home restores the view. Thread controls are available below.");
    if (options.backgroundOnly) { canvas.tabIndex = -1; canvas.setAttribute("aria-hidden", "true"); root.setAttribute("aria-hidden", "true"); }
    const ctx = canvas.getContext("2d"), spriteCtx = spriteCanvas.getContext("2d");
    const header = element("div", "spatial-header");
    const heading = element("div", "spatial-heading", "", header); element("span", "spatial-kicker", "Bomb Code", heading); element("h2", "", "Strata Observatory", heading);
    const controls = element("div", "spatial-controls", "", header);
    function button(label, fn, parent = controls) { const b = element("button", "spatial-control", label, parent); b.type = "button"; listen(b, "click", fn); return b; }
    const overviewButton = button("Overview", () => setView("overview"));
    const focusButton = button("Focus", () => setView("focus"));
    const rotateLeft = button("↶", () => turn(-0.16, 0)); rotateLeft.setAttribute("aria-label", "Rotate projection left");
    const rotateRight = button("↷", () => turn(0.16, 0)); rotateRight.setAttribute("aria-label", "Rotate projection right");
    const homeButton = button("Home view", () => setProjection({yaw:0,tilt:0,plane:0}));
    const planeSelect = element("select", "spatial-plane-select", "", controls); planeSelect.setAttribute("aria-label", "E8 projection plane");
    ["Source plane 1", "Source plane 2", "Source plane 3", "Source plane 4"].forEach((name, i) => { const o = element("option", "", name, planeSelect); o.value = String(i); });
    [rotateLeft, rotateRight, homeButton, planeSelect].forEach(b => b.classList.add("spatial-orbit-control"));
    listen(planeSelect, "change", () => setProjection({plane:Number(planeSelect.value)}));
    const pauseButton = button("Pause motion", () => { setMotionPaused(!userPaused); options.onPauseChange?.(userPaused); });
    const summary = element("p", "spatial-summary"); summary.setAttribute("role", "status"); summary.setAttribute("aria-live", "polite");
    const approvals = element("span", "spatial-approval-summary");
    const beaconList = element("nav", "spatial-beacons"); beaconList.setAttribute("aria-label", "Threads and observed status");
    const moreButton = button("All threads", () => { if (view === "focus") { showAll = true; setView("overview"); } else showAll = !showAll; renderLabels(); requestDraw(); }, root); moreButton.classList.add("spatial-more"); moreButton.setAttribute("aria-expanded", "false");
    const workspace = element("div", "spatial-workspace");
    const resizeHandle = button("↘", () => {}, root); resizeHandle.classList.add("spatial-resize-handle"); resizeHandle.setAttribute("aria-label", "Resize working surface. Drag or use arrow keys; Shift makes larger changes."); resizeHandle.title = "Drag to resize · Arrow keys resize · Home resets";
    let contentElement = null, originalParent = null, originalNext = null;
    const placeholder = element("p", "spatial-empty-focus", "Select a thread to open its working surface.", workspace);
    function mountContent(node) {
      if (!node || node === container || node === root || node.contains(container)) throw new Error("Working content must be a separate DOM element");
      if (contentElement === node) { if (!workspace.contains(node)) workspace.appendChild(node); placeholder.hidden = true; return; }
      if (contentElement) detachContent(true);
      contentElement = node; originalParent = node.parentNode; originalNext = node.nextSibling;
      workspace.appendChild(node); placeholder.hidden = true;
    }
    function detachContent(restore = true) {
      const node = contentElement;
      if (!node) return null;
      if (workspace.contains(node)) {
        if (restore && originalParent) { if (originalNext?.parentNode === originalParent) originalParent.insertBefore(node, originalNext); else originalParent.appendChild(node); }
        else node.remove();
      }
      contentElement = null; originalParent = null; originalNext = null; placeholder.hidden = false;
      return node;
    }
    if (options.contentElement) mountContent(options.contentElement);
    const empty = element("p", "spatial-empty", "No threads in this view.");
    const legend = element("p", "spatial-legend", "240 E8 roots · geometric edges · thread locations are navigation addresses"); legend.title = "A sparse subset of 6,720 original 8D edges is shown. Geometric adjacency does not assert a work dependency or meaning.";
    const notice = element("p", "spatial-notice", "Loading the pinned E8 reference…"); notice.setAttribute("role", "status");

    function typing() { const active = doc.activeElement; return raw.typing === true || !!active && (active.tagName === "TEXTAREA" || active.tagName === "INPUT" || active.isContentEditable); }
    function globalPause() { return doc.documentElement.dataset.motion === "paused"; }
    function motionStopped() { return userPaused || reduced.matches || globalPause() || doc.hidden || typing(); }
    function nowClock() { return win.performance.now(); }

    function refresh(source = raw) {
      raw = Array.isArray(source) ? { sessions: source } : source && typeof source === "object" ? source : {};
      sessions = normalizeSnapshots(raw.sessions, Date.now());
      if (Object.prototype.hasOwnProperty.call(raw, "selectedSessionId")) selectedId = typeof raw.selectedSessionId === "string" ? raw.selectedSessionId : null;
      if (raw.view === "overview" || raw.view === "focus") setView(raw.view);
      if (selectedId && !sessions.some(s => s.id === selectedId)) selectedId = null;
      if (!selectedId && !contentElement && !options.allowEmptyFocus && view === "focus") setView("overview");
      renderLabels(); requestDraw();
    }

    function renderLabels() {
      const reported = sessions.filter(s => s.running).length;
      const quiet = sessions.filter(s => s.running && !s.fresh).length;
      const unknown = sessions.filter(s => !s.saved && (s.live === null || s.phase === "unknown" || s.phase === "disconnected")).length;
      const bits = [`${sessions.length} ${sessions.length === 1 ? "thread" : "threads"}`];
      if (reported) bits.push(`${reported} reported active`);
      if (quiet) bits.push(`${quiet} without a recent signal`);
      if (unknown) bits.push(`${unknown} with unknown or disconnected status`);
      summary.textContent = bits.join(" · ");
      const n = count(raw.pendingApprovals);
      const coverageKnown = raw.approvalCoverageKnown !== false && raw.coverageKnown !== false;
      approvals.textContent = n === null || n === 0 && !coverageKnown ? "Coding permission count unavailable" : n === 0 ? "No pending coding permissions" : `${n} coding permission ${n === 1 ? "request" : "requests"}${coverageKnown ? "" : " · other counts unknown"}`;
      if (!telemetryAvailable && n !== null) approvals.textContent = `Last observed: ${n} coding permission requests · telemetry unavailable`;
      approvals.dataset.pending = String(n !== null && n > 0);
      overviewButton.setAttribute("aria-pressed", String(view === "overview")); focusButton.setAttribute("aria-pressed", String(view === "focus")); focusButton.disabled = !selectedId && !contentElement && !options.allowEmptyFocus;
      [rotateLeft, rotateRight, homeButton, planeSelect].forEach(b => { b.disabled = view === "focus" || !!options.backgroundScene && !options.backgroundScene.setProjection; });
      pauseButton.textContent = reduced.matches ? "Reduced motion" : globalPause() ? "Motion paused" : userPaused ? "Resume motion" : "Pause motion";
      pauseButton.disabled = reduced.matches || globalPause(); pauseButton.setAttribute("aria-pressed", String(userPaused || reduced.matches || globalPause()));
      root.dataset.view = view; root.dataset.motion = motionStopped() ? "paused" : "running";
      if (view === "focus" && !options.backgroundOnly) exclusionElements.set(primaryExclusionOwner,[workspace]); else exclusionElements.delete(primaryExclusionOwner);
      if (options.backgroundScene?.setExclusionElements) options.backgroundScene.setExclusionElements(view === "focus" ? [workspace] : [], primaryExclusionOwner);
      resizeHandle.hidden = options.backgroundOnly || view !== "focus";
      heading.querySelector("h2").textContent = view === "focus" ? "Petrie Loom" : "Strata Observatory";
      root.classList.toggle("spatial-all-threads", showAll);
      empty.hidden = sessions.length > 0;
      const visible = showAll ? sessions : sessions.slice(0, 6);
      const keep = new Set();
      visible.forEach((session, i) => {
        keep.add(session.id);
        let b = snapshots.get(session.id);
        if (!b) {
          b = doc.createElement("button"); b.type = "button"; b.className = "spatial-beacon";
          element("span", "spatial-beacon-mark", "", b); element("span", "spatial-beacon-title", "", b); element("span", "spatial-beacon-status", "", b); element("span", "spatial-beacon-age", "", b);
          b.dataset.sessionId = session.id;
          snapshots.set(session.id, b);
        }
        if (beaconList.children[i] !== b) beaconList.insertBefore(b, beaconList.children[i] || null);
        b.style.setProperty("--beacon-color", PALETTE[stableIndex(session.id, 3)]);
        b.dataset.phase = session.saved ? "saved" : session.pendingApprovals > 0 ? "wait" : session.phase;
        b.dataset.animate = String(session.animated);
        b.setAttribute("aria-pressed", String(session.id === selectedId));
        b.setAttribute("aria-label", `${session.title}. ${session.engine}. ${session.label}. ${session.qualifier ? `${session.qualifier}. ` : ""}${session.ageMs === null ? "Signal time unavailable" : `Last signal ${formatAge(session.ageMs)} ago`}. Open thread.`);
        b.querySelector(".spatial-beacon-title").textContent = session.title;
        b.querySelector(".spatial-beacon-status").textContent = `${session.engine} · ${session.label}`;
        b.querySelector(".spatial-beacon-age").textContent = session.saved ? "Stored conversation" : session.qualifier || (session.ageMs === null ? "Signal time unavailable" : `Last signal ${formatAge(session.ageMs)} ago`);
        b.style.setProperty("--beacon-row", String(i));
      });
      snapshots.forEach((b, id) => { if (!keep.has(id)) { b.remove(); snapshots.delete(id); } });
      moreButton.hidden = sessions.length <= 6 && !showAll;
      moreButton.textContent = showAll ? "Compact view" : `All ${sessions.length} threads`;
      moreButton.setAttribute("aria-expanded", String(showAll));
      if (!contentElement) placeholder.textContent = selectedId ? `${sessions.find(s => s.id === selectedId)?.title || "Selected thread"} · the host owns the working surface.` : "Select a thread to open its working surface.";
    }

    function readHost() {
      if (typeof options.readSessions !== "function") return;
      try { const next = options.readSessions(); if (next && typeof next.then === "function") throw new Error("Session reader must return a synchronous snapshot"); telemetryAvailable = true; refresh(next); }
      catch { notice.hidden = false; notice.textContent = "Thread telemetry unavailable. Last observed labels are retained."; telemetryAvailable = false; sessions = sessions.map(s => ({ ...s, animated: false, fresh: false, running: false, live: null, phase: s.saved ? "idle" : "unknown", label: s.saved ? "Saved history" : "Telemetry unavailable" })); renderLabels(); requestDraw(); }
    }

    function setView(next) {
      if (next !== "overview" && next !== "focus") throw new Error("Unknown spatial view");
      if (next === "focus" && !selectedId && !contentElement && !options.allowEmptyFocus) return;
      if (view !== next) {
        view = next;
        transition = { from: aperture, to: view === "focus" ? 1 : 0, started: nowClock(), duration: view === "focus" ? 340 : 240 };
        if (motionStopped()) { aperture = transition.to; transition = null; }
        options.onViewChange?.(view);
      }
      renderLabels(); requestDraw();
    }

    function setMotionPaused(paused) {
      userPaused = paused === true;
      if (motionStopped() && transition) { aperture = transition.to; transition = null; }
      renderLabels(); requestDraw();
    }

    function projectionState() { return options.backgroundScene?.getState?.().rotation || {yaw,tilt,plane}; }
    function setProjection(next = {}) {
      const previous = projectionState(), value = { yaw: next.yaw ?? previous.yaw, tilt: next.tilt ?? previous.tilt, plane: next.plane ?? previous.plane };
      if (![value.yaw,value.tilt].every(Number.isFinite) || !Number.isInteger(value.plane) || value.plane < 0 || value.plane > 3) throw new Error("Invalid source view");
      value.yaw = clamp(value.yaw,-Math.PI,Math.PI); value.tilt = clamp(value.tilt,-.65,.65);
      if (options.backgroundScene) options.backgroundScene.setProjection?.(value);
      else { yaw = value.yaw; tilt = value.tilt; plane = value.plane; }
      planeSelect.value = String(value.plane); requestDraw();
    }
    function turn(dx, dy) { if (view !== "overview" || options.backgroundOnly) return; const previous = projectionState(); setProjection({yaw:previous.yaw+dx,tilt:previous.tilt+dy}); }
    function resize() {
      const bounds = root.getBoundingClientRect();
      if (bounds.width <= 0 || bounds.height <= 0) { options.backgroundScene?.refreshExclusions?.(); return; }
      width = Math.max(1, bounds.width); height = Math.max(1, bounds.height); dpr = Math.min(win.devicePixelRatio || 1, 2);
      [canvas, spriteCanvas].forEach(c => { c.width = Math.round(width * dpr); c.height = Math.round(height * dpr); }); if (workspaceRect) setWorkspaceRect(workspaceRect); positionResizeHandle(); requestDraw();
    }

    function getWorkspaceRect() { const b = workspace.getBoundingClientRect(), origin = root.getBoundingClientRect(); return {x:b.left-origin.left,y:b.top-origin.top,width:b.width,height:b.height}; }
    function positionResizeHandle() { const r = getWorkspaceRect(); resizeHandle.style.left = `${r.x+r.width-20}px`; resizeHandle.style.top = `${r.y+r.height-20}px`; }
    function setWorkspaceRect(next) {
      const previous = getWorkspaceRect(), input = {...previous,...next};
      if (![input.x,input.y,input.width,input.height].every(Number.isFinite)) throw new Error("Invalid working-surface bounds");
      workspaceRect = boundWorkspaceRect(input,{width,height});
      Object.assign(workspace.style,{left:`${workspaceRect.x}px`,top:`${workspaceRect.y}px`,right:"auto",bottom:"auto",width:`${workspaceRect.width}px`,height:`${workspaceRect.height}px`});
      positionResizeHandle(); options.backgroundScene?.refreshExclusions?.(); requestDraw(); options.onWorkspaceResize?.({...workspaceRect}); return {...workspaceRect};
    }
    function resetWorkspaceRect() { workspaceRect = null; ["left","top","right","bottom","width","height"].forEach(k => workspace.style.removeProperty(k)); positionResizeHandle(); options.backgroundScene?.refreshExclusions?.(); requestDraw(); }
    function setExclusionElements(elements, owner = "host") {
      const rows = (Array.isArray(elements) ? elements : []).filter(e => e?.getBoundingClientRect); if (rows.length) exclusionElements.set(owner,rows); else exclusionElements.delete(owner);
      if (exclusionObserver) { exclusionObserver.disconnect(); const seen = new Set(); exclusionElements.forEach(rows => rows.forEach(e => {if (!seen.has(e)) {seen.add(e);exclusionObserver.observe(e);} })); }
      requestDraw();
    }
    function setExclusions(rects, owner = "host") { if (Array.isArray(rects) && rects.length) exclusionRects.set(owner,rects); else exclusionRects.delete(owner); requestDraw(); }
    function maskRects() {
      const origin = root.getBoundingClientRect(), rects = [...exclusionRects.values()].flat();
      exclusionElements.forEach(elements => elements.forEach(e => {
        if (!e.isConnected || !e.getClientRects().length) return;
        const style = win.getComputedStyle(e); if (style.display === "none" || style.visibility === "hidden") return;
        const b = e.getBoundingClientRect(); rects.push({x:b.left-origin.left,y:b.top-origin.top,width:b.width,height:b.height});
      }));
      return normalizeExclusions(rects,width,height);
    }

    function readingFace() {
      const bounds = workspace.getBoundingClientRect(), origin = root.getBoundingClientRect();
      return { left: bounds.left - origin.left - 5, top: bounds.top - origin.top - 5, right: bounds.right - origin.left + 5, bottom: bounds.bottom - origin.top + 5 };
    }
    function screen(p) {
      const depth = 1 / (1.8 - p.z * 0.18);
      // Anisotropic screen framing fills the canvas; it is separate from the 8D projection.
      return { x: width * .5 + p.x * width * .98 * depth, y: height * .52 - p.y * height * 1.02 * depth, z: p.z, depth };
    }
    function rootPoints() {
      if (!scaffold) return [];
      if (options.backgroundScene?.getRootPositions) {
        const points = options.backgroundScene.getRootPositions(root);
        if (Array.isArray(points) && points.length === 240) return points;
      }
      return scaffold.roots.map(r => screen(projectVector(r.position8, scaffold.projectionBasisQ, plane, yaw, tilt)));
    }
    function getRootPositions(target) {
      if (!scaffold) return [];
      const origin = root.getBoundingClientRect(), relative = target?.getBoundingClientRect?.() || origin;
      return rootPoints().map((p, index) => ({ ...p, index, rootId: scaffold.roots[index].id, x: p.x + origin.left - relative.left, y: p.y + origin.top - relative.top }));
    }
    function getPointPosition(vector, target) {
      if (!scaffold || !Array.isArray(vector) || vector.length !== 8 || vector.some(x => !Number.isFinite(x))) return null;
      if (options.backgroundScene?.getPointPosition) return options.backgroundScene.getPointPosition(vector,target || root);
      const p = screen(projectVector(vector,scaffold.projectionBasisQ,plane,yaw,tilt));
      const origin = root.getBoundingClientRect(), relative = target?.getBoundingClientRect?.() || origin;
      return {...p,x:p.x+origin.left-relative.left,y:p.y+origin.top-relative.top};
    }

    function drawLattice(clock) {
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, width, height);
      if (!scaffold) return;
      const points = rootPoints(), renderBase = options.renderScaffold !== false;
      // Strata are interface sections, not literal E8 facets or semantic dimensions.
      const strataCount = renderBase && !options.backgroundOnly ? 3 : 0;
      for (let i = 0; i < strataCount; i++) {
        const y = height * (0.34 + i * 0.21), open = aperture;
        const x0 = width * .28, x1 = width * .91, skew = height * .045;
        ctx.beginPath(); ctx.moveTo(x0, y - skew); ctx.lineTo(x1, y - skew * 0.35); ctx.lineTo(x1 + width * 0.035, y + skew); ctx.lineTo(x0 + width * 0.035, y + skew * 0.35); ctx.closePath();
        ctx.globalAlpha = 1 - open; ctx.fillStyle = "rgba(23,41,65,.25)"; ctx.fill(); ctx.strokeStyle = ["rgba(183,227,206,.26)", "rgba(196,181,221,.24)", "rgba(238,216,181,.24)"][i]; ctx.lineWidth = 0.8; ctx.stroke();
        // Each depth trace uses an actual source-plane projection, flattened for strata layout.
        const trace = scaffold.roots.map(r => projectVector(r.position8, scaffold.projectionBasisQ, (plane + i + 1) % 4, yaw, tilt));
        ctx.fillStyle = PALETTE[i]; trace.forEach((p, j) => { if (j % 4) return; ctx.beginPath(); ctx.arc(width * .61 + p.x * Math.min(width * .23, height * .34), y - p.y * height * .036, 1.05, 0, Math.PI * 2); ctx.fill(); }); ctx.globalAlpha = 1;
      }
      const chosen = selectedId ? stableIndex(selectedId) : -1;
      const owners = new Map(); sessions.slice(0, 6).forEach(s => { const id = stableIndex(s.id); if (!owners.has(id)) owners.set(id, PALETTE[stableIndex(s.id, 3)]); });
      const activeRoots = new Set();
      (Array.isArray(raw.activations) ? raw.activations : []).forEach(a => { const m = /^e8-root:(\d+)$/.exec(a?.rootId || a?.placement?.root_id || ""); if (m && Number(m[1]) >= 1 && Number(m[1]) <= 240) activeRoots.add(Number(m[1]) - 1); });
      ctx.lineWidth = 0.48;
      scaffold.edges.forEach(([a, b], i) => {
        const emphasis = owners.has(a) || owners.has(b) || activeRoots.has(a) || activeRoots.has(b);
        if (!emphasis && (!renderBase || i % 23 !== 0)) return;
        const p = points[a], q = points[b];
        ctx.beginPath(); ctx.moveTo(p.x, p.y); ctx.lineTo(q.x, q.y);
        ctx.strokeStyle = emphasis ? owners.get(a) || owners.get(b) || "#b7e3ce" : "#91bcae"; ctx.globalAlpha = emphasis ? (renderBase ? .44 : .25) : .40; ctx.stroke();
      });
      ctx.globalAlpha = 1;
      points.map((p, i) => ({ ...p, i })).sort((a, b) => a.z - b.z).forEach(p => {
        const on = owners.has(p.i) || activeRoots.has(p.i);
        if (!renderBase && !on) return;
        ctx.globalAlpha = on ? 0.68 : clamp(0.72 + p.z * 0.16, 0.5, 0.94);
        ctx.fillStyle = owners.get(p.i) || (on ? PALETTE[p.i % 3] : PALETTE[Math.floor(p.i / 80)]);
        ctx.beginPath(); ctx.arc(p.x, p.y, on ? 3 : 1.25 + p.depth * 0.45, 0, Math.PI * 2); ctx.fill();
      }); ctx.globalAlpha = 1;
      // UI tethers are dashed. They connect native labelled buttons to navigation addresses.
      if (!options.backgroundOnly) sessions.slice(0, 6).forEach(s => {
        const button = snapshots.get(s.id); if (!button || !button.getClientRects().length) return;
        const rect = button.getBoundingClientRect(), origin = root.getBoundingClientRect(), p = points[stableIndex(s.id)];
        ctx.strokeStyle = PALETTE[stableIndex(s.id, 3)]; ctx.globalAlpha = .23 * (1 - aperture); ctx.setLineDash([2, 7]); ctx.beginPath(); ctx.moveTo(rect.right - origin.left + 6, rect.top - origin.top + 20); ctx.lineTo(p.x, p.y); ctx.stroke(); ctx.setLineDash([]);
        ctx.globalAlpha = .9; ctx.beginPath(); ctx.arc(p.x, p.y, 5, 0, Math.PI * 2); ctx.stroke();
      }); ctx.globalAlpha = 1;
      if (!options.backgroundOnly && aperture > 0) {
        // Cube faces and inward reach are UI presentation geometry, never E8 facets.
        const face = readingFace(), progress = smooth(aperture), hinge = 16 + progress * Math.min(58, width * .075);
        const corners = [{x:face.left,y:face.top},{x:face.right,y:face.top},{x:face.right,y:face.bottom},{x:face.left,y:face.bottom}];
        const faces = [corners, [corners[0],corners[1],{x:face.right-hinge*.45,y:face.top-hinge*.48},{x:face.left+hinge*.35,y:face.top-hinge*.48}], [corners[1],corners[2],{x:face.right+hinge*.65,y:face.bottom-hinge*.40},{x:face.right+hinge*.65,y:face.top+hinge*.18}]];
        faces.forEach((polygon, i) => { ctx.beginPath(); polygon.forEach((p,j) => j ? ctx.lineTo(p.x,p.y) : ctx.moveTo(p.x,p.y)); ctx.closePath(); ctx.fillStyle = "rgba(20,36,58,.055)"; ctx.fill(); ctx.strokeStyle = PALETTE[i]; ctx.globalAlpha = .16 + progress * .12; ctx.lineWidth = .65; ctx.stroke(); });
        corners.forEach((corner,i) => {
          const nearest = points.map((p,index) => ({p,index,d:Math.hypot(p.x-corner.x,p.y-corner.y)})).sort((a,b) => a.d-b.d).filter(a => a.d > 18).slice(0,3);
          nearest.forEach(({p}) => { ctx.globalAlpha = .12 + .14 * progress; ctx.strokeStyle = PALETTE[i%3]; ctx.setLineDash([2,5]); ctx.beginPath(); ctx.moveTo(p.x,p.y); ctx.lineTo(p.x+(corner.x-p.x)*(.35+.65*progress),p.y+(corner.y-p.y)*(.35+.65*progress)); ctx.stroke(); });
          ctx.setLineDash([]); ctx.globalAlpha = .55; ctx.fillStyle = PALETTE[i%3]; ctx.beginPath(); ctx.arc(corner.x,corner.y,2.5,0,Math.PI*2); ctx.fill();
        }); ctx.globalAlpha = 1;
      }
      // Only source-supplied fine coordinates appear as semantic centers.
      (Array.isArray(raw.activations) ? raw.activations : []).forEach(a => {
        const v = a?.position8 || a?.placement?.position8;
        if (!Array.isArray(v) || v.length !== 8 || v.some(x => !Number.isFinite(x))) return;
        const p = getPointPosition(v);
        ctx.strokeStyle = a.source_mapping_asserted === true ? "#eed8b5" : "#c4b5dd"; ctx.lineWidth = 1.1; ctx.setLineDash(a.source_mapping_asserted === true ? [] : [2, 2]);
        ctx.beginPath(); ctx.arc(p.x, p.y, 5.5, 0, Math.PI * 2); ctx.stroke(); ctx.setLineDash([]);
      });
      // Fresh, observed work gets a small rim marker; never a completion meter.
      if (!motionStopped()) sessions.filter(s => s.animated).slice(0, 6).forEach(s => {
        const p = points[stableIndex(s.id)], r = 4.5 + (Math.sin(clock / 450 + stableIndex(s.id, 13)) + 1) * 1.25;
        ctx.strokeStyle = PALETTE[stableIndex(s.id, 3)]; ctx.globalAlpha = 0.5; ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, Math.PI * 2); ctx.stroke(); ctx.globalAlpha = 1;
      });
      // Erase every registered working area. ClearRect unions overlap safely.
      maskRects().forEach(r => ctx.clearRect(r.x,r.y,r.width,r.height));
    }

    function validateSprite(config) {
      if (!config || typeof config.imageUrl !== "string" || !config.imageUrl || /^(?:javascript|data):/i.test(config.imageUrl)) throw new Error("A local sprite image URL is required");
      const url = new URL(config.imageUrl, doc.baseURI);
      if (url.origin !== new URL(doc.baseURI).origin) throw new Error("Sprite images must be packaged with the app");
      const source = config.frames || config.atlas?.frames;
      const named = source && !Array.isArray(source) ? Object.keys(source) : null;
      const frames = (Array.isArray(source) ? source : named ? named.map(name => ({ ...source[name], name })) : []).map(f => {
        if (f?.rotated || f?.trimmed) throw new Error("Rotated or trimmed sprites need an explicit untrimmed host adapter");
        const r = f?.frame || f;
        return { name: f?.name, x: r?.x, y: r?.y, width: r?.width ?? r?.w, height: r?.height ?? r?.h };
      });
      const indices = new Map(frames.map((f, i) => [f.name, i]));
      const indexOf = value => typeof value === "string" ? indices.get(value) : value;
      if (!frames.length || frames.length > 1000 || frames.some(f => !f || [f.x, f.y, f.width, f.height].some(n => !Number.isFinite(n) || n < 0) || f.width === 0 || f.height === 0)) throw new Error("Sprite frames require valid rectangles");
      const animations = {};
      Object.entries(config.animations || { idle: { frames: [0], fps: 1 } }).forEach(([name, a]) => {
        const sequence = a?.frames?.map(indexOf);
        if (!Array.isArray(sequence) || !sequence.length || sequence.some(i => !Number.isInteger(i) || i < 0 || i >= frames.length)) throw new Error("Sprite animation frame differs");
        animations[name] = { frames: sequence, fps: clamp(Number(a.fps) || 8, 1, 24) };
      });
      const pausedFrame = indexOf(config.pausedFrame ?? (indices.has("idle-neutral") ? "idle-neutral" : 0));
      if (!Number.isInteger(pausedFrame) || pausedFrame < 0 || pausedFrame >= frames.length) throw new Error("Invalid paused sprite frame");
      return { imageUrl: config.imageUrl, frames, animations, pausedFrame, animation: config.animation || (animations.walk ? "walk" : "idle"), size: clamp(Number(config.size) || 136, 40, 180), opacity: clamp(Number(config.opacity) || 0.52, 0.15, 1), traverse: config.traverse !== false };
    }

    function setSprite(config) {
      const generation = ++spriteGeneration; spriteLoaded = false; spriteImage = null; sprite = null; spriteTime = 0;
      if (options.backgroundOnly) return;
      if (!config) { requestDraw(); return; }
      sprite = validateSprite(config);
      const image = new win.Image();
      image.onload = () => {
        if (destroyed || generation !== spriteGeneration) return;
        if (sprite.frames.some(f => f.x + f.width > image.naturalWidth || f.y + f.height > image.naturalHeight)) { notice.textContent = "Joe sprite rectangles do not match the packaged image."; sprite = null; requestDraw(); return; }
        spriteImage = image; spriteLoaded = true; requestDraw();
      };
      image.onerror = () => { if (generation === spriteGeneration) { sprite = null; notice.textContent = "Joe sprite image is unavailable; thread navigation remains available."; requestDraw(); } };
      image.src = sprite.imageUrl;
    }

    function drawSprite(delta) {
      if (!spriteCtx) return;
      spriteCtx.setTransform(dpr, 0, 0, dpr, 0, 0); spriteCtx.clearRect(0, 0, width, height);
      if (!spriteLoaded || !sprite || !spriteImage) return;
      const stopped = motionStopped();
      if (!stopped) spriteTime += delta / 1000;
      const animation = sprite.animations[sprite.animation] || sprite.animations.idle || Object.values(sprite.animations)[0];
      const frameIndex = stopped ? sprite.pausedFrame : animation.frames[Math.floor(spriteTime * animation.fps) % animation.frames.length], frame = sprite.frames[frameIndex];
      const position = peripheralRoute(sprite.traverse ? spriteTime : 0, width, height), { x, y } = position;
      const size = Math.min(sprite.size, height * 0.23), h = size * frame.height / frame.width;
      spriteCtx.globalAlpha = sprite.opacity; spriteCtx.drawImage(spriteImage, frame.x, frame.y, frame.width, frame.height, x - size / 2, y - h, size, h); spriteCtx.globalAlpha = 1;
    }

    function requestDraw() { if (!destroyed && !doc.hidden && raf === null) raf = win.requestAnimationFrame(draw); }
    function draw(clock) {
      raf = null; if (destroyed || doc.hidden) return;
      const delta = lastFrame === null ? 0 : Math.min(clock - lastFrame, 60); lastFrame = clock;
      if (transition) { const t = (clock - transition.started) / transition.duration; aperture = transition.from + (transition.to - transition.from) * smooth(t); if (t >= 1 || motionStopped()) { aperture = transition.to; transition = null; } }
      root.style.setProperty("--aperture-open", String(aperture));
      workspace.inert = view !== "focus"; workspace.setAttribute("aria-hidden", String(view !== "focus"));
      positionResizeHandle();
      drawSprite(delta); drawLattice(clock);
      if (transition || !motionStopped() && (sessions.some(s => s.animated) || spriteLoaded)) requestDraw();
    }

    listen(canvas, "pointerdown", e => { if (view !== "overview" || e.button !== 0) return; drag = e.pointerId; lastPointerX = e.clientX; lastPointerY = e.clientY; canvas.setPointerCapture(e.pointerId); canvas.classList.add("is-turning"); });
    listen(canvas, "pointermove", e => { if (drag !== e.pointerId) return; turn((e.clientX - lastPointerX) * 0.004, (e.clientY - lastPointerY) * 0.003); lastPointerX = e.clientX; lastPointerY = e.clientY; });
    function endDrag(e) { if (drag !== e.pointerId) return; drag = null; canvas.classList.remove("is-turning"); }
    listen(canvas, "pointerup", endDrag); listen(canvas, "pointercancel", endDrag); listen(canvas, "lostpointercapture", endDrag);
    listen(resizeHandle,"pointerdown",e => {if (e.button !== 0) return; resizeDrag={id:e.pointerId,x:e.clientX,y:e.clientY,rect:getWorkspaceRect()}; resizeHandle.setPointerCapture(e.pointerId);e.preventDefault();resizeHandle.focus({preventScroll:true});});
    listen(resizeHandle,"pointermove",e => {if (resizeDrag?.id !== e.pointerId) return;setWorkspaceRect({...resizeDrag.rect,width:resizeDrag.rect.width+e.clientX-resizeDrag.x,height:resizeDrag.rect.height+e.clientY-resizeDrag.y});});
    const finishResize = e => {if (resizeDrag?.id === e.pointerId) resizeDrag=null;};
    ["pointerup","pointercancel","lostpointercapture"].forEach(type => listen(resizeHandle,type,finishResize));
    listen(resizeHandle,"keydown",e => {if (e.key === "Home") {resetWorkspaceRect();e.preventDefault();return;} const delta=e.shiftKey?40:10,r=getWorkspaceRect(),moves={ArrowLeft:{width:r.width-delta},ArrowRight:{width:r.width+delta},ArrowUp:{height:r.height-delta},ArrowDown:{height:r.height+delta}};if(moves[e.key]) {setWorkspaceRect(moves[e.key]);e.preventDefault();}});
    listen(beaconList, "click", async e => {
      const button = e.target.closest(".spatial-beacon"); if (!button || !beaconList.contains(button)) return;
      const id = button.dataset.sessionId;
      try {
        await options.onSelect?.(id, { view: "focus", navigationOnly: true }); if (destroyed) return;
        selectedId = id; raw = { ...raw, selectedSessionId: id }; setView("focus");
        if (typeof options.readSessions === "function") readHost(); else refresh(raw);
      } catch { notice.hidden = false; notice.textContent = "Thread selection was unavailable. The current work remains open."; }
    });
    listen(canvas, "keydown", e => {
      if (e.key === "Escape") { setView("overview"); e.preventDefault(); return; }
      if (view !== "overview") return;
      const moves = { ArrowLeft: [-0.12, 0], ArrowRight: [0.12, 0], ArrowUp: [0, -0.08], ArrowDown: [0, 0.08] };
      if (moves[e.key]) { turn(...moves[e.key]); e.preventDefault(); }
      if (e.key === "Home") { setProjection({yaw:0,tilt:0,plane:0}); e.preventDefault(); }
      if (e.key === "Enter" && selectedId) { setView("focus"); e.preventDefault(); }
    });
    listen(doc, "visibilitychange", () => { if (doc.hidden && raf !== null) { win.cancelAnimationFrame(raf); raf = null; } lastFrame = null; renderLabels(); requestDraw(); });
    listen(doc, "focusin", () => { renderLabels(); requestDraw(); }); listen(doc, "focusout", () => { win.queueMicrotask(() => { if (!destroyed) { renderLabels(); requestDraw(); } }); });
    listen(reduced, "change", () => { if (transition && reduced.matches) { aperture = transition.to; transition = null; } renderLabels(); requestDraw(); });
    const observer = new win.ResizeObserver(resize); observer.observe(root); cleanup.push(() => observer.disconnect());
    const workspaceObserver = new win.ResizeObserver(() => {positionResizeHandle();options.backgroundScene?.refreshExclusions?.();requestDraw();}); workspaceObserver.observe(workspace);cleanup.push(() => workspaceObserver.disconnect());
    exclusionObserver = new win.ResizeObserver(requestDraw); cleanup.push(() => exclusionObserver.disconnect());
    const motionObserver = new win.MutationObserver(() => { renderLabels(); requestDraw(); }); motionObserver.observe(doc.documentElement, { attributes: true, attributeFilter: ["data-motion"] }); cleanup.push(() => motionObserver.disconnect());
    if (!options.backgroundOnly) { ticker = win.setInterval(() => { if (destroyed || doc.hidden) return; if (options.readSessions) readHost(); else refresh(raw); }, 1000); cleanup.push(() => win.clearInterval(ticker)); }

    async function load() {
      try {
        if (options.scaffold) scaffold = validateScaffold(options.scaffold);
        else {
          const response = await win.fetch(options.scaffoldUrl || "assets/e8-scaffold.json"); if (!response.ok) throw new Error("E8 asset unavailable");
          const bytes = await response.arrayBuffer();
          if (!win.crypto?.subtle) throw new Error("E8 asset integrity verification unavailable");
          const digest = Array.from(new Uint8Array(await win.crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
          if (digest !== ASSET_SHA) throw new Error("E8 asset bytes differ from the pinned scaffold");
          scaffold = validateScaffold(JSON.parse(new TextDecoder().decode(bytes)));
        }
        if (destroyed) return;
        notice.textContent = ""; notice.hidden = true; requestDraw();
      } catch { if (!destroyed) { notice.textContent = "E8 reference unavailable. Thread navigation and approval labels remain usable."; scaffold = null; requestDraw(); } }
    }
    refresh({ sessions: options.sessions || [], selectedSessionId: selectedId, pendingApprovals: options.pendingApprovals });
    if (options.readSessions) readHost();
    if (options.sprite && !options.backgroundOnly) setSprite(options.sprite);
    resize(); load();
    return {
      element: root, workspace,
      update: refresh, setView, setSprite, setMotionPaused, mountContent, detachContent, getRootPositions, getPointPosition, setProjection, setExclusionElements, setExclusions, refreshExclusions:requestDraw, getWorkspaceRect, setWorkspaceRect, resetWorkspaceRect,
      getState: () => ({ view, selectedSessionId: selectedId, paused: userPaused, reducedMotion: reduced.matches, geometryReady: scaffold !== null, rotation: projectionState() }),
      destroy() { if (destroyed) return; destroyed = true; spriteGeneration++; if (raf !== null) win.cancelAnimationFrame(raf); cleanup.forEach(fn => fn()); options.backgroundScene?.setExclusionElements?.([],primaryExclusionOwner); detachContent(true); root.remove(); }
    };
  }

  function attachBackground(container, options = {}) { return attach(container, { ...options, backgroundOnly:true, renderScaffold:true, sessions:[], selectedSessionId:null, view:"overview", sprite:null, readSessions:undefined, backgroundScene:undefined }); }
  const api = Object.freeze({ attach, attachBackground, validateScaffold, rotate8, projectVector, stableIndex, normalizeSnapshots, normalizeExclusions, boundWorkspaceRect, peripheralRoute, formatAge, smooth, constants: Object.freeze({ PLANE_SHA, ASSET_SHA, STALE_MS }) });
  global.BombSpatialWorld = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
