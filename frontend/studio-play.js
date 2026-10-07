/* Opening play surface. One stored verb, ten-second loops, no consequences. */
(function (global) {
  "use strict";

  const VERBS = Object.freeze(["spin", "stretch", "drop", "paint", "hum"]);
  const WORDS = Object.freeze({ spin: "Spin", stretch: "Stretch", drop: "Drop", paint: "Paint", hum: "Hum" });
  const TONES = Object.freeze({ spin: 220, stretch: 277, drop: 165, paint: 330, hum: 196, miss: 110 });
  const LOOP_MS = 10000;
  const DWELL_MS = 600;
  const VERB_KEY = "c3:session-verb:v1";
  const SKIN_KEY = "c3:session-skin:v1";

  function normalizeVerb(value) {
    const text = typeof value === "string" ? value.trim().toLowerCase() : "";
    return VERBS.includes(text) ? text : "spin";
  }

  function normalizeSkin(value) {
    if (typeof value !== "string") return "";
    const cleaned = value.replace(/[\u0000-\u001f]/g, " ").replace(/\s+/g, " ").trim();
    return cleaned.slice(0, 32);
  }

  function toneHz(kind, verb) {
    if (kind === "miss") return TONES.miss;
    return TONES[normalizeVerb(verb)];
  }

  function beginLoop(verb, nowMs) {
    const chosen = normalizeVerb(verb);
    const startedAt = Number(nowMs) || 0;
    return { verb: chosen, kind: "hit", word: WORDS[chosen], startedAt, endsAt: startedAt + LOOP_MS };
  }

  function beginMiss(nowMs) {
    const startedAt = Number(nowMs) || 0;
    return { verb: "", kind: "miss", word: "", startedAt, endsAt: startedAt + LOOP_MS };
  }

  function silenceNext(muted) {
    return muted !== true;
  }

  function samePlay(skinA, skinB, verb) {
    normalizeSkin(skinA);
    normalizeSkin(skinB);
    const named = playResult(verb);
    const unnamed = playResult(verb);
    return named.verb === unnamed.verb && named.word === unnamed.word && named.toneHz === unnamed.toneHz && named.kind === unnamed.kind;
  }

  function gestureStep(armed, type, region) {
    const holding = armed === true;
    if (type === "down" && region === "miss") return { armed: false, action: "miss" };
    if (type === "down" && region === "hit") return { armed: true, action: holding ? "hit" : null };
    if ((type === "up" || type === "dwell") && holding) return { armed: false, action: "hit" };
    return { armed: holding, action: null };
  }

  function acceptClick(sincePointerMs) {
    const elapsed = Number(sincePointerMs);
    if (!Number.isFinite(elapsed)) return true;
    return elapsed < 0 || elapsed >= 500;
  }

  function playResult(verb) {
    const loop = beginLoop(verb, 0);
    return { verb: loop.verb, kind: loop.kind, word: loop.word, toneHz: toneHz(loop.kind, loop.verb), endsInMs: LOOP_MS };
  }

  function shouldSpeak(kind, muted) {
    return kind === "hit" && muted !== true;
  }

  function classifyPoint(x, y, width, height) {
    const w = Number(width) || 0;
    const h = Number(height) || 0;
    if (w <= 0 || h <= 0) return "miss";
    const insetX = w * 0.18;
    const insetY = h * 0.18;
    if (x < insetX || y < insetY || x > w - insetX || y > h - insetY) return "miss";
    return "hit";
  }

  function readStorage(storage, key) {
    try { return storage.getItem(key); } catch { return null; }
  }

  function writeStorage(storage, key, value) {
    try { storage.setItem(key, value); return true; } catch { return false; }
  }

  function attach(options = {}) {
    const doc = options.document || global.document;
    if (!doc?.documentElement || doc.getElementById("studio-play")) return null;
    const storage = options.storage || doc.defaultView?.localStorage;
    const now = options.now || (() => Date.now());
    let muted = false;
    let armed = false;
    let playedAt = 0;
    let dwell = 0;
    let settle = 0;
    let audio = null;

    const surface = doc.createElement("section");
    surface.id = "studio-play";
    surface.setAttribute("aria-label", "Play");
    const word = doc.createElement("p");
    word.id = "studio-play-word";
    word.hidden = true;
    word.textContent = "";
    const silence = doc.createElement("button");
    silence.id = "studio-silence";
    silence.type = "button";
    silence.hidden = true;
    silence.setAttribute("aria-label", "Silence");
    silence.setAttribute("aria-pressed", "false");
    surface.hidden = true;
    surface.append(word, silence);
    (options.parent || doc.body).appendChild(surface);

    const invitation = doc.getElementById("studio-invitation");
    if (invitation && !invitation.querySelector(".studio-verbs")) {
      const row = doc.createElement("div");
      row.className = "studio-verbs";
      row.setAttribute("role", "group");
      row.setAttribute("aria-label", "Next session verb");
      VERBS.forEach(verb => {
        const button = doc.createElement("button");
        button.type = "button";
        button.textContent = WORDS[verb];
        button.addEventListener("click", () => {
          writeStorage(storage, VERB_KEY, verb);
          const status = invitation.querySelector(".studio-status");
          if (status) status.textContent = `Next verb: ${WORDS[verb]}. Nothing else starts.`;
        });
        row.appendChild(button);
      });
      invitation.appendChild(row);
    }

    function storedVerb() {
      return normalizeVerb(readStorage(storage, VERB_KEY));
    }

    function paintWord() {
      word.hidden = true;
      word.textContent = "";
      const skin = normalizeSkin(readStorage(storage, SKIN_KEY));
      if (skin) surface.dataset.skin = skin;
      else delete surface.dataset.skin;
    }

    function enter() {
      paintWord();
      doc.documentElement.classList.add("is-studio-play");
      surface.hidden = false;
      const composer = doc.getElementById("composer");
      if (composer) composer.hidden = true;
      if (invitation) invitation.hidden = true;
    }

    function leave() {
      doc.documentElement.classList.remove("is-studio-play");
      surface.hidden = true;
      const composer = doc.getElementById("composer");
      if (composer) composer.hidden = false;
    }

    function playTone(hz) {
      const Ctx = global.AudioContext || global.webkitAudioContext;
      if (muted || !Ctx) return;
      if (!audio) audio = new Ctx();
      const osc = audio.createOscillator();
      const gain = audio.createGain();
      const start = audio.currentTime;
      osc.type = "sine";
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(0.08, start + 0.04);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + 1.6);
      osc.connect(gain).connect(audio.destination);
      osc.start(start);
      osc.stop(start + 1.7);
    }

    function speak(text) {
      if (!text || muted || !global.speechSynthesis || !global.SpeechSynthesisUtterance) return;
      global.speechSynthesis.cancel();
      global.speechSynthesis.speak(new global.SpeechSynthesisUtterance(text));
    }

    function show(kind) {
      const verb = storedVerb();
      const action = kind === "miss" ? beginMiss(now()) : beginLoop(verb, now());
      word.classList.remove("is-hit", "is-miss");
      void word.offsetWidth;
      word.classList.add(kind === "miss" ? "is-miss" : "is-hit");
      clearTimeout(settle);
      settle = setTimeout(() => word.classList.remove("is-hit", "is-miss"), LOOP_MS);
      playTone(toneHz(action.kind, verb));
      if (shouldSpeak(action.kind, muted)) speak(action.word);
      return action;
    }

    function applyGesture(type, region) {
      const step = gestureStep(armed, type, region);
      armed = step.armed;
      if (type === "down") {
        clearTimeout(dwell);
        if (step.armed) dwell = setTimeout(() => applyGesture("dwell", "hit"), DWELL_MS);
      }
      if (type === "up" || type === "dwell" || region === "miss") clearTimeout(dwell);
      if (step.action) {
        playedAt = now();
        surface.dataset.action = step.action;
        show(step.action);
      }
    }

    surface.addEventListener("pointerdown", event => {
      if (event.target?.closest?.("#studio-silence")) return;
      if (surface.setPointerCapture && event.pointerId != null) {
        try { surface.setPointerCapture(event.pointerId); } catch { /* pointer already released */ }
      }
      const bounds = surface.getBoundingClientRect();
      const region = classifyPoint(event.clientX - bounds.left, event.clientY - bounds.top, bounds.width, bounds.height);
      applyGesture("down", region);
    });

    surface.addEventListener("pointerup", () => applyGesture("up", "hit"));

    surface.addEventListener("click", event => {
      if (event.target?.closest?.("#studio-silence")) return;
      if (!acceptClick(now() - playedAt)) return;
      const bounds = surface.getBoundingClientRect();
      const region = classifyPoint(event.clientX - bounds.left, event.clientY - bounds.top, bounds.width, bounds.height);
      applyGesture("down", region);
      if (region !== "miss") applyGesture("up", "hit");
    });

    silence.addEventListener("click", event => {
      event.stopPropagation();
      muted = silenceNext(muted);
      silence.setAttribute("aria-pressed", muted ? "true" : "false");
      if (muted && global.speechSynthesis) global.speechSynthesis.cancel();
    });

    silence.addEventListener("pointerdown", event => event.stopPropagation());

    doc.querySelectorAll(".nav-item").forEach(button => button.addEventListener("click", leave));
    return { element: surface, enter, leave, show };
  }

  const api = Object.freeze({
    VERBS, WORDS, LOOP_MS, DWELL_MS, VERB_KEY, SKIN_KEY,
    normalizeVerb, normalizeSkin, toneHz, beginLoop, beginMiss, silenceNext, samePlay,
    playResult, shouldSpeak, classifyPoint, gestureStep, acceptClick, attach,
  });
  global.BombStudioPlay = api;
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  // The page CSP allows file scripts and blocks inline scripts, so boot here.
  const page = global.document;
  if (page?.documentElement) attach();
})(typeof window !== "undefined" ? window : globalThis);
