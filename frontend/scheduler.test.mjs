import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createContext, runInContext } from 'node:vm';

const source = readFileSync(new URL('./scheduler.js', import.meta.url), 'utf8');
function harness(job, { confirmed = true, commandError = null } = {}) {
  let jobs = [job];
  const elements = new Map(), calls = [], confirmations = [];
  const element = id => {
    if (!elements.has(id)) elements.set(id, {
      classList: { contains: () => true, toggle() {} }, querySelectorAll: () => [],
      textContent: '', innerHTML: '',
    });
    return elements.get(id);
  };
  const detail = element('scheduler-detail');
  const view = element('view-scheduler');
  const context = createContext({
    $: id => {
      if (['scheduler-pause', 'scheduler-resume', 'scheduler-delete'].includes(id)
        && !detail.innerHTML.includes(`id="${id}"`)) return null;
      return element(id);
    },
    document: { hidden: false, addEventListener() {} },
    window: { confirm: () => { throw new Error('unexpected fallback'); } },
    askConfirm: async (message, options) => { confirmations.push([message, options]); return confirmed; },
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === 'scheduler_list') return structuredClone(jobs);
      if (commandError) {
        jobs = [{ ...job, status: 'uncertain', error: commandError }];
        throw new Error(commandError);
      }
      jobs = [{ ...job, status: command === 'scheduler_cancel' ? 'cancelled' : 'scheduled' }];
    },
  });
  runInContext(source, context);
  return { context, elements, calls, confirmations, detail, view, replaceJobs: value => { jobs = value; }, refresh: () => context.window.BombScheduler.refresh() };
}
const job = (status, extra = {}) => ({ id: 'routine-1', name: 'Research', cwd: '/tmp/owned', prompt: 'Review', status,
  schedule: { interval: { secs: 60 } }, run_count: 1, runs: [], ...extra });
const run = outcome => ({ run_id: 'attempt-1', session_id: 'session-1', outcome });

test('restored interrupted binding offers deliberate new attempt without replaying automatically', async () => {
  const h = harness(job('interrupted', { active_run: run(null) }), { confirmed: false });
  await h.refresh();
  assert.match(h.detail.innerHTML, /Interrupted/);
  assert.match(h.detail.innerHTML, /attempt-1/);
  assert.match(h.detail.innerHTML, /No observed terminal result/);
  assert.match(h.detail.innerHTML, /id="scheduler-resume"/);
  assert.equal(h.calls.filter(([command]) => command !== 'scheduler_list').length, 0);
  await h.elements.get('scheduler-resume').onclick();
  assert.match(h.confirmations[0][0], /external effects may already have happened/);
  assert.equal(h.calls.filter(([command]) => command === 'scheduler_resume').length, 0);
});

test('confirmed recovery invokes the existing host resume and retains prior identity on screen', async () => {
  const h = harness(job('interrupted', { active_run: run(null) }));
  await h.refresh();
  await h.elements.get('scheduler-resume').onclick();
  assert.equal(h.calls.find(([command]) => command === 'scheduler_resume')[1].id, 'routine-1');
  assert.match(h.detail.innerHTML, /attempt-1/);
});

test('unconfirmed cleanup blocks enable and exposes retry with typed outcome and escaped errors', async () => {
  const h = harness(job('uncertain', { runs: [run({ cleanup_complete: false, error: '<unsafe>',
    process: { end: 'cleanup_failure', exit_code: 1, cleanup_scope: 'dedicated_process_group' } })] }));
  await h.refresh();
  assert.doesNotMatch(h.detail.innerHTML, /id="scheduler-resume"/);
  assert.match(h.detail.innerHTML, /Retry cleanup/);
  assert.match(h.detail.innerHTML, /cleanup_failure/);
  assert.match(h.detail.innerHTML, /exit 1/);
  assert.match(h.detail.innerHTML, /dedicated_process_group/);
  assert.match(h.detail.innerHTML, /&lt;unsafe&gt;/);
  assert.doesNotMatch(h.detail.innerHTML, /<unsafe>/);
  await h.elements.get('scheduler-delete').onclick();
  assert.equal(h.calls.find(([command]) => command === 'scheduler_cancel')[1].id, 'routine-1');
  assert.match(h.detail.innerHTML, /attempt-1/);
  assert.match(h.elements.get('scheduler-status').textContent, /history retained/);
});

test('pause of an active attempt cannot enable a concurrent attempt', async () => {
  const h = harness(job('paused', { active_run: run(null) }));
  await h.refresh();
  assert.doesNotMatch(h.detail.innerHTML, /id="scheduler-resume"/);
  assert.match(h.detail.innerHTML, /Stop routine/);
});

test('failed Stop refreshes authoritative uncertainty and keeps the recording failure visible', async () => {
  const h = harness(job('running', { active_run: run(null) }), { commandError: 'journal unavailable' });
  await h.refresh();
  await h.elements.get('scheduler-delete').onclick();
  assert.match(h.detail.innerHTML, /Uncertain/);
  assert.match(h.detail.innerHTML, /journal unavailable/);
  assert.equal(h.elements.get('scheduler-status').textContent, 'journal unavailable');
  assert.equal(h.calls.filter(([command]) => command === 'scheduler_list').length, 2);
});

test('an outcome without an observed process result cannot be presented as Success', async () => {
  const h = harness(job('failed', { runs: [run({ cleanup_complete: true, error: null, process: null })] }));
  await h.refresh();
  assert.match(h.detail.innerHTML, /No successful process completion observed/);
  assert.doesNotMatch(h.detail.innerHTML, /<dd>Success/);
  assert.match(h.detail.innerHTML, /Admitted attempts/);
});

test('two clicks during a pending confirmation can admit only one control request', async () => {
  const h = harness(job('paused'));
  let resolve, dialogs = 0;
  h.context.askConfirm = () => { dialogs++; return new Promise(r => { resolve = r; }); };
  await h.refresh();
  const click = h.elements.get('scheduler-resume').onclick;
  const first = click();
  await click();
  assert.equal(dialogs, 1);
  assert.match(h.detail.innerHTML, /id="scheduler-resume" disabled/);
  resolve(true);
  await first;
  assert.equal(h.calls.filter(([command]) => command === 'scheduler_resume').length, 1);
});

test('a late confirmation cannot act after its selected routine disappeared', async () => {
  const h = harness(job('paused'));
  let resolve;
  h.context.askConfirm = () => new Promise(r => { resolve = r; });
  await h.refresh();
  const pending = h.elements.get('scheduler-resume').onclick();
  h.replaceJobs([{ ...job('paused'), id: 'other-routine' }]);
  await h.refresh();
  resolve(true);
  await pending;
  assert.equal(h.calls.filter(([command]) => command === 'scheduler_resume').length, 0);
});

test('pending enabled creation blocks duplicates and preserves a newer prompt draft', async () => {
  const h = harness(job('paused'));
  for (const [id, value] of Object.entries({ 'scheduler-name': 'New routine', 'scheduler-prompt': 'Original prompt',
    'scheduler-cwd': '/tmp/owned', 'scheduler-kind': 'interval', 'scheduler-interval': '60', 'scheduler-max-runs': '' })) {
    h.context.$(id).value = value;
  }
  h.context.$('scheduler-enable').checked = true;
  let resolve, dialogs = 0;
  h.context.askConfirm = () => { dialogs++; return new Promise(r => { resolve = r; }); };
  const submit = h.context.$('scheduler-form').onsubmit;
  const pending = submit({ preventDefault() {} });
  await submit({ preventDefault() {} });
  assert.equal(dialogs, 1);
  h.context.$('scheduler-prompt').value = 'Newer draft';
  resolve(true);
  await pending;
  const created = h.calls.filter(([command]) => command === 'scheduler_add');
  assert.equal(created.length, 1);
  assert.equal(created[0][1].request.prompt, 'Original prompt');
  assert.equal(h.context.$('scheduler-prompt').value, 'Newer draft');
});
