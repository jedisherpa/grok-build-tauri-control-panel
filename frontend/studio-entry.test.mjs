import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const entry = require('./studio-entry.js');
const rect = { x: 20, y: 140, width: 400, height: 300 };
const record = JSON.stringify({ version: 1, projectId: '/work/demo', threadId: 'thread-1', primary: rect, secondary: [{ id: 'thread-2', ...rect }] });

test('first visit, legacy upgrade, and returning preference stay distinct', () => {
  assert.equal(entry.entryMode(null, []).mode, 'first');
  assert.equal(entry.entryMode(null, ['bomb-code:panel-cubes:v1']).mode, 'upgrade');
  assert.equal(entry.entryMode(JSON.stringify({ version: 1, seenIntroduction: true }), ['bomb-code:joe-corner']).mode, 'returning');
  assert.equal(entry.entryMode('{', ['bomb-code.pause-visual-motion']).mode, 'upgrade');
  assert.equal(entry.entryMode('{', []).mode, 'work');
});

test('restoration rejects oversized, secret, and malformed state without truncating it', () => {
  const oversized = entry.parseRestoration(`{"version":1,"threadId":"${'x'.repeat(5000)}"}`);
  assert.equal(oversized.withheld, true);
  assert.equal(entry.parseRestoration(JSON.stringify({ version: 1, transcript: 'private' })).ok, false);
  assert.equal(entry.parseRestoration('{').ok, false);
  const parsed = entry.parseRestoration(record);
  assert.equal(parsed.value.threadId, 'thread-1');
  assert.equal(parsed.value.secondary.length, 1);
  assert.equal(JSON.stringify(parsed.value).includes('transcript'), false);
});

test('user input and missing bootstrap block restore; a deleted thread falls back to the project', () => {
  const parsed = entry.parseRestoration(record);
  assert.equal(entry.restorationDecision({ record: parsed, sessions: [{ id: 'thread-1' }] }).reason, 'waiting');
  assert.equal(entry.restorationDecision({ bootstrapped: true, userTouched: true, record: parsed, sessions: [{ id: 'thread-1' }] }).reason, 'user');
  const missing = entry.restorationDecision({ bootstrapped: true, record: parsed, sessions: [] });
  assert.equal(missing.fallback, 'project');
  assert.equal(missing.selectThread, false);
  const ready = entry.restorationDecision({ bootstrapped: true, record: parsed, sessions: [{ id: 'thread-1' }] });
  assert.equal(ready.selectThread, true);
});

test('rectangles clamp into the current window and oversized saves are withheld', () => {
  const clamped = entry.clampRect({ x: -40, y: 900, width: 800, height: 500 }, { width: 320, height: 240 });
  assert.deepEqual(clamped, { x: 0, y: 0, width: 320, height: 240 });
  const saved = entry.serializeRestoration({ threadId: 'thread-1', primary: rect, secondary: [] });
  assert.equal(saved.ok, true);
  assert.equal(saved.text.includes('thread-1'), true);
  const huge = entry.serializeRestoration({ threadId: 't'.repeat(500), secondary: Array.from({ length: 8 }, (_, i) => ({ id: `${i}${'p'.repeat(400)}`, ...rect })) });
  assert.equal(huge.withheld, true);
});

test('entry routes stay inside existing views', () => {
  assert.equal(entry.ROUTES.explore.view, 'spatial');
  assert.equal(entry.ROUTES.collaborate.view, 'spatial');
  assert.equal(entry.ROUTES.experiment.view, 'builds');
  assert.equal(entry.ROUTES.memory.view, 'memory');
  assert.equal(Object.values(entry.ROUTES).some(route => route.invoke || route.command), false);
});

test('welcome controls stay hidden after Stop because flex does not override the hidden attribute', () => {
  const css = readFileSync(new URL('./studio-entry.css', import.meta.url), 'utf8');
  assert.match(css, /\.studio-tour\[hidden\]\s*\{\s*display:\s*none;/);
});
