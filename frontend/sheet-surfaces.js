/* Sheets held by the lattice. Corners are the controls and the thread ends.
   θ turns, ∠ tips, ⊥ lifts or sets back, λ shrinks. Pause and reduced motion stop the wind. */
(function (global) {
  "use strict";
  const MARKS = Object.freeze(["θ", "∠", "⊥", "λ"]);
  const AXES = Object.freeze(["turn", "angle", "lift", "shrink"]);
  const LABELS = Object.freeze({
    turn: "Turn this surface sideways",
    angle: "Tip this surface",
    lift: "Lift or set this surface back",
    shrink: "Shrink this surface",
  });
  const EMERGE_AT = Object.freeze([0.08, 0.32, 0.56, 0.78]);
  const HELD = Object.freeze({ turn: 0, angle: 0, shrink: 0, lift: 1 });
  const RECESSED = Object.freeze({ turn: 0, angle: 0, shrink: 0, lift: 0 });
  const WIND_LEEWAY = 16;

  function clamp01(value) {
    const n = Number(value);
    if (!Number.isFinite(n)) return 0;
    return Math.min(1, Math.max(0, n));
  }
  function lerp(from, to, t) { return from + (to - from) * t; }
  function poseOf(pose) {
    return {
      turn: clamp01(pose?.turn),
      angle: clamp01(pose?.angle),
      shrink: clamp01(pose?.shrink),
      lift: clamp01(pose?.lift),
    };
  }

  /** Ease toward the target. Reduced motion snaps. */
  function easePose(shown, target, dt, snap) {
    const from = poseOf(shown), to = poseOf(target);
    if (snap) return to;
    const blend = 1 - Math.exp(-Math.max(0, Number(dt) || 0) / 0.36);
    const step = (a, b) => {
      const next = a + (b - a) * blend;
      return Math.abs(next - b) < 0.004 ? b : next;
    };
    return { turn: step(from.turn, to.turn), angle: step(from.angle, to.angle), shrink: step(from.shrink, to.shrink), lift: step(from.lift, to.lift) };
  }

  function windDrift(time, index, lift, still) {
    if (still) return { x: 0, y: 0 };
    const amp = lerp(0.2, 1, clamp01(lift));
    const phase = index * 1.7;
    const sway = Math.sin(time * 0.32 + phase) * 0.75 + Math.sin(time * 0.71 + phase * 1.4) * 0.25;
    const bob = Math.cos(time * 0.26 + phase * 0.8) * 0.7 + Math.sin(time * 0.58 + phase) * 0.3;
    return { x: sway * 4.5 * amp, y: bob * 2.8 * amp };
  }

  /** Cloth corners for a cell. Still sheets do not flutter. Live corners stay within the wind leeway. */
  function sheetCorners(rect, pose, time, index, still) {
    const held = poseOf(pose);
    const short = rect.height <= 72;
    const sideW = rect.width >= rect.height ? Math.min(rect.width, Math.max(72, rect.height * 1.35)) : rect.width;
    const sideH = rect.width >= rect.height ? rect.height : Math.min(rect.height, Math.max(64, rect.width * 0.56));
    let width = lerp(rect.width, sideW, held.turn);
    let height = lerp(rect.height, sideH, held.turn);
    const scale = lerp(0.9, 1, held.lift) * lerp(1, 0.78, held.shrink) * lerp(1, 0.94, held.angle);
    width = Math.min(rect.width, Math.max(48, width * scale));
    height = Math.min(rect.height, Math.max(short && held.turn < 0.5 ? rect.height : 28, height * scale));
    if (short && held.turn < 0.5) height = rect.height;
    const drift = windDrift(time, index, held.lift, still);
    const x = rect.x + (rect.width - width) / 2 + drift.x;
    const y = rect.y + (rect.height - height) / 2 + drift.y;
    const corners = [
      { x, y },
      { x: x + width, y },
      { x, y: y + height },
      { x: x + width, y: y + height },
    ];
    corners[0].y -= 10 * held.angle;
    corners[1].y -= 7 * held.angle;
    corners[0].x -= 5 * held.angle;
    corners[1].x += 5 * held.angle;
    const amp = still ? 0 : lerp(0.2, 1, held.lift) * (short ? 0.35 : 1);
    const cx = x + width / 2, cy = y + height / 2, phase = index * 1.7;
    return corners.map((corner, cornerIndex) => {
      const ripple = time * 0.48 + phase + cornerIndex * 1.35;
      let outX = corner.x - cx, outY = corner.y - cy;
      const span = Math.max(1, Math.hypot(outX, outY));
      outX /= span; outY /= span;
      const breath = Math.sin(ripple) * 5 * amp;
      const along = Math.cos(ripple * 0.73 + 0.4) * 2.6 * amp;
      return {
        x: Math.min(rect.x + rect.width + WIND_LEEWAY, Math.max(rect.x - WIND_LEEWAY, corner.x + outX * breath - outY * along)),
        y: Math.min(rect.y + rect.height + WIND_LEEWAY, Math.max(rect.y - WIND_LEEWAY, corner.y + outY * breath + outX * along)),
      };
    });
  }

  function sheetTransform(pose, drift, time, index, still) {
    const held = poseOf(pose);
    const scale = (0.92 + 0.08 * held.lift) * (1 - 0.22 * held.shrink) * (1 - 0.04 * held.angle);
    const tilt = still ? 0 : Math.sin(time * 0.37 + index * 1.7) * 0.35 * held.lift;
    return `translate(${drift.x.toFixed(2)}px, ${drift.y.toFixed(2)}px) rotate(${tilt.toFixed(3)}deg) rotateX(${(-10 * held.angle).toFixed(2)}deg) rotateY(${(72 * held.turn).toFixed(2)}deg) scale(${scale.toFixed(4)})`;
  }

  function attach(doc = global.document) {
    const win = doc.defaultView;
    if (!win) throw new Error("A document window is required");
    const reduced = win.matchMedia("(prefers-reduced-motion: reduce)");
    const states = new Map();
    let raf = 0, last = 0, started = win.performance.now(), destroyed = false;

    function still() {
      if (reduced.matches || doc.hidden) return true;
      const toggle = doc.getElementById("toggle-visual-motion");
      if (toggle?.checked) return true;
      if (doc.documentElement.dataset.motion === "paused") return true;
      const world = doc.querySelector(".spatial-world:not(.spatial-background-only)");
      return world?.dataset.motion === "paused";
    }
    function surfaces() {
      return [...doc.querySelectorAll(".spatial-panel-cube, .spatial-workspace, .spatial-thread-face")];
    }
    function ensureMarks(node) {
      if (node.querySelector(":scope > .sheet-corner")) return;
      AXES.forEach((axis, index) => {
        const button = doc.createElement("button");
        button.type = "button";
        button.className = "sheet-corner";
        button.dataset.axis = axis;
        button.textContent = MARKS[index];
        button.setAttribute("aria-label", LABELS[axis]);
        button.addEventListener("pointerdown", event => event.stopPropagation());
        button.addEventListener("click", event => {
          event.preventDefault();
          event.stopPropagation();
          const state = states.get(node);
          if (!state) return;
          state.emerged = true;
          state.target = { ...state.target, [axis]: state.target[axis] > 0.5 ? 0 : 1 };
        });
        node.appendChild(button);
      });
    }
    function opacityFor(node, lift) {
      const veil = 0.62 + 0.38 * lift;
      if (!node.classList.contains("spatial-workspace")) return veil;
      const open = Number(win.getComputedStyle(node.parentElement || node).getPropertyValue("--aperture-open"));
      return (Number.isFinite(open) ? open : 1) * veil;
    }
    function tick(now) {
      if (destroyed) return;
      const dt = last ? Math.min(0.05, (now - last) / 1000) : 0.016;
      last = now;
      const paused = still();
      const time = (now - started) / 1000;
      const nodes = surfaces();
      const live = new Set(nodes);
      nodes.forEach((node, index) => {
        ensureMarks(node);
        let state = states.get(node);
        if (!state) {
          state = { shown: { ...RECESSED }, target: { ...HELD }, emerged: reduced.matches };
          states.set(node, state);
        }
        if (!state.emerged && time >= EMERGE_AT[index % EMERGE_AT.length]) state.emerged = true;
        const target = state.emerged ? state.target : { ...state.target, lift: 0 };
        state.shown = easePose(state.shown, target, dt, paused && reduced.matches);
        const quiet = paused || state.shown.lift < 0.15;
        const drift = windDrift(time, index, state.shown.lift, quiet);
        node.style.transformOrigin = "50% 50%";
        node.style.transform = sheetTransform(state.shown, drift, time, index, quiet);
        node.style.opacity = opacityFor(node, state.shown.lift).toFixed(3);
        node.querySelectorAll(":scope > .sheet-corner").forEach(button => {
          button.setAttribute("aria-pressed", String(state.target[button.dataset.axis] > 0.5));
        });
      });
      states.forEach((_, node) => { if (!live.has(node)) states.delete(node); });
      raf = win.requestAnimationFrame(tick);
    }
    raf = win.requestAnimationFrame(tick);
    return { destroy() { destroyed = true; win.cancelAnimationFrame(raf); states.forEach((_, node) => { node.style.transform = ""; node.style.opacity = ""; }); states.clear(); } };
  }

  const api = Object.freeze({ attach, easePose, windDrift, sheetCorners, sheetTransform, MARKS, AXES, EMERGE_AT, HELD, RECESSED, WIND_LEEWAY });
  global.BombSheetSurfaces = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
})(typeof window !== "undefined" ? window : globalThis);
