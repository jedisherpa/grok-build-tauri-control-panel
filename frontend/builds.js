// Build output is evidence for human review; it is always rendered as text.
(() => {
  const B = { builds: [], selected: null, busy: false, loading: false, signature: '', dependencySignature: '', limit: 2, limitDirty: false };
  const esc = escapeHtml;
  const roles = ['planner', 'implementer', 'auditor', 'verifier'];
  const labels = { planning: 'Planning', awaiting_plan_approval: 'Plan approval needed', implementing: 'Implementing', auditing: 'Auditing', verifying: 'Verifying', ready_for_review: 'Ready for your review', accepted: 'Accepted by you', needs_changes: 'Repair limit reached', stalled: 'Repeated findings', cancelled: 'Cancelled', failed: 'Failed', interrupted: 'Interrupted after restart' };
  const queueLabels = { reserved: 'Slot and scope reserved', finished: 'Finished', waiting_dependencies: 'Waiting for accepted prerequisites', blocked_dependencies: 'Blocked by unsuccessful prerequisites', waiting_scope: 'Waiting for overlapping write paths', waiting_slot: 'Waiting for a concurrency slot', queued: 'Queued' };
  const queueText = value => queueLabels[value] || value || '';
  const terminal = new Set(['accepted', 'needs_changes', 'stalled', 'cancelled', 'failed', 'interrupted']);
  const title = role => role.charAt(0).toUpperCase() + role.slice(1);
  const status = value => labels[value] || value.replaceAll('_', ' ');
  const visible = () => $('view-builds').classList.contains('active');
  function notice(message, error = false) {
    $('builds-status').textContent = message;
    $('builds-status').classList.toggle('builds-error', error);
  }
  $('builds-routes').innerHTML = roles.map(role => `<label>${title(role)}<select id="builds-${role}-engine" aria-label="${title(role)} engine"><option value="codex">Codex</option><option value="claude">Claude Code</option><option value="grok">Grok</option></select><input id="builds-${role}-model" type="text" maxlength="256" placeholder="Configured default model" aria-label="${title(role)} model" spellcheck="false" /></label>`).join('');

  function selectBuild(id) { B.selected = id; renderList(); renderDetail(); }
  function refreshDependencies() {
    const signature = JSON.stringify(B.builds.map(w => [w.id, w.spec.objective, w.status]));
    if (signature === B.dependencySignature) return;
    const select = $('builds-dependencies');
    if (document.activeElement === select) return;
    B.dependencySignature = signature;
    const selected = new Set(Array.from(select.selectedOptions, option => option.value));
    select.innerHTML = B.builds.map(w => `<option value="${esc(w.id)}" ${selected.has(w.id) ? 'selected' : ''}>${esc(w.id.slice(0, 8))} · ${esc(w.spec.objective.slice(0, 100))} · ${esc(status(w.status))}</option>`).join('');
    select.disabled = !B.builds.length;
  }
  function renderList() {
    $('builds-list').innerHTML = B.builds.map(w => `<button type="button" class="builds-row${w.id === B.selected ? ' selected' : ''}" data-id="${esc(w.id)}"><span class="builds-row-title">${esc(w.spec.objective)}</span><span class="builds-row-status">${esc(w.cleanup_pending && terminal.has(w.status) ? 'Native cleanup required' : w.queue_state && !['reserved', 'finished'].includes(w.queue_state) ? queueText(w.queue_state) : status(w.status))} · repair ${w.round}/${w.spec.max_repairs}</span><span class="builds-row-id">${esc(w.id.slice(0, 8))}</span><span class="builds-row-project">${esc(w.spec.project_root)}</span></button>`).join('') || '<p class="empty-hint">No builds yet. Start with a small change and specific write paths.</p>';
    $('builds-list').querySelectorAll('[data-id]').forEach(button => button.onclick = () => selectBuild(button.dataset.id));
  }
  function renderDetail() {
    const w = B.builds.find(w => w.id === B.selected);
    if (!w) { $('builds-detail').innerHTML = '<p class="empty-hint">Select a build to inspect its plan, checks and result.</p>'; return; }
    const cleanupRequired = w.cleanup_pending && terminal.has(w.status);
    const reviewing = w.status === 'ready_for_review';
    const approving = w.status === 'awaiting_plan_approval';
    const plan = w.plan == null ? `<p class="builds-help">${terminal.has(w.status) ? 'The build stopped before a plan was recorded.' : w.queue_state && w.queue_state !== 'reserved' ? esc(queueText(w.queue_state)) + '. Planning begins when the task is eligible.' : 'The planner is preparing the plan.'}</p>` : `<pre class="builds-output">${esc(w.plan)}</pre>`;
    $('builds-detail').innerHTML = `<h2>${esc(w.spec.objective)}</h2><div class="builds-current">${esc(w.status === 'planning' && w.queue_state && !['reserved', 'finished'].includes(w.queue_state) ? 'Queued' : status(w.status))} · repair ${w.round}/${w.spec.max_repairs}</div><p class="builds-queue">${esc(queueText(w.queue_state))}</p>${cleanupRequired ? '<p class="builds-failure" role="status">Bomb Code could not confirm that the native session stopped. This build retains its concurrency slot and write paths until cleanup succeeds. Retry cleanup to release them; the worktree and evidence remain available.</p>' : ''}<dl class="builds-meta"><dt>Build ID</dt><dd><input class="builds-id" aria-label="Build ID" value="${esc(w.id)}" readonly /></dd><dt>Project</dt><dd>${esc(w.spec.project_root)}</dd><dt>Write paths</dt><dd>${esc(w.spec.write_set.join(', '))}</dd>${w.worktree ? `<dt>Worktree</dt><dd>${esc(w.worktree)} <button id="builds-folder" class="btn ghost" type="button">Reveal</button></dd>` : ''}${w.base_commit ? `<dt>Baseline</dt><dd>${esc(w.base_commit)}</dd>` : ''}</dl>${w.dependencies?.length ? `<section class="builds-prerequisites"><h3>Prerequisite builds</h3>${w.dependencies.map(id => { const dependency = B.builds.find(candidate => candidate.id === id); return `<button class="btn ghost builds-dependency" data-build="${esc(id)}" type="button">${esc(id.slice(0, 8))} · ${esc(dependency?.spec.objective || 'Unavailable prerequisite')} · ${esc(dependency ? status(dependency.status) : 'Missing')}</button>`; }).join('')}<p class="builds-help">Every prerequisite requires your acceptance before this build starts. Each result remains in its separate worktree.</p></section>` : ''}${w.error ? `<p class="builds-failure" role="status">${esc(w.error)}</p>` : ''}<div class="builds-actions">${cleanupRequired ? `<button id="builds-retry-cleanup" class="btn primary" type="button" ${B.busy ? 'disabled' : ''}>Retry native cleanup</button>` : ''}${w.active_session_id ? `<button class="btn ghost builds-session" data-session="${esc(w.active_session_id)}" type="button">Open active native session</button>` : ''}${approving ? `<button id="builds-approve" class="btn primary" type="button" ${B.busy || w.cleanup_pending || !w.approval_digest ? 'disabled' : ''}>Approve plan and implement</button>` : ''}${reviewing ? `<button id="builds-accept" class="btn primary" type="button" ${B.busy || w.cleanup_pending ? 'disabled' : ''}>Accept reviewed result</button>` : ''}${!terminal.has(w.status) ? `<button id="builds-cancel" class="btn ghost danger" type="button" ${B.busy ? 'disabled' : ''}>Cancel build</button>` : ''}</div>${approving ? '<p class="builds-help">Approval applies to this exact plan, task and checkout. Tool requests still use the native approval controls.</p>' : reviewing ? '<p class="builds-help">The agent checks passed. Inspect the worktree and verification evidence before accepting. Acceptance keeps the changes in their worktree.</p>' : w.status === 'accepted' ? '<p class="builds-help">You accepted this result. Changes remain available in the worktree.</p>' : ''}<section class="builds-plan"><h3>Plan</h3>${plan}</section>${w.findings ? `<details class="builds-evidence" open><summary>Latest repair findings</summary><pre class="builds-output">${esc(w.findings)}</pre></details>` : ''}<section class="builds-steps"><h3>Role evidence</h3>${w.steps.map((step, index) => `<details class="builds-evidence" ${index === w.steps.length - 1 ? 'open' : ''}><summary>${esc(title(step.role))} · repair ${step.round}</summary>${step.session_id ? `<button class="btn ghost builds-session" data-session="${esc(step.session_id)}" type="button">Open native session</button>` : ''}<pre class="builds-output">${esc(step.output)}</pre></details>`).join('') || '<p class="builds-help">Completed role outputs will appear here. Native tool approval requests appear in Session.</p>'}</section>`;
    if ($('builds-approve')) $('builds-approve').onclick = () => action('approve_build_plan', { id: w.id, digest: w.approval_digest });
    if ($('builds-accept')) $('builds-accept').onclick = () => action('accept_build', { id: w.id });
    if ($('builds-cancel')) $('builds-cancel').onclick = () => action('cancel_build', { id: w.id });
    if ($('builds-retry-cleanup')) $('builds-retry-cleanup').onclick = () => action('retry_build_cleanup', { id: w.id });
    if ($('builds-folder')) $('builds-folder').onclick = () => invoke('reveal_project', { cwd: w.worktree }).catch(e => notice(String(e), true));
    $('builds-detail').querySelectorAll('[data-build]').forEach(button => button.onclick = () => selectBuild(button.dataset.build));
    $('builds-detail').querySelectorAll('[data-session]').forEach(button => button.onclick = () => window.BombBuildsHost.openSession(button.dataset.session).catch(e => notice(String(e), true)));
  }
  async function refresh() {
    if (B.loading) return;
    B.loading = true;
    try {
      const [builds, limit] = await Promise.all([invoke('list_builds'), invoke('get_build_concurrency')]);
      const signature = JSON.stringify(builds);
      B.builds = builds;
      B.limit = limit;
      $('builds-concurrency-current').textContent = `Current limit: ${B.limit}`;
      if (!B.limitDirty && document.activeElement !== $('builds-concurrency')) $('builds-concurrency').value = String(B.limit);
      refreshDependencies();
      if (signature !== B.signature) {
        B.signature = signature;
        if (!B.selected && builds.length) B.selected = builds[0].id;
        renderList(); renderDetail();
      }
    } catch (e) { notice(`Builds unavailable: ${e}`, true); }
    finally { B.loading = false; }
  }
  async function action(command, args) {
    if (B.busy) return;
    B.busy = true; renderDetail();
    try {
      const result = await invoke(command, args);
      B.signature = ''; await refresh();
      if (result?.cleanup_pending && terminal.has(result.status)) {
        notice('Native cleanup remains pending. This build still reserves its concurrency slot and write paths. Open the build details to retry cleanup.', true);
        return;
      }
      notice(command === 'approve_build_plan' ? 'Plan approved. Implementation is starting; open its native Session for tool approvals.' : command === 'accept_build' ? 'Result accepted. Eligible dependent builds can now start. The accepted worktree remains available.' : command === 'retry_build_cleanup' ? 'Native cleanup confirmed. This build released its concurrency slot and write paths; its worktree and evidence remain available.' : 'Build cancelled. Its worktree and evidence remain available.');
    } catch (e) { notice(String(e), true); }
    finally { B.busy = false; renderDetail(); }
  }
  $('builds-form').onsubmit = async event => {
    event.preventDefault();
    if (B.busy) return;
    const spec = {
      project_root: $('builds-project').value.trim(), objective: $('builds-objective').value.trim(),
      write_set: $('builds-scope').value.split('\n').map(s => s.trim()).filter(Boolean),
      max_repairs: Number($('builds-repairs').value),
      roles: Object.fromEntries(roles.map(role => [role, { backend: $(`builds-${role}-engine`).value, model: $(`builds-${role}-model`).value.trim() || null }]))
    };
    B.busy = true; $('builds-create').disabled = true;
    notice('Submitting the reviewed build and its prerequisites…');
    try {
      const dependencies = Array.from($('builds-dependencies').selectedOptions, option => option.value);
      const workflow = await invoke('create_build', { spec, dependencies });
      B.selected = workflow.id; B.signature = ''; $('builds-new').open = false;
      await refresh(); notice('Build submitted. Review its queue state and approve the full plan when planning completes.');
    } catch (e) { notice(String(e), true); }
    finally { B.busy = false; $('builds-create').disabled = false; renderDetail(); }
  };
  $('builds-use-project').onclick = () => {
    $('builds-project').value = $('cwd').value;
    if (!$('builds-project').value) notice('Choose a project with the project selector, or enter its absolute folder path.', true);
  };
  $('builds-concurrency').onchange = () => { B.limitDirty = true; };
  $('builds-concurrency-save').onclick = async () => {
    if (B.busy) return;
    B.busy = true; $('builds-concurrency-save').disabled = true;
    try {
      B.limit = await invoke('set_build_concurrency', { limit: Number($('builds-concurrency').value) });
      B.limitDirty = false;
      $('builds-concurrency').value = String(B.limit);
      $('builds-concurrency-current').textContent = `Current limit: ${B.limit}`;
      notice(`Concurrency limit saved: ${B.limit}. Queued builds use the new limit.`);
      B.signature = ''; await refresh();
    } catch (e) { notice(String(e), true); }
    finally { B.busy = false; $('builds-concurrency-save').disabled = false; }
  };
  $('builds-refresh').onclick = refresh;
  window.BombBuilds = { refresh: async () => { if (!$('builds-project').value) $('builds-project').value = $('cwd').value; await refresh(); } };
  setInterval(() => { if (visible() && !document.hidden && !B.busy) refresh(); }, 3000);
})();
