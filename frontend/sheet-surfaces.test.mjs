import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const sheets = require("./sheet-surfaces.js");

test("corner marks are turn, tip, lift, and shrink", () => {
  assert.deepEqual(sheets.MARKS, ["θ", "∠", "⊥", "λ"]);
  assert.deepEqual(sheets.AXES, ["turn", "angle", "lift", "shrink"]);
});

test("reduced motion snaps a recessed sheet to held", () => {
  assert.deepEqual(sheets.easePose(sheets.RECESSED, sheets.HELD, 0.2, true), { turn: 0, angle: 0, shrink: 0, lift: 1 });
});

test("a sheet eases out of the lattice instead of jumping", () => {
  const next = sheets.easePose(sheets.RECESSED, sheets.HELD, 0.2, false);
  assert.ok(next.lift > 0.3 && next.lift < 0.7);
  assert.equal(next.turn, 0);
});

test("pause holds the corners still", () => {
  const rect = { x: 40, y: 80, width: 220, height: 160 };
  const still = sheets.sheetCorners(rect, sheets.HELD, 1.2, 1, true);
  const again = sheets.sheetCorners(rect, sheets.HELD, 4.8, 1, true);
  assert.deepEqual(still, again);
  assert.deepEqual(sheets.windDrift(2, 1, 1, true), { x: 0, y: 0 });
});

test("wind moves a held sheet and stays inside the leeway", () => {
  const rect = { x: 40, y: 80, width: 220, height: 160 };
  const live = sheets.sheetCorners(rect, sheets.HELD, 1.2, 2, false);
  const still = sheets.sheetCorners(rect, sheets.HELD, 1.2, 2, true);
  assert.ok(live.some((corner, index) => corner.x !== still[index].x || corner.y !== still[index].y));
  live.forEach(corner => {
    assert.ok(corner.x >= rect.x - sheets.WIND_LEEWAY - 0.01);
    assert.ok(corner.x <= rect.x + rect.width + sheets.WIND_LEEWAY + 0.01);
    assert.ok(corner.y >= rect.y - sheets.WIND_LEEWAY - 0.01);
    assert.ok(corner.y <= rect.y + rect.height + sheets.WIND_LEEWAY + 0.01);
  });
});

test("a short paused header does not flutter", () => {
  const rect = { x: 0, y: 0, width: 180, height: 34 };
  const a = sheets.sheetCorners(rect, sheets.HELD, 0.4, 0, true);
  const b = sheets.sheetCorners(rect, sheets.HELD, 3.2, 0, true);
  assert.deepEqual(a, b);
  assert.equal(a[0].y, a[1].y);
});
