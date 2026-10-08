// Scheduler surface — list / create / pause / resume / stop agent routines.
// Jobs require an absolute cwd. New jobs default to paused unless Enable now
// is checked (and confirmed). Firing a job still needs a working LLM/agent.
(() => {
  const S = { jobs: [], selected: null, busy: false, confirming: false, loading: false };

  const statusLabel = {
    scheduled: 'Scheduled',
    running: 'Running',
    paused: 'Paused',
    completed: 'Completed',
    failed: 'Failed',
    cancelled: 'Cancelled',
    cancelling: 'Stopping',
    interrupted: 'Interrupted — review required',
    uncertain: 'Uncertain — review required',
  };

  const esc = typeof escapeHtml === 'function' ? escapeHtml : (s) => String(s)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

  function notice(message, error = false) {
    const el = $('scheduler-status');
    if (!el) return;
    el.textContent = message;
    el.classList.toggle('scheduler-error', !!error);
  }

  function visible() {
    return !!$('view-scheduler')?.classList.contains('active');
  }

  function formatWhen(value) {
    if (!value) return '—';
    try {
      const d = new Date(value);
      if (Number.isNaN(d.getTime())) return String(value);
      return d.toLocaleString();
    } catch (_) {
      return String(value);
    }
  }

  function scheduleText(schedule) {
    if (!schedule || typeof schedule !== 'object') return 'Unknown schedule';
    if (schedule.interval && schedule.interval.secs != null) {
      return `Every ${schedule.interval.secs}s`;
    }
    if (schedule.cron && schedule.cron.expr) {
      return `Cron (UTC): ${schedule.cron.expr}`;
    }
    if (schedule.once && schedule.once.delay_secs != null) {
      return `Once after ${schedule.once.delay_secs}s`;
    }
    // Internally-tagged fallbacks some serializers emit
    if (schedule.secs != null) return `Every ${schedule.secs}s`;
    if (schedule.expr) return `Cron (UTC): ${schedule.expr}`;
    if (schedule.delay_secs != null) return `Once after ${schedule.delay_secs}s`;
    return JSON.stringify(schedule);
  }

  function statusText(status) {
    const key = String(status || '').toLowerCase();
    return statusLabel[key] || String(status || 'unknown');
  }

  function syncScheduleFields() {
    const kind = $('scheduler-kind')?.value || 'interval';
    const show = (id, on) => { const el = $(id); if (el) el.hidden = !on; };
    show('scheduler-field-interval', kind === 'interval');
    show('scheduler-field-cron', kind === 'cron');
    show('scheduler-field-once', kind === 'once');
  }

  function collectRequest() {
    const name = ($('scheduler-name')?.value || '').trim();
    const prompt = ($('scheduler-prompt')?.value || '').trim();
    const cwd = ($('scheduler-cwd')?.value || '').trim();
    const kind = $('scheduler-kind')?.value || 'interval';
    const enable = !!$('scheduler-enable')?.checked;
    const maxRaw = ($('scheduler-max-runs')?.value || '').trim();
    const max_runs = maxRaw === '' ? null : Number(maxRaw);
    if (max_runs != null && (!Number.isFinite(max_runs) || max_runs < 1)) {
      throw new Error('Max runs must be a positive number, or left blank.');
    }
    const request = { name, prompt, cwd, enable, maxRuns: max_runs };
    if (kind === 'cron') {
      const cron = ($('scheduler-cron')?.value || '').trim();
      if (!cron) throw new Error('Cron expression is required.');
      request.cron = cron;
    } else if (kind === 'once') {
      const once = Number($('scheduler-once')?.value);
      if (!Number.isFinite(once) || once < 0) throw new Error('Once delay must be zero or more seconds.');
      request.onceDelaySecs = once;
    } else {
      const secs = Number($('scheduler-interval')?.value);
      if (!Number.isFinite(secs) || secs < 1) throw new Error('Interval must be at least 1 second.');
      request.intervalSecs = secs;
    }
    if (!name) throw new Error('Name is required.');
    if (!prompt) throw new Error('Prompt is required.');
    if (!cwd) throw new Error('Working directory (cwd) is required.');
    if (!cwd.startsWith('/')) throw new Error('Working directory must be an absolute path.');
    return request;
  }

  function renderList() {
    const list = $('scheduler-list');
    if (!list) return;
    if (!S.jobs.length) {
      list.innerHTML = '<p class="empty-hint">No scheduled jobs yet. Create one below — new jobs stay paused until you enable them.</p>';
      return;
    }
    list.innerHTML = S.jobs.map((job) => {
      const selected = job.id === S.selected ? ' selected' : '';
      return `<button type="button" class="scheduler-row${selected}" data-id="${esc(job.id)}">
        <span class="scheduler-row-title">${esc(job.name || '(unnamed)')}</span>
        <span class="scheduler-row-status">${esc(statusText(job.status))} · ${esc(scheduleText(job.schedule))}</span>
        <span class="scheduler-row-meta">next ${esc(formatWhen(job.next_run))} · attempts ${esc(String(job.run_count ?? 0))}${job.max_runs != null ? `/${esc(String(job.max_runs))}` : ''}</span>
        <span class="scheduler-row-cwd">${esc(job.cwd || '(no cwd)')}</span>
      </button>`;
    }).join('');
    list.querySelectorAll('[data-id]').forEach((btn) => {
      btn.onclick = () => { S.selected = btn.dataset.id; renderList(); renderDetail(); };
    });
  }

  function renderDetail() {
    const panel = $('scheduler-detail');
    if (!panel) return;
    const job = S.jobs.find((j) => j.id === S.selected);
    if (!job) {
      panel.innerHTML = '<p class="empty-hint">Select a job to inspect schedule, next run, and controls.</p>';
      return;
    }
    const st = String(job.status || '').toLowerCase();
    const canPause = st === 'scheduled' || st === 'running';
    const last = job.active_run || job.runs?.at(-1);
    const outcome = last?.outcome;
    const unconfirmed = outcome?.cleanup_complete === false;
    const active = st === 'running' || st === 'cancelling' || (st === 'paused' && !!job.active_run);
    const canResume = !active && !unconfirmed && ['paused', 'failed', 'cancelled', 'interrupted', 'uncertain'].includes(st);
    const canDelete = st !== 'cancelled' || unconfirmed;
    const process = outcome?.process;
    const disabled = S.busy || S.confirming;
    const result = outcome ? (process?.end || 'No successful process completion observed') : 'No observed terminal result';
    panel.innerHTML = `
      <h2>${esc(job.name || '(unnamed)')}</h2>
      <div class="scheduler-current">${esc(statusText(job.status))} · ${esc(scheduleText(job.schedule))}</div>
      <dl class="scheduler-meta">
        <dt>Job ID</dt><dd><code>${esc(job.id)}</code></dd>
        <dt>Working directory</dt><dd>${esc(job.cwd || '(missing — will not run)')}</dd>
        <dt>Prompt</dt><dd class="scheduler-prompt-body">${esc(job.prompt || '')}</dd>
        <dt>Created</dt><dd>${esc(formatWhen(job.created_at))}</dd>
        <dt>Last run</dt><dd>${esc(formatWhen(job.last_run))}</dd>
        <dt>Next run</dt><dd>${esc(formatWhen(job.next_run))}</dd>
        <dt>Admitted attempts</dt><dd>${esc(String(job.run_count ?? 0))}${job.max_runs != null ? ` / ${esc(String(job.max_runs))}` : ''}</dd>
        <dt>Last run ID</dt><dd><code>${esc(last?.run_id || '—')}</code></dd>
        <dt>Session ID</dt><dd><code>${esc(last?.session_id || '—')}</code></dd>
        <dt>Observed result</dt><dd>${esc(result)}${process?.exit_code != null ? ` · exit ${esc(process.exit_code)}` : ''}</dd>
        <dt>Cleanup</dt><dd>${outcome ? (outcome.cleanup_complete ? 'Confirmed for recorded scope' : 'Unconfirmed — retry cleanup before enabling') : 'No terminal cleanup observation'}${process?.cleanup_scope ? ` · ${esc(process.cleanup_scope)}` : ''}</dd>
        <dt>Error</dt><dd>${esc(job.error || outcome?.error || process?.error || '—')}</dd>
      </dl>
      <p class="scheduler-help">An admitted attempt is not a successful completion. Pausing prevents future attempts; Stop also requests cleanup of the current attempt and retains its history. Interrupted or uncertain work may already have produced effects: inspect the recorded session before retrying. Cleanup covers the recorded process scope; detached descendants remain outside that scope. Required native policy capabilities must be available before an enabled job can start.</p>
      <div class="scheduler-actions">
        ${canPause ? `<button type="button" class="btn ghost" id="scheduler-pause" ${disabled ? 'disabled' : ''}>Pause</button>` : ''}
        ${canResume ? `<button type="button" class="btn primary" id="scheduler-resume" ${disabled ? 'disabled' : ''}>Review / enable</button>` : ''}
        ${canDelete ? `<button type="button" class="btn ghost danger" id="scheduler-delete" ${disabled ? 'disabled' : ''}>${unconfirmed ? 'Retry cleanup' : 'Stop routine'}</button>` : ''}
      </div>`;
    if ($('scheduler-pause')) $('scheduler-pause').onclick = () => action('scheduler_pause', { id: job.id }, 'Job paused.');
    if ($('scheduler-resume')) $('scheduler-resume').onclick = () => confirmResume(job);
    if ($('scheduler-delete')) $('scheduler-delete').onclick = () => confirmDelete(job);
  }

  async function confirmResume(job) {
    const message = `Enable “${job.name}”? Inspect previous effects and the recorded session first. This admits a new attempt when due; the prior attempt stays in history. Its external effects may already have happened. It may spawn an agent in ${job.cwd || 'its cwd'}.`;
    await confirmedAction(job, message, 'Enable scheduled job', 'scheduler_resume', 'Job enabled.');
  }

  async function confirmDelete(job) {
    await confirmedAction(job, `Stop scheduled job “${job.name}”? Active work will be asked to stop and cleanup will be checked. Its run history will be retained.`, 'Stop scheduled job', 'scheduler_cancel', 'Job stopped; history retained.');
  }

  async function confirmedAction(job, message, title, command, success) {
    if (S.busy || S.confirming) return;
    S.confirming = true;
    renderDetail();
    let ok = false;
    try {
      ok = typeof askConfirm === 'function'
        ? await askConfirm(message, { title, kind: 'warning' })
        : window.confirm(message);
    } catch (e) {
      notice(String(e?.message || e), true);
    } finally {
      S.confirming = false;
      renderDetail();
    }
    if (ok && S.selected === job.id) await action(command, { id: job.id }, success);
  }

  async function action(command, args, okMessage) {
    if (S.busy || S.confirming) return;
    S.busy = true;
    renderDetail();
    try {
      await invoke(command, args);
      await refresh();
      notice(okMessage || 'Done.');
    } catch (e) {
      // Protective cleanup can happen even if recording it failed. Refresh the
      // host state, keeping that error visible instead of claiming success.
      await refresh();
      notice(String(e?.message || e), true);
      if (typeof toastError === 'function') toastError(e);
    } finally {
      S.busy = false;
      renderDetail();
    }
  }

  async function refresh() {
    if (S.loading) return;
    S.loading = true;
    try {
      const jobs = await invoke('scheduler_list');
      if (!Array.isArray(jobs)) throw new Error('Invalid scheduler list response');
      S.jobs = jobs.slice().sort((a, b) => String(a.created_at || '').localeCompare(String(b.created_at || '')));
      if (S.selected && !S.jobs.some((j) => j.id === S.selected)) S.selected = null;
      if (!S.selected && S.jobs.length) S.selected = S.jobs[0].id;
      renderList();
      renderDetail();
      notice(S.jobs.length ? `${S.jobs.length} scheduled job${S.jobs.length === 1 ? '' : 's'}.` : 'No scheduled jobs.');
    } catch (e) {
      notice(`Scheduler unavailable: ${e?.message || e}`, true);
      if (typeof toastError === 'function') toastError(e);
      const list = $('scheduler-list');
      if (list) list.innerHTML = `<p class="empty-hint">Could not load jobs: ${esc(String(e?.message || e))}</p>`;
    } finally {
      S.loading = false;
    }
  }

  async function createJob(event) {
    event?.preventDefault?.();
    if (S.busy || S.confirming) return;
    let request;
    try {
      request = collectRequest();
    } catch (e) {
      notice(String(e?.message || e), true);
      return;
    }
    if (request.enable) {
      S.confirming = true;
      if ($('scheduler-create')) $('scheduler-create').disabled = true;
      renderDetail();
      let ok = false;
      try {
        const message = `Create and enable “${request.name}”? It may spawn an agent in ${request.cwd} on the schedule you chose.`;
        ok = typeof askConfirm === 'function'
          ? await askConfirm(message, { title: 'Enable scheduled job', kind: 'warning' })
          : window.confirm(message);
      } catch (e) {
        notice(String(e?.message || e), true);
      } finally {
        S.confirming = false;
        if ($('scheduler-create')) $('scheduler-create').disabled = false;
        renderDetail();
      }
      if (!ok) return;
    }
    S.busy = true;
    if ($('scheduler-create')) $('scheduler-create').disabled = true;
    notice(request.enable ? 'Creating enabled job…' : 'Creating paused job…');
    try {
      const job = await invoke('scheduler_add', { request });
      S.selected = job?.id || S.selected;
      if ($('scheduler-name')?.value.trim() === request.name) $('scheduler-name').value = '';
      if ($('scheduler-prompt')?.value.trim() === request.prompt) $('scheduler-prompt').value = '';
      if ($('scheduler-enable')) $('scheduler-enable').checked = false;
      await refresh();
      notice(request.enable
        ? `Created and enabled “${job?.name || request.name}”.`
        : `Created “${job?.name || request.name}” paused. Resume when you want it to run.`);
    } catch (e) {
      notice(String(e?.message || e), true);
      if (typeof toastError === 'function') toastError(e);
    } finally {
      S.busy = false;
      if ($('scheduler-create')) $('scheduler-create').disabled = false;
    }
  }

  function wire() {
    if (!$('view-scheduler')) return;
    if ($('scheduler-kind')) $('scheduler-kind').onchange = syncScheduleFields;
    syncScheduleFields();
    if ($('scheduler-form')) $('scheduler-form').onsubmit = createJob;
    if ($('scheduler-refresh')) $('scheduler-refresh').onclick = () => refresh();
    if ($('scheduler-use-project')) {
      $('scheduler-use-project').onclick = () => {
        const cwd = $('cwd')?.value || '';
        if ($('scheduler-cwd')) $('scheduler-cwd').value = cwd;
        if (!cwd) notice('Choose a project in the sidebar, or type an absolute folder path.', true);
      };
    }
    document.addEventListener('visibilitychange', () => { if (!document.hidden && visible()) refresh(); });
  }

  wire();
  window.BombScheduler = { refresh };
})();
