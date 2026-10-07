import test, { mock } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const motion = require('./studio-motion.js');
const world = require('./spatial-world.js');
const active = id => ({ id, animated: true, live: true, phase: 'tools', pendingApprovals: 0 });

test('idle work keeps a 6 degree primary turn and no supplemental axes', () => {
  const target = motion.activityTarget([], null);
  assert.equal(target.primaryDegPerSec, 6);
  assert.equal(target.secondaryAmpDeg, 0);
  assert.equal(target.tertiaryAmpDeg, 0);
  assert.equal(target.a, 0);
});

test('one fresh selected session reaches the activity floor without counting stale or saved rows', () => {
  const target = motion.activityTarget([
    active('live'),
    { id: 'stale', animated: false, live: true, phase: 'tools' },
    { id: 'saved', animated: true, savedOnly: true, live: false, phase: 'idle' },
    { id: 'unknown', animated: false, live: null, phase: 'unknown' },
  ], 'live');
  assert.equal(target.n, 1);
  assert.equal(target.a, 0.35);
  assert.equal(target.primaryDegPerSec, 6 + 3 * 0.35);
  assert.equal(target.tertiaryAmpDeg, 0);
  const unknown = motion.activityTarget([{ id: 'unknown', animated: false, live: null, phase: 'unknown' }], 'unknown');
  assert.equal(unknown.a, 0);
});

test('four or more fresh sessions cap activity and a pending approval returns to the quiet turn', () => {
  const busy = motion.activityTarget([1, 2, 3, 4, 5, 6].map(n => active(`s${n}`)), 's1');
  assert.equal(busy.n, 4);
  assert.equal(busy.a, 1);
  assert.equal(busy.primaryDegPerSec, 9);
  assert.equal(busy.secondaryAmpDeg, 14);
  assert.equal(busy.tertiaryAmpDeg, 5);
  const approval = motion.activityTarget([{ ...active('s1'), pendingApprovals: 1 }, active('s2')], 's1');
  assert.equal(approval.pendingApproval, true);
  assert.equal(approval.primaryDegPerSec, 6);
  assert.equal(approval.secondaryAmpDeg, 0);
  assert.equal(approval.tertiaryAmpDeg, 0);
});

test('token volume does not change the activity target', () => {
  const left = motion.activityTarget([active('s1')], 's1');
  const right = motion.activityTarget([{ ...active('s1'), tokenCount: 9000, text: 'x'.repeat(5000) }], 's1');
  assert.deepEqual(left, right);
});

test('fake clock advances one idle second across clamped steps and ignores a hidden gap', () => {
  let pose = { primaryDeg: 0, secondaryAmp: 10, tertiaryAmp: 4, secondaryPhase: 0, tertiaryPhase: 0 };
  const target = motion.activityTarget([], null);
  for (let i = 0; i < 20; i++) pose = motion.step(pose, 50, target, {});
  assert.ok(Math.abs(pose.primaryDeg - 6) < 1e-9);
  const jumped = motion.step(pose, 5000, target, {});
  assert.ok(Math.abs(jumped.primaryDeg - (pose.primaryDeg + 0.3)) < 1e-9);
  const hidden = motion.step(pose, 5000, { primaryDegPerSec: 9, secondaryAmpDeg: 14, tertiaryAmpDeg: 5 }, { hidden: true });
  assert.equal(hidden.primaryDeg, pose.primaryDeg);
  const paused = motion.step(pose, 50, target, { paused: true });
  assert.equal(paused.primaryDeg, pose.primaryDeg);
  const reduced = motion.step(pose, 50, target, { reduced: true });
  assert.deepEqual(reduced, { primaryDeg: 0, secondaryAmp: 0, tertiaryAmp: 0, secondaryPhase: 0, tertiaryPhase: 0 });
});

test('typing keeps the idle primary turn and eases supplemental axes down', () => {
  let pose = { primaryDeg: 0, secondaryAmp: 8, tertiaryAmp: 3, secondaryPhase: 1, tertiaryPhase: 1 };
  const busy = { primaryDegPerSec: 9, secondaryAmpDeg: 14, tertiaryAmpDeg: 5 };
  pose = motion.step(pose, 50, busy, { typing: true });
  assert.ok(Math.abs(pose.primaryDeg - 0.3) < 1e-9);
  assert.ok(pose.secondaryAmp < 8);
  assert.ok(pose.tertiaryAmp < 3);
  assert.equal(pose.secondaryPhase, 1);
});

test('display orientation is post-projection and a full turn returns to the projected point', () => {
  const vector = [1, 0, 0, 0, 0, 0, 0, 0];
  const basis = world.validateScaffold(JSON.parse(require('node:fs').readFileSync(new URL('./assets/e8-scaffold.json', import.meta.url)))).projectionBasisQ;
  const projected = world.projectVector(vector, basis, 0, 0.4, -0.2);
  const manual = world.projectVector(vector, basis, 0, 0, 0);
  assert.notDeepEqual(projected, manual);
  const identity = motion.orientPoint(projected, { primaryDeg: 0, secondaryAmp: 0, tertiaryAmp: 0, secondaryPhase: 0, tertiaryPhase: 0 });
  assert.ok(Math.abs(identity.x - projected.x) < 1e-12);
  assert.ok(Math.abs(identity.y - projected.y) < 1e-12);
  const turned = motion.orientPoint(projected, { primaryDeg: 360, secondaryAmp: 0, tertiaryAmp: 0, secondaryPhase: 0, tertiaryPhase: 0 });
  assert.ok(Math.abs(turned.x - projected.x) < 1e-9);
  assert.ok(Math.abs(turned.y - projected.y) < 1e-9);
  const points = [projected, { ...projected, x: projected.x + 3, y: projected.y + 4 }].map(point => motion.orientPoint(point, { primaryDeg: 30, secondaryAmp: 0, tertiaryAmp: 0, secondaryPhase: 0, tertiaryPhase: 0 }));
  assert.equal(motion.nearestIndex(points, points[1].x, points[1].y), 1);
});

test('clock drops time spent hidden and clamps a large visible gap', () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  let now = 1000, hidden = false;
  const seen = [];
  const clock = motion.createClock({ now: () => now, hidden: () => hidden, running: () => !hidden, onTick: dt => seen.push(dt), intervalMs: 33 });
  clock.start();
  mock.timers.tick(33);
  now = 1033;
  mock.timers.tick(33);
  now = 8000;
  hidden = true;
  mock.timers.tick(33);
  hidden = false;
  clock.resume();
  now = 12000;
  mock.timers.tick(33);
  now = 16000;
  mock.timers.tick(33);
  assert.deepEqual(seen, [0, 33, 0, 50]);
  clock.stop();
  mock.timers.reset();
});
