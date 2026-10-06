// Local appearance preference only. Never changes execution or permissions.
(() => {
  const key = "bomb-code.pause-visual-motion";
  const toggle = document.getElementById("toggle-visual-motion");
  if (!toggle) return;
  let paused = false;
  try { paused = localStorage.getItem(key) === "true"; } catch { /* Restricted storage. */ }
  toggle.checked = paused;
  const apply = () => {
    document.documentElement.dataset.motion = paused || document.hidden ? "paused" : "running";
  };
  toggle.addEventListener("change", () => {
    paused = toggle.checked;
    try { localStorage.setItem(key, String(paused)); } catch { /* Still applies this run. */ }
    apply();
  });
  document.addEventListener("visibilitychange", apply);
  apply();
})();
