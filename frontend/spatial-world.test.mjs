import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const world = require('./spatial-world.js');
const bytes = readFileSync(new URL('./assets/e8-scaffold.json', import.meta.url));
const scaffold = JSON.parse(bytes);
const clone = () => structuredClone(scaffold);
const norm = v => v.reduce((sum, x) => sum + x * x, 0);
const now = 1800000000000;
const row = change => ({ id: 'real-thread', title: 'Actual work', live: true, phase: 'tools', lastSignalAt: now - 800, toolsActive: 2, pendingApprovals: 0, ...change });

test('pinned source scaffold keeps byte identity, canonical IDs, 240 roots and 6720 original 8D edges', () => {
  assert.equal(createHash('sha256').update(bytes).digest('hex'), world.constants.ASSET_SHA);
  assert.equal(world.validateScaffold(scaffold), scaffold);
  const degrees = new Array(240).fill(0);
  for (const [a, b] of scaffold.edges) { degrees[a]++; degrees[b]++; }
  assert.ok(degrees.every(n => n === 56));
});

test('a different root order cannot silently reuse source root IDs', () => {
  const data = clone();
  [data.roots[0].position8, data.roots[1].position8] = [data.roots[1].position8, data.roots[0].position8];
  assert.throws(() => world.validateScaffold(data), /source lexicographic order/);
});

test('duplicate and geometrically false edges fail validation', () => {
  const duplicate = clone(); duplicate.edges[0] = duplicate.edges[1];
  assert.throws(() => world.validateScaffold(duplicate), /8D nearest-neighbor/);
  const falseEdge = clone(); falseEdge.edges[0] = [0, 239];
  assert.throws(() => world.validateScaffold(falseEdge), /8D nearest-neighbor/);
});

test('a changed or nonorthonormal projection fails without changing geometry silently', () => {
  const changed = clone(); changed.sourcePlaneSha256 = 'different';
  assert.throws(() => world.validateScaffold(changed), /convention differs/);
  const nonorthogonal = clone(); nonorthogonal.projectionBasisQ[0][0] += .001;
  assert.throws(() => world.validateScaffold(nonorthogonal), /not orthonormal/);
});

test('rotation preserves every root norm and is reversible', () => {
  for (const { position8: v } of scaffold.roots) {
    const rotated = world.rotate8(v, [[0, 2, 1.17], [1, 3, -.52], [4, 7, .31]]);
    assert.ok(Math.abs(norm(rotated) - norm(v)) < 1e-13);
    const restored = world.rotate8(rotated, [[4, 7, -.31], [1, 3, .52], [0, 2, -1.17]]);
    assert.ok(restored.every((x, i) => Math.abs(x - v[i]) < 1e-13));
  }
});

test('four source planes retain the full norm before screen perspective or aperture displacement', () => {
  for (const { position8: v } of scaffold.roots) {
    const projectedNorm = [0, 1, 2, 3].reduce((sum, plane) => {
      const p = world.projectVector(v, scaffold.projectionBasisQ, plane, .7, -.2);
      return sum + p.x * p.x + p.y * p.y;
    }, 0);
    assert.ok(Math.abs(projectedNorm - 2) < 1e-12);
  }
});

test('saved histories never impersonate active sessions even with fresh old tool labels', () => {
  for (const override of [{ live: false }, { savedOnly: true }]) {
    const [s] = world.normalizeSnapshots([row(override)], now);
    assert.equal(s.label, 'Saved history'); assert.equal(s.running, false); assert.equal(s.animated, false);
  }
});

test('unknown, missing, future and stale telemetry cannot animate', () => {
  const [unknown, missing, future, stale] = world.normalizeSnapshots([
    row({ id: 'unknown', live: null, phase: 'tools' }),
    row({ id: 'missing', lastSignalAt: null }),
    row({ id: 'future', lastSignalAt: now + 6000 }),
    row({ id: 'stale', lastSignalAt: now - 26000 })
  ], now);
  assert.equal(unknown.label, 'Live status unknown');
  assert.equal(missing.label, 'Active reported · signal time unknown');
  assert.equal(future.ageMs, null); assert.match(stale.label, /No recent signal/);
  assert.ok([unknown, missing, future, stale].every(s => !s.animated));
});

test('only fresh observed activity animates; waits and turn endings stay literal', () => {
  const [active, waiting, ended] = world.normalizeSnapshots([
    row({ id: 'active' }), row({ id: 'waiting', phase: 'wait', pendingApprovals: 3 }), row({ id: 'ended', phase: 'done' })
  ], now);
  assert.equal(active.label, '2 tools running'); assert.equal(active.animated, true);
  assert.equal(waiting.label, '3 requests for approval'); assert.equal(waiting.animated, false);
  assert.equal(ended.label, 'Turn ended'); assert.equal(ended.animated, false);
});

test('thread snapshots deduplicate IDs and preserve titles as literal text', () => {
  const out = world.normalizeSnapshots([row({ title: '<button>Private title</button>' }), row({ title: 'Duplicate' }), { id: 4 }], now);
  assert.equal(out.length, 1); assert.equal(out[0].title, '<button>Private title</button>');
  assert.equal(world.stableIndex(out[0].id), world.stableIndex(out[0].id));
  assert.ok(world.stableIndex('another-id') >= 0 && world.stableIndex('another-id') < 240);
});

test('Joe peripheral route is bounded, continuous and traverses the actual scene', () => {
  for (const [width, height] of [[1280, 720], [850, 650], [700, 480]]) {
    let previous = null, minX = Infinity, maxX = -Infinity;
    for (let t = 0; t <= 60; t += .05) {
      const p = world.peripheralRoute(t, width, height);
      assert.ok(p.x >= width * .07 - 1e-9 && p.x <= width * .93 + 1e-9);
      assert.ok(p.y >= Math.min(height - 24, Math.max(210, height * .34)) - 1e-9 && p.y <= height - 18 + 1e-9);
      if (previous) assert.ok(Math.hypot(p.x - previous.x, p.y - previous.y) <= 64 * .05 + 1e-7);
      previous = p; minX = Math.min(minX, p.x); maxX = Math.max(maxX, p.x);
    }
    assert.ok(maxX - minX > width * .70);
  }
});

test('exclusion masks clip partly offscreen surfaces and reject empty or malformed bounds', () => {
  assert.deepEqual(world.normalizeExclusions([
    {x:-20,y:40,width:100,height:80}, {left:80,top:70,right:160,bottom:150},
    {x:400,y:400,width:20,height:20}, {x:2,y:2,width:-1,height:20}, {x:NaN,y:0,width:2,height:2}
  ],120,100), [{x:0,y:40,width:80,height:60},{x:80,y:70,width:40,height:30}]);
});

test('resize and movement keep native working surfaces inside scene and preserve usable minimums', () => {
  for (const bounds of [{width:1000,height:720},{width:700,height:480}]) {
    const oversized = world.boundWorkspaceRect({x:-100,y:-100,width:3000,height:3000},bounds);
    assert.equal(oversized.x,16); assert.equal(oversized.y,125);
    assert.equal(oversized.width,bounds.width-32); assert.equal(oversized.height,bounds.height-185);
    const small = world.boundWorkspaceRect({x:9999,y:9999,width:10,height:10},bounds);
    assert.ok(small.width>=320 && small.height>=240);
    assert.ok(small.x+small.width<=bounds.width-16 && small.y+small.height<=bounds.height-60);
  }
});

test('each information-face corner links to one lattice point outside the faces', () => {
  const points = [{ x: 0, y: 0 }, { x: 50, y: 10 }, { x: 10, y: 90 }, { x: 200, y: 200 }, { x: 40, y: 40 }];
  const faces = [{ x: 20, y: 20, width: 60, height: 40 }];
  const links = world.cornerLinks(points, faces);
  assert.equal(links.length, 4);
  const corners = [[20, 20], [80, 20], [80, 60], [20, 60]];
  links.forEach((link, i) => {
    assert.equal(link.x2, corners[i][0]);
    assert.equal(link.y2, corners[i][1]);
    assert.notEqual(link.x1, 40);
    assert.ok(points.some(p => p.x === link.x1 && p.y === link.y1));
  });
});

test('a corner uses a lattice point whose segment misses the other faces', () => {
  const points = [{ x: 200, y: 10 }, { x: 10, y: -30 }];
  const faces = [{ x: 0, y: 0, width: 40, height: 40 }, { x: 50, y: 0, width: 40, height: 40 }];
  const topRight = world.cornerLinks(points, faces).find(link => link.x2 === 40 && link.y2 === 0);
  assert.equal(topRight.x1, 10);
  assert.equal(topRight.y1, -30);
});

test('a panel sheet hangs from its four corners', () => {
  const corners = world.panelCorners({ x: 10, y: 20, width: 100, height: 80 });
  assert.deepEqual(corners, [
    { x: 10, y: 20 },
    { x: 110, y: 20 },
    { x: 110, y: 100 },
    { x: 10, y: 100 },
  ]);
});

test('a corner joint stays when another face blocks the middle of the segment', () => {
  const pieces = world.outsideSegments(0, 0, 100, 0, [{ x: 40, y: -10, width: 20, height: 20 }]);
  assert.ok(pieces.some(seg => (seg.x1 === 0 && seg.y1 === 0) || (seg.x2 === 0 && seg.y2 === 0)));
  assert.ok(pieces.some(seg => seg.x1 >= 60 || seg.x2 >= 60));
  pieces.forEach(seg => {
    const midX = (seg.x1 + seg.x2) / 2;
    assert.ok(midX <= 40 || midX >= 60);
  });
});

test('session cleanup retains literal turn-ending evidence without implying task acceptance', () => {
  const [closed,unknown,saved] = world.normalizeSnapshots([
    row({id:'closed',phase:'idle',sessionClosed:true,turnEndedAt:now-1000}),
    row({id:'closed-unknown',phase:'idle',sessionClosed:true}),
    row({id:'saved-closed',live:false,sessionClosed:true,turnEndedAt:now-1000})
  ],now);
  assert.equal(closed.qualifier,'Session closed · turn ended'); assert.equal(closed.animated,false);
  assert.equal(unknown.qualifier,'Session closed');
  assert.equal(saved.label,'Saved history'); assert.equal(saved.qualifier,'');
});
