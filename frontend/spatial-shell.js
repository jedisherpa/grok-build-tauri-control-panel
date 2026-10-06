// Persistent app appearance; native commands remain in app.js.
(() => {
  document.documentElement.dataset.view = "spatial";
  const background = BombSpatialWorld.attachBackground(document.getElementById("spatial-background"));
  const host = BombSpatialHost.attach(document.getElementById("view-spatial"), {
    backgroundScene: background, renderScaffold: false, loadSprite: false,
    getState: () => state, selectSession, activateView,
    contentElement: document.getElementById("view-chat"), initialHostView: "spatial",
    onSpriteError: error => console.warn("Joe animation unavailable", error),
  });
  host.setView("focus");
  BombSpatialFaces.attach({ host, background, getState: () => state, selectSession, activateView });
  BombSpatialPanels.attach({ background, scene: host.scene });
  BombJoeCompanion.attach({ getState: () => state });
  document.documentElement.dataset.view = "spatial";
  const motion = document.getElementById("toggle-visual-motion");
  const applyMotion = () => host.scene.setMotionPaused(motion?.checked === true);
  motion?.addEventListener("change", applyMotion);
  document.addEventListener("bomb-code:thread-selected", () => {
    if (document.documentElement.dataset.view === "spatial") host.setView("focus");
  });
  applyMotion();
})();
