import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const root = new URL("./", import.meta.url);
const read = (name) => readFileSync(new URL(name, root), "utf8");

function parseGlb(name) {
  const buf = readFileSync(new URL(name, root));
  assert.equal(buf.subarray(0, 4).toString("utf8"), "glTF");
  const jsonLen = buf.readUInt32LE(12);
  return JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8"));
}

const SOLIDS = ["tetrahedron", "cube", "octahedron", "dodecahedron", "icosahedron"];

test("each platonic solid is one centered mesh", () => {
  for (const name of SOLIDS) {
    const doc = parseGlb(`assets/platonic-${name}.glb`);
    assert.equal(doc.nodes.length, 1, name);
    assert.equal(doc.meshes.length, 1, name);
    assert.equal(doc.scenes[0].nodes.length, 1, name);
    assert.equal(doc.nodes[0].translation, undefined, name);
    const position = doc.accessors[doc.meshes[0].primitives[0].attributes.POSITION];
    const span = Math.max(...position.min.map(Math.abs), ...position.max.map(Math.abs));
    assert.ok(span < 2, `${name} span ${span}`);
  }
});

test("the desk no longer ships the pixel bomb or the combined scene", () => {
  const html = read("index.html");
  const app = read("app.js");
  const css = read("bombs.css");
  const bundled = html + app + css;
  for (const banned of ["logo.png", "platonic-solids.glb", "wick-on", "fuse-spark", "fuse-twinkle"]) {
    assert.equal(bundled.includes(banned), false, banned);
  }
  for (const name of SOLIDS) {
    assert.match(html, new RegExp(`platonic-${name}\\.glb`));
    assert.match(app, new RegExp(`platonic-${name}\\.glb`));
  }
});
