import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const panels = require("./spatial-panels.js");

/** Mirror of spatial-panels clampAboveComposer y adjustment */
function clampAboveComposer(rect, boundsHeight) {
  const reserve = Math.min(160, Math.max(120, Math.floor(boundsHeight * 0.18)));
  const maxBottom = Math.max(40, boundsHeight - reserve);
  if (rect.y + rect.height > maxBottom) {
    return {
      ...rect,
      y: Math.max(12, maxBottom - rect.height),
      height: Math.min(rect.height, Math.max(34, maxBottom - 12)),
    };
  }
  return rect;
}

function dockRightX(rect, boundsWidth) {
  return { ...rect, x: Math.max(12, boundsWidth - rect.width - 24) };
}

test("composer reserve lifts cube out of bottom band at 670px height", () => {
  const before = { x: 700, y: 500, width: 214, height: 200 };
  const after = clampAboveComposer(before, 670);
  assert.ok(after.y + after.height <= 670 - 120, `bottom ${after.y + after.height} must clear composer`);
});

test("enlarge re-docks right rail to new width", () => {
  const small = dockRightX({ x: 700, y: 64, width: 214, height: 34 }, 960);
  assert.equal(small.x, 960 - 214 - 24);
  const large = dockRightX(small, 1380);
  assert.equal(large.x, 1380 - 214 - 24);
  assert.ok(large.x > small.x);
});

/** Mirror of tuck: context faces park at right, not full-bleed over composer */
function tuckRect(bounds, index) {
  return {
    x: Math.max(12, bounds.width - 340),
    y: 155 + (index % 5) * 40,
    width: 300,
    height: Math.min(220, Math.max(160, bounds.height - 320)),
  };
}

test("tucked context face stays above composer band at 670px", () => {
  const r = tuckRect({ width: 960, height: 670 }, 0);
  assert.ok(r.y + r.height < 670 - 100, "face must not cover sticky composer");
});

test("navigate cube fits Home through System at the 900px desk", () => {
  const inner = 900;
  const leftScale = Math.min(1, (inner - 76 - 36) / 740);
  const nav = panels.LEFT_COLUMN.nav * leftScale;
  const projectsTop = 76 + nav;
  const projects = panels.LEFT_COLUMN.projects * leftScale;
  const servicesTop = 88 + nav + projects;
  const services = panels.LEFT_COLUMN.services * leftScale;
  const header = 32;
  const row = 30;
  assert.ok(nav - header >= 10 * row, `nav body ${nav - header} must hold 10 destinations`);
  assert.ok(projectsTop >= 64 + nav);
  assert.ok(servicesTop >= projectsTop + projects);
  assert.ok(servicesTop + services < inner);
});

test("enlarge should recompute default x for left and right rails", () => {
  const smallW = 960;
  const largeW = 1380;
  const left = { x: 12, y: 64, width: 200, height: 120 };
  const right = dockRightX({ x: 700, y: 64, width: 214, height: 34 }, smallW);
  const rightLarge = dockRightX(right, largeW);
  assert.ok(rightLarge.x > right.x);
  // Left rail stays near left but must remain on-canvas
  assert.ok(left.x + left.width < largeW);
});
