import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const tour = require('./studio-tour.js');

test('welcome exposes the same whitelist the runner will perform', () => {
  const checked = tour.auditTour(tour.WELCOME);
  assert.equal(checked.ok, true);
  assert.deepEqual(checked.steps.map(step => step.action), ['open-view']);
  assert.equal(checked.steps[0].view, 'spatial');
  for (const key of tour.FORBIDDEN) assert.equal(Object.hasOwn(tour.WELCOME.steps[0].action, key), false);
});

test('forbidden commands and unknown views never become actions', () => {
  assert.equal(tour.audit({ type: 'open-view', view: 'spatial', invoke: 'analyze' }).ok, false);
  assert.equal(tour.audit({ type: 'send-message' }).reason, 'unlisted');
  assert.equal(tour.audit({ type: 'open-view', view: 'shell' }).reason, 'view');
  const calls = [];
  const result = tour.perform({ type: 'open-view', view: 'spatial', payload: { approve: true } }, { activateView: view => calls.push(view) });
  assert.equal(result.ok, false);
  assert.deepEqual(calls, []);
});

test('welcome can move back, stop, and complete without another command', () => {
  const start = tour.move(tour.WELCOME, 0, 'back');
  assert.equal(start.index, 0);
  assert.equal(tour.move(tour.WELCOME, 0, 'stop').status, 'skipped');
  assert.equal(tour.move(tour.WELCOME, 0, 'next').status, 'completed');
  const seen = [];
  tour.perform(tour.WELCOME.steps[0].action, { activateView: view => seen.push(view), highlight: id => seen.push(id) });
  assert.deepEqual(seen, ['spatial', 'studio-invitation']);
});
