/* Read-only spatial projection of Bomb Code's existing state and events.
   Selection delegates to the native UI; this module owns no coding commands. */
(function (global) {
  "use strict";
  const ACTIVE = new Set(["send", "think", "tools", "reply"]);
  const TERMINAL_TOOL = /complete|done|success|fail|error|denied|reject|cancel/i;
  const valueOf = (map, id) => map && typeof map.get === "function" ? map.get(id) : undefined;
  const timeOf = (value) => {
    const n = typeof value === "number" ? value : typeof value === "string" ? Date.parse(value) : NaN;
    return Number.isFinite(n) && n > 0 ? n : null;
  };
  const sessionIdOf = event => event?.session_id || event?.sessionId || null;

  function createTelemetry() {
    const sessions = new Map();
    function observe(event, now = Date.now()) {
      const id = sessionIdOf(event);
      if (typeof id !== "string" || !id || !event || typeof event !== "object") return false;
      let item = sessions.get(id);
      if (!item) {
        item = { phase: null, signalAt: null, status: null, pending: new Set(), settled: new Set(), discardTranscriptApprovals: false, tools: new Set(), approvalsKnown: false, toolsKnown: false, finishedAt: null, uncertain: false, sessionClosed: false };
        sessions.set(id, item);
        if (sessions.size > 512) sessions.delete(sessions.keys().next().value);
      }
      const type = String(event.type || "").replace(/[A-Z]/g, c => `_${c.toLowerCase()}`);
      const signal = () => { item.signalAt = timeOf(event.at || event.event?.at || event.payload?.at) || now; };
      if (type === "prompt_finished") {
        const reason = event.stop_reason || event.stopReason;
        const endedNormally = reason === "end_turn" || reason === "mock";
        item.finishedAt = endedNormally ? (timeOf(event.at) || now) : null;
        item.sessionClosed = false;
        item.uncertain = item.finishedAt === null;
        item.phase = item.finishedAt ? "done" : /cancel|error|fail/.test(String(reason)) ? "error" : "unknown";
        if (endedNormally) { item.tools.clear(); item.toolsKnown = true; item.approvalsKnown = true; }
        signal();
      } else if (type === "session_created") {
        item.tools.clear(); item.toolsKnown = true; item.approvalsKnown = true; item.uncertain = false; signal();
      } else if (type === "session_status_changed") {
        const next = String(event.status || "").toLowerCase().replace(/_/g, "");
        if (next === "idle") {
          if (item.status === "running" && !item.finishedAt) item.uncertain = true;
        } else if (next === "running") {
          // Approval resolution resumes the same turn; it is not completion.
          item.finishedAt = null; item.sessionClosed = false; item.uncertain = false;
        }
        item.status = next;
        if (/cancel/.test(next) && item.finishedAt && !item.uncertain && !item.pending.size) { item.sessionClosed = true; item.phase = null; }
        else if (/fail|cancel/.test(next)) { item.phase = "error"; item.finishedAt = null; item.sessionClosed = false; item.uncertain = true; }
        else if (/wait|approv/.test(next)) item.phase = "wait";
        signal();
      } else if (type === "tool_call") {
        const tool = event.event || event;
        if (tool.id != null) {
          if (TERMINAL_TOOL.test(String(tool.status || ""))) item.tools.delete(String(tool.id));
          else item.tools.add(String(tool.id));
        }
        item.phase = item.tools.size ? "tools" : null;
        item.finishedAt = null; item.sessionClosed = false; signal();
      } else if (type === "approval_required") {
        const requestId = event.request_id || event.requestId;
        if (!(event.auto_approved ?? event.autoApproved) && requestId) {
          item.settled.delete(String(requestId)); item.pending.add(String(requestId));
        }
        if (item.pending.size) item.phase = "wait";
        signal();
      } else if (type === "approval_resolved") {
        const requestId = String(event.request_id || event.requestId || "");
        if (requestId) item.settled.add(requestId);
        item.pending.delete(requestId);
        if (!item.pending.size && item.phase === "wait") item.phase = null;
        signal();
      } else if (type === "agent_message") {
        const text = String(event.text || ""), normalized = text.trim().toLowerCase();
        if (!text || /^(🧠|📜|⚙|wrote )/.test(text) || normalized === "turn complete" || /^(prompt sent|still generating after|\[local\/mock\])/.test(normalized)) return false;
        item.phase = String(event.text || "").startsWith("💭") ? "think" : "reply";
        item.finishedAt = null; item.sessionClosed = false; signal();
      } else if (type === "error") {
        item.phase = "error"; item.finishedAt = null; item.sessionClosed = false; item.uncertain = true; signal();
      } else if (type === "session_cancelled") {
        const closedAfterTurn = !!item.finishedAt && !item.uncertain && !item.pending.size && item.phase !== "error";
        item.phase = closedAfterTurn ? null : "error"; item.status = "cancelled"; item.sessionClosed = closedAfterTurn;
        if (!closedAfterTurn) { item.finishedAt = null; item.uncertain = true; }
        item.pending.forEach(requestId => item.settled.add(requestId));
        item.pending.clear(); item.discardTranscriptApprovals = true; item.approvalsKnown = true; signal();
      } else if (type === "raw" && event.payload?.channel === "term" && /session\/prompt still open after/i.test(event.payload.line || "")) {
        item.uncertain = true; item.finishedAt = null; item.phase = "unknown"; signal();
      } else return false;
      return true;
    }
    return { sessions, observe };
  }

  function projectSessions(source = {}, telemetry = createTelemetry(), now = Date.now()) {
    const rows = Array.isArray(source.sessions) ? source.sessions : [];
    return rows.filter(row => row && typeof row.id === "string").map(row => {
      const id = row.id, observed = telemetry.sessions.get(id);
      const presence = valueOf(source.presenceBySession, id) || (id === source.selectedSession ? source.turn : null);
      const subsequentActivity = ACTIVE.has(presence?.phase) && timeOf(presence?.lastSignalAt) > (observed?.finishedAt || Infinity) && !presence.completedAt;
      const endedAt = subsequentActivity ? null : observed?.finishedAt;
      const closedAfterTurn = !subsequentActivity && observed?.sessionClosed;
      const status = observed?.status || String(row.status || "").toLowerCase().replace(/_/g, "");
      const saved = row.live === false || status === "saved";
      const live = saved ? false : row.live === true ? true : null;
      const open = valueOf(source.openToolsBySession, id);
      const toolCount = saved ? 0 : observed?.uncertain ? null : subsequentActivity && open?.size > 0 ? open.size : observed?.toolsKnown ? observed.tools.size : open?.size > 0 ? open.size : observed?.tools.size > 0 ? observed.tools.size : null;
      const pending = new Set(observed?.pending || []);
      const transcript = valueOf(source.transcriptBySession, id);
      if (!saved && !observed?.discardTranscriptApprovals && Array.isArray(transcript)) transcript.forEach(entry => {
        const requestId = String(entry.meta?.requestId || "");
        if (entry.role === "approval" && requestId && !entry.meta.resolved && entry.meta.options?.length && !observed?.settled.has(requestId)) pending.add(requestId);
      });
      const permissionsKnown = observed?.approvalsKnown && (!observed.uncertain || observed.discardTranscriptApprovals);
      const approvalCount = saved ? 0 : pending.size ? pending.size : permissionsKnown && !/wait|approv/.test(status) ? 0 : null;
      const approvalCoverageKnown = saved || !!(permissionsKnown && (!/wait|approv/.test(status) || pending.size > 0));
      let phase = "unknown";
      if (saved) phase = "idle";
      else if (observed?.phase === "error" || (!subsequentActivity && !closedAfterTurn && /fail|cancel/.test(status))) phase = "error";
      else if (observed?.uncertain) phase = "unknown";
      else if (live === true) {
        if (subsequentActivity) phase = presence.phase;
        else if ((!closedAfterTurn && /fail|cancel/.test(status)) || observed?.phase === "error") phase = "error";
        else if (pending.size || /wait|approv/.test(status)) phase = "wait";
        else if (endedAt && now >= endedAt && now - endedAt < 1500) phase = "done";
        else if (closedAfterTurn || status === "idle" || status === "completed") phase = subsequentActivity ? presence.phase : "idle";
        else if (status === "running") {
          phase = toolCount > 0 ? "tools" : ACTIVE.has(observed?.phase) ? observed.phase : ACTIVE.has(presence?.phase) ? presence.phase : "think";
          if (phase === "tools" && toolCount === 0) phase = "think";
        }
      }
      // presence.phase=done is deliberately ignored: older UI celebrates Idle.
      const signalAt = Math.max(observed?.signalAt || 0, timeOf(presence?.lastSignalAt) || 0) || null;
      return { id, title: row.label || `Thread ${id.slice(0, 8)}`, engine: row.backend || "Engine unreported", project: row.projectRoot || row.project_root || row.cwd || "Project unreported", phase, live, savedOnly: saved, sessionClosed: !!closedAfterTurn, turnEndedAt: endedAt || null, lastSignalAt: signalAt, toolsActive: toolCount, pendingApprovals: approvalCount, approvalCoverageKnown };
    });
  }

  function summarizeApprovals(sessions) {
    const pendingApprovals = sessions.reduce((sum, session) => sum + (Number.isInteger(session.pendingApprovals) && session.pendingApprovals >= 0 ? session.pendingApprovals : 0), 0);
    const approvalCoverageKnown = sessions.every(session => session.approvalCoverageKnown === true);
    return { pendingApprovals, approvalCoverageKnown, coverageKnown: approvalCoverageKnown };
  }

  function activationsOf(detail, source = {}) {
    if (detail?.schema !== "bomb-code/joe-visual-state/v1" || detail.status === "invalidated") return [];
    const result = detail.result;
    if (result?.schema !== "bomb-code/joe-result/v1" || !["grounded-model-proposal", "clarification-needed-proposal"].includes(result.status)) return [];
    if ((result.threadId || null) !== (source.selectedSession || null)) return [];
    if (result.authority?.toolsDispatched !== false || result.authority?.approvalsGranted !== false || result.authority?.memoryCommitted !== false) return [];
    const readings = result.interpretation?.binding?.readings;
    return (Array.isArray(readings) ? readings : []).flatMap(reading => (Array.isArray(reading.e8_activations) ? reading.e8_activations : []).filter(activation => {
      const vector = activation.position8 || activation.placement?.position8;
      return Array.isArray(vector) && vector.length === 8 && vector.every(Number.isFinite);
    }));
  }

  function atlasSprite(atlas, imageUrl) {
    const frames = [], indexes = new Map();
    Object.entries(atlas?.frames || {}).forEach(([name, data]) => {
      const f = data?.frame;
      if (!f || data.rotated === true || [f.x, f.y, f.w, f.h].some(n => !Number.isFinite(n) || n < 0) || !f.w || !f.h) return;
      indexes.set(name, frames.length); frames.push({ x: f.x, y: f.y, width: f.w, height: f.h });
    });
    if (!frames.length) throw new Error("Packaged Joe atlas has no usable frames");
    let walk = [1, 2, 3, 4, 5, 6].map(n => indexes.get(`walk-${n}`)).filter(Number.isInteger);
    if (!walk.length) walk = [1, 2, 3, 4].map(n => indexes.get(`front-walk-${n}`)).filter(Number.isInteger);
    const run = [1, 2, 3].map(n => indexes.get(`run-${n}`)).filter(Number.isInteger);
    const idle = indexes.get("idle-neutral") ?? indexes.get("front-idle") ?? 0;
    const idleFrames = ["idle-neutral", "idle-inhale", "idle-neutral", "idle-shift"].map(name => indexes.get(name)).filter(Number.isInteger);
    return { imageUrl, frames, pausedFrame: idle, animations: { idle: { frames: idleFrames.length ? idleFrames : [idle], fps: 4 }, walk: { frames: walk.length ? walk : [idle], fps: 10 }, run: { frames: run.length ? run : walk.length ? walk : [idle], fps: 14 } }, animation: run.length ? "run" : walk.length ? "walk" : "idle", size: 170, opacity: 0.85, traverse: true };
  }

  function attach(container, options = {}) {
    if (!global.BombSpatialWorld) throw new Error("Spatial renderer is unavailable");
    const doc = container.ownerDocument, win = doc.defaultView;
    const getState = options.getState || (() => typeof state !== "undefined" ? state : {});
    const choose = options.selectSession || (id => typeof selectSession === "function" ? selectSession(id) : Promise.reject(new Error("Native thread selection is unavailable")));
    const activate = options.activateView || (name => { if (typeof activateView === "function") activateView(name); });
    const content = options.contentElement || doc.getElementById("view-chat");
    const telemetry = createTelemetry(), cleanup = [];
    let scene, destroyed = false, activations = [], unlisten = null, worldActive = false, pauseBeforeHide = false;
    function snapshot() {
      const source = getState(), sessions = projectSessions(source, telemetry);
      return { sessions, selectedSessionId: source.selectedSession || null, ...summarizeApprovals(sessions), activations };
    }
    scene = global.BombSpatialWorld.attach(container, {
      scaffoldUrl: options.scaffoldUrl || "assets/e8-scaffold.json", readSessions: snapshot, view: "overview", allowEmptyFocus: true,
      backgroundScene: options.backgroundScene, renderScaffold: options.renderScaffold,
      selectedSessionId: getState().selectedSession || null,
      onSelect: async id => {
        if (!Array.isArray(getState().sessions) || !getState().sessions.some(row => row.id === id)) throw new Error("This thread is no longer available");
        await choose(id); activate("spatial"); scene.setView("focus");
      },
      onViewChange: view => { options.onViewChange?.(view); doc.dispatchEvent(new win.CustomEvent("bomb-code:spatial-presentation", { detail: { view, navigationOnly: true } })); },
      onPauseChange: paused => options.onPauseChange?.(paused),
    });
    function listen(target, type, callback) { target.addEventListener(type, callback); cleanup.push(() => target.removeEventListener(type, callback)); }
    function setHostView(name) {
      const wasActive = worldActive;
      worldActive = name === "spatial";
      if (worldActive && !wasActive) scene.setMotionPaused(pauseBeforeHide);
      else if (!worldActive) {
        if (wasActive) pauseBeforeHide = scene.getState().paused;
        scene.setMotionPaused(true);
      }
      if (worldActive && content) {
        if (scene.mountContent) scene.mountContent(content);
        content.classList.add("active");
      } else if (scene.detachContent) {
        scene.detachContent(true);
        if (content) content.classList.toggle("active", name === "chat");
      }
      scene.element.hidden = !worldActive;
      if (worldActive) scene.update(snapshot());
    }
    listen(doc, "bomb-code:view-selected", event => setHostView(event.detail?.view || event.detail?.name));
    listen(doc, "bomb-code:thread-selected", () => { activations = []; win.queueMicrotask(() => { if (!destroyed) scene.update(snapshot()); }); });
    listen(doc, "bomb-code:joe-interpretation", event => { activations = activationsOf(event.detail, getState()); scene.update(snapshot()); });
    function observeControlEvent(event) { if (telemetry.observe(event)) scene.update(snapshot()); }
    const bus = options.listenEvents || (win.__TAURI__?.event?.listen ? callback => win.__TAURI__.event.listen("control-event", event => callback(event.payload)) : null);
    if (bus) Promise.resolve(bus(observeControlEvent)).then(dispose => { if (destroyed && typeof dispose === "function") dispose(); else unlisten = dispose; }).catch(() => { options.onTelemetryError?.(); });
    if (options.sprite) scene.setSprite(options.sprite);
    else if (options.loadSprite !== false) win.fetch(options.atlasUrl || "assets/joe/wizard-joe-hd.json").then(response => { if (!response.ok) throw new Error("Joe atlas unavailable"); return response.json(); }).then(atlas => { if (!destroyed) scene.setSprite(atlasSprite(atlas, options.imageUrl || "assets/joe/wizard-joe-hd.webp")); }).catch(error => options.onSpriteError?.(error));
    setHostView(options.initialHostView || (doc.getElementById("view-spatial")?.classList.contains("active") ? "spatial" : "chat"));
    return { scene, snapshot, observeControlEvent, setHostView, setView: view => scene.setView(view), destroy() { if (destroyed) return; destroyed = true; if (typeof unlisten === "function") unlisten(); cleanup.forEach(dispose => dispose()); scene.destroy(); } };
  }
  const api = Object.freeze({ attach, createTelemetry, projectSessions, summarizeApprovals, activationsOf, atlasSprite });
  global.BombSpatialHost = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
