/* Display-only motion. Native E8 coordinates and projection yaw stay untouched. */
(function (global) {
  "use strict";

  const MAX_DT = 50;

  function activityTarget(sessions, selectedId) {
    const rows = Array.isArray(sessions) ? sessions : [];
    const pending = rows.some(row => Number(row?.pendingApprovals) > 0);
    const seen = new Set();
    rows.forEach(row => {
      if (!row || typeof row.id !== "string" || seen.has(row.id)) return;
      if (row.animated !== true || row.saved === true || row.savedOnly === true || row.live === false || row.phase === "error") return;
      seen.add(row.id);
    });
    const n = Math.min(4, seen.size);
    let amount = pending ? 0 : Math.min(1, n / 4);
    const selected = rows.find(row => row && row.id === selectedId);
    const unknown = !selected || selected.live == null || selected.phase === "unknown" || selected.phase === "disconnected";
    if (!pending && !unknown && selected.animated === true) amount = Math.max(amount, 0.35);
    const counted = pending ? 0 : n;
    return {
      n: counted,
      a: amount,
      primaryDegPerSec: 6 + 3 * amount,
      secondaryAmpDeg: 6 * amount + 8 * amount * amount,
      tertiaryAmpDeg: 5 * Math.min(1, Math.max(0, counted - 1) / 3),
      pendingApproval: pending,
    };
  }

  function wrap360(degrees) {
    const turned = degrees % 360;
    return turned < 0 ? turned + 360 : turned;
  }

  function ease(current, target, dt, tau) {
    if (!(dt > 0) || !(tau > 0)) return current;
    return current + (target - current) * (1 - Math.exp(-dt / tau));
  }

  function poseOf(state) {
    return {
      primaryDeg: Number.isFinite(state?.primaryDeg) ? state.primaryDeg : 0,
      secondaryAmp: Number.isFinite(state?.secondaryAmp) ? state.secondaryAmp : 0,
      tertiaryAmp: Number.isFinite(state?.tertiaryAmp) ? state.tertiaryAmp : 0,
      secondaryPhase: Number.isFinite(state?.secondaryPhase) ? state.secondaryPhase : 0,
      tertiaryPhase: Number.isFinite(state?.tertiaryPhase) ? state.tertiaryPhase : 0,
    };
  }

  function step(state, dtMs, target, flags = {}) {
    const pose = poseOf(state);
    if (flags.hidden) return pose;
    if (flags.reduced) return { primaryDeg: 0, secondaryAmp: 0, tertiaryAmp: 0, secondaryPhase: 0, tertiaryPhase: 0 };
    if (flags.paused) return pose;
    const dt = Math.min(MAX_DT, Math.max(0, Number(dtMs) || 0));
    const typing = flags.typing === true;
    const speed = typing ? 6 : Number(target?.primaryDegPerSec) || 6;
    const secondaryTarget = typing ? 0 : Number(target?.secondaryAmpDeg) || 0;
    const tertiaryTarget = typing ? 0 : Number(target?.tertiaryAmpDeg) || 0;
    const rising = secondaryTarget > pose.secondaryAmp + 1e-9 || tertiaryTarget > pose.tertiaryAmp + 1e-9;
    const tau = rising ? 800 : 2500;
    const phase = typing ? 0 : dt / 1000;
    return {
      primaryDeg: wrap360(pose.primaryDeg + speed * (dt / 1000)),
      secondaryAmp: ease(pose.secondaryAmp, secondaryTarget, dt, tau),
      tertiaryAmp: ease(pose.tertiaryAmp, tertiaryTarget, dt, tau),
      secondaryPhase: pose.secondaryPhase + phase * 0.35,
      tertiaryPhase: pose.tertiaryPhase + phase * 0.22,
    };
  }

  function orientPoint(point, pose) {
    const source = poseOf(pose);
    const yaw = source.primaryDeg * Math.PI / 180;
    const tilt = source.secondaryAmp * Math.sin(source.secondaryPhase) * Math.PI / 180;
    const roll = source.tertiaryAmp * Math.sin(source.tertiaryPhase) * Math.PI / 180;
    const cy = Math.cos(yaw), sy = Math.sin(yaw);
    let x = cy * point.x + sy * (point.z || 0);
    let z = -sy * point.x + cy * (point.z || 0);
    let y = point.y;
    const ct = Math.cos(tilt), st = Math.sin(tilt);
    const y2 = ct * y - st * z;
    const z2 = st * y + ct * z;
    const cr = Math.cos(roll), sr = Math.sin(roll);
    return { ...point, x: cr * x - sr * y2, y: sr * x + cr * y2, z: z2 };
  }

  function nearestIndex(points, x, y) {
    let best = -1, distance = Infinity;
    (points || []).forEach((point, index) => {
      const next = (point.x - x) ** 2 + (point.y - y) ** 2;
      if (next < distance) { distance = next; best = index; }
    });
    return best;
  }

  function frameDelta(last, now, hidden) {
    if (hidden || last == null || !Number.isFinite(now)) return { dt: 0, last: null };
    return { dt: Math.min(MAX_DT, Math.max(0, now - last)), last: now };
  }

  function createClock({ now, hidden, running, onTick, intervalMs = 33 } = {}) {
    let timer = null, last = null, stopped = false;
    const clear = () => { if (timer !== null) { clearTimeout(timer); timer = null; } };
    const arm = () => { if (!stopped && timer === null) timer = setTimeout(tick, intervalMs); };
    function tick() {
      timer = null;
      if (stopped) return;
      if (hidden?.()) { last = null; return; }
      const time = now();
      const dt = last === null ? 0 : time - last;
      last = time;
      onTick(Math.min(MAX_DT, Math.max(0, dt)));
      if (running?.() !== false) arm();
    }
    return {
      start() { stopped = false; last = null; clear(); arm(); },
      resume() { last = null; if (!stopped) arm(); },
      pause() { clear(); last = null; },
      stop() { stopped = true; clear(); last = null; },
    };
  }

  const api = Object.freeze({ activityTarget, step, orientPoint, nearestIndex, frameDelta, createClock, constants: Object.freeze({ MAX_DT, IDLE_DEG_PER_SEC: 6 }) });
  global.BombStudioMotion = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
