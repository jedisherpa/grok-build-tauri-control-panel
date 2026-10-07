import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const play = require("./studio-play.js");

test("unknown and empty verbs fall back to spin, and the five verbs stay distinct", () => {
  assert.equal(play.normalizeVerb(""), "spin");
  assert.equal(play.normalizeVerb(null), "spin");
  assert.equal(play.normalizeVerb("nope"), "spin");
  assert.equal(play.normalizeVerb(" SPIN "), "spin");
  for (const verb of ["spin", "stretch", "drop", "paint", "hum"]) {
    assert.equal(play.normalizeVerb(verb), verb);
    assert.equal(play.playResult(verb).word.split(/\s+/).length, 1);
  }
});

test("a loop lasts ten seconds and does not keep the previous loop", () => {
  const first = play.beginLoop("drop", 1000);
  first.verb = "hum";
  first.carried = true;
  const second = play.beginLoop("drop", 2500);
  assert.equal(first.endsAt - 1000, 10000);
  assert.equal(second.verb, "drop");
  assert.equal(second.kind, "hit");
  assert.equal(second.word, "Drop");
  assert.equal(second.startedAt, 2500);
  assert.equal(second.endsAt, 12500);
  assert.equal(second.carried, undefined);
  assert.notEqual(first, second);
});

test("a miss is a different result with no word and the same ten seconds", () => {
  const missed = play.beginMiss(40);
  assert.equal(missed.kind, "miss");
  assert.equal(missed.word, "");
  assert.equal(missed.endsAt - missed.startedAt, 10000);
  assert.equal(play.toneHz("miss", "spin"), 110);
  assert.notEqual(play.toneHz("hit", "spin"), play.toneHz("miss", "spin"));
});

test("tones are numbers for every verb and silence only flips the mute flag", () => {
  assert.equal(play.toneHz("hit", "spin"), 220);
  assert.equal(play.toneHz("hit", "stretch"), 277);
  assert.equal(play.toneHz("hit", "drop"), 165);
  assert.equal(play.toneHz("hit", "paint"), 330);
  assert.equal(play.toneHz("hit", "hum"), 196);
  assert.equal(play.toneHz("hit", "nope"), 220);
  assert.equal(play.silenceNext(false), true);
  assert.equal(play.silenceNext(true), false);
});

test("a skin does not change the verb, the word, or the tone", () => {
  assert.equal(play.normalizeSkin("  the critic\n"), "the critic");
  assert.equal(play.normalizeSkin(""), "");
  assert.equal(play.samePlay("the critic", "", "spin"), true);
  assert.equal(play.samePlay("the critic", "unnamed", "hum"), true);
  const named = play.beginLoop("paint", 0);
  const unnamed = play.beginLoop("paint", 0);
  assert.equal(named.verb, unnamed.verb);
  assert.equal(named.word, unnamed.word);
  assert.equal(play.toneHz(named.kind, named.verb), play.toneHz(unnamed.kind, unnamed.verb));
});

test("play cannot name a consequence, and speech is only an unsilenced hit", () => {
  const result = play.playResult("stretch");
  assert.deepEqual(Object.keys(result).sort(), ["endsInMs", "kind", "toneHz", "verb", "word"]);
  for (const key of ["send", "land", "post", "buy", "message", "consent", "dose"]) {
    assert.equal(Object.hasOwn(result, key), false);
  }
  assert.equal(result.endsInMs, 10000);
  assert.equal(play.shouldSpeak("hit", false), true);
  assert.equal(play.shouldSpeak("hit", true), false);
  assert.equal(play.shouldSpeak("miss", false), false);
});

test("a tap, a dwell, and a mash play the verb, and a corner does not", () => {
  const press = play.gestureStep(false, "down", "hit");
  assert.equal(press.action, null);
  assert.equal(press.armed, true);
  const tap = play.gestureStep(press.armed, "up", "hit");
  assert.equal(tap.action, "hit");
  assert.equal(tap.armed, false);
  const held = play.gestureStep(true, "dwell", "hit");
  assert.equal(held.action, "hit");
  assert.equal(play.gestureStep(held.armed, "up", "hit").action, null);
  const mash = play.gestureStep(true, "down", "hit");
  assert.equal(mash.action, "hit");
  assert.equal(play.gestureStep(false, "down", "miss").action, "miss");
  assert.equal(play.gestureStep(true, "down", "miss").armed, false);
});

test("a click plays unless a pointer gesture just played", () => {
  assert.equal(play.acceptClick(0), false);
  assert.equal(play.acceptClick(499), false);
  assert.equal(play.acceptClick(500), true);
  assert.equal(play.acceptClick(-1), true);
  assert.equal(play.acceptClick(Number.NaN), true);
});

test("the center is a hit and the corners are misses", () => {
  assert.equal(play.classifyPoint(200, 200, 400, 400), "hit");
  assert.equal(play.classifyPoint(8, 8, 400, 400), "miss");
  assert.equal(play.classifyPoint(390, 12, 400, 400), "miss");
  assert.equal(play.classifyPoint(12, 380, 400, 400), "miss");
  assert.equal(play.classifyPoint(390, 390, 400, 400), "miss");
  assert.equal(play.DWELL_MS, 600);
});

test("opening the desk does not cover the thread with the play plate", () => {
  const classes = new Set();
  const made = [];
  function element() {
    return {
      id: "",
      hidden: false,
      textContent: "",
      dataset: {},
      children: [],
      attributes: {},
      classList: { add() {}, remove() {} },
      setAttribute(name, value) { this.attributes[name] = value; },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      addEventListener() {},
      querySelector() { return null; },
    };
  }
  const doc = {
    documentElement: {
      classList: {
        add(name) { classes.add(name); },
        remove(name) { classes.delete(name); },
      },
    },
    body: element(),
    getElementById(id) { return made.find(node => node.id === id) || null; },
    createElement() {
      const node = element();
      made.push(node);
      return node;
    },
    querySelectorAll() { return []; },
  };
  const attached = play.attach({
    document: doc,
    storage: { getItem() { return null; }, setItem() { return true; } },
  });
  const surface = doc.getElementById("studio-play");
  const word = surface.children.find(node => node.id === "studio-play-word");
  assert.ok(attached);
  assert.equal(surface.hidden, true);
  assert.equal(classes.has("is-studio-play"), false);
  assert.equal(word.textContent, "");
  assert.equal(word.hidden, true);
});
