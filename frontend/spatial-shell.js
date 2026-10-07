// Entry coordinator. Native commands remain in app.js.
(() => {
  document.documentElement.dataset.view = "spatial";
  document.documentElement.dataset.joeCube = "on";
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
  const motionToggle = document.getElementById("toggle-visual-motion");
  const background = BombSpatialWorld.attachBackground(document.getElementById("spatial-background"), { frameIntervalMs: 33 });
  let entry = null, syncing = false;
  const host = BombSpatialHost.attach(document.getElementById("view-spatial"), {
    backgroundScene: background, renderScaffold: false, loadSprite: false, frameIntervalMs: 33,
    getState: () => state, selectSession, activateView,
    contentElement: document.getElementById("view-chat"), initialHostView: "spatial",
    onSpriteError: error => console.warn("Joe animation unavailable", error),
    onPauseChange(paused) {
      if (!motionToggle || motionToggle.checked === paused) return;
      syncing = true;
      motionToggle.checked = paused;
      motionToggle.dispatchEvent(new Event("change"));
      syncing = false;
      syncClock();
    },
    onWorkspaceResize: () => entry?.noteLayout(),
  });
  host.setView("focus");
  const faces = BombSpatialFaces.attach({ host, background, getState: () => state, selectSession, activateView });
  BombSpatialPanels.attach({ background, scene: host.scene });
  const joe = BombJoeCompanion.attach({ getState: () => state, cube: true });
  const flags = () => {
    const active = document.activeElement;
    return {
      typing: !!active && (active.tagName === "INPUT" || active.tagName === "TEXTAREA" || active.isContentEditable),
      paused: motionToggle?.checked === true,
      reduced: reduced.matches,
      hidden: document.hidden,
    };
  };
  const exclusions = () => [...document.querySelectorAll("#composer, .approval-card, dialog, .spatial-panel-cube, #studio-invitation")].flatMap(node => {
    if (node.hidden) return [];
    const rect = node.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 ? [{ x: rect.x, y: rect.y, width: rect.width, height: rect.height }] : [];
  });
  let pose = { primaryDeg: 0, secondaryAmp: 0, tertiaryAmp: 0, secondaryPhase: 0, tertiaryPhase: 0 };
  const publish = () => { background.setDisplayOrientation?.(pose); background.requestDraw?.(); };
  const clock = BombStudioMotion.createClock({
    now: () => performance.now(),
    hidden: () => document.hidden,
    running: () => !document.hidden && !reduced.matches && motionToggle?.checked !== true,
    onTick(dt) {
      const sample = host.snapshot();
      const motionFlags = flags();
      pose = BombStudioMotion.step(pose, dt, BombStudioMotion.activityTarget(sample.sessions, state.selectedSession || null), motionFlags);
      publish();
      joe.setExclusions?.(exclusions());
      joe.tick?.(dt, motionFlags);
    },
  });
  function syncClock() {
    if (document.hidden || reduced.matches || motionToggle?.checked) {
      if (reduced.matches) pose = BombStudioMotion.step(pose, 0, { primaryDegPerSec: 6, secondaryAmpDeg: 0, tertiaryAmpDeg: 0 }, { reduced: true });
      publish();
      clock.pause();
      return;
    }
    clock.resume();
  }
  motionToggle?.addEventListener("change", () => {
    if (!syncing) host.scene.setMotionPaused(motionToggle.checked === true);
    syncClock();
  });
  document.addEventListener("visibilitychange", syncClock);
  reduced.addEventListener("change", syncClock);
  document.addEventListener("bomb-code:thread-selected", () => {
    if (document.documentElement.dataset.view === "spatial") host.setView("focus");
    entry?.noteLayout();
  });
  document.addEventListener("pointerup", () => entry?.noteLayout());
  entry = BombStudioEntry.attach({
    activateView,
    selectSession,
    setFocus: view => host.setView(view),
    focusFaces: () => document.querySelector(".spatial-faces-toolbar select")?.focus(),
    parkJoe: () => joe.park?.(),
    layout: () => faces.layout?.() || { primary: host.scene.getWorkspaceRect?.(), secondary: [] },
    work: () => {
      const session = (state.sessions || []).find(row => row.id === state.selectedSession);
      return { projectId: session?.projectRoot || session?.project_root || session?.cwd || state.cwd || null, threadId: state.selectedSession || null };
    },
    setWorkspaceRect: rect => host.scene.setWorkspaceRect?.(rect),
    placeFace: (id, rect) => faces.placeFace?.(id, rect),
    bounds: () => ({ width: window.innerWidth, height: window.innerHeight }),
    clamp: (rect, bounds) => BombSpatialWorld.boundWorkspaceRect(rect, { ...bounds, margin: 16, top: 125, bottom: 60 }),
  });
  if ((entry.mode === "first" || entry.mode === "upgrade") && !reduced.matches) {
    window.requestAnimationFrame(() => joe.travelTo?.(entry.element.getBoundingClientRect(), exclusions()));
  }
  if (motionToggle?.checked) host.scene.setMotionPaused(true);
  syncClock();
})();
