// Build output is evidence for human review; it is always rendered as text.
(() => {
  const B = { builds: [], selected: null, busy: false, loading: false, signature: '' };
  const esc = escapeHtml;
  const roles = ['planner', 'implementer', 'auditor', 'verifier'];
  const labels = { planning: 'Planning', awaiting_plan_approval: 'Plan approval needed', implementing: 'Implementing', auditing: 'Auditing', verifying: 'Verifying', ready_for_review: 'Ready for your review', accepted: 'Accepted by you', needs_changes: 'Repair limit reached', stalled: 'Repeated findings', cancelled: 'Cancelled', failed: 'Failed', interrupted: 'Interrupted after restart' };
  const terminal = new Set(['accepted', 'needs_changes', 'stalled', 'cancelled', 'failed', 'interrupted']);
  const title = role => role.charAt(0).toUpperCase() + role.slice(1);
  const status = value => labels[value] || value.replaceAll('_', ' ');
  const visible = () => $('view-builds').classList.contains('active');
  function notice(message, error = false) {
    $('builds-status').textContent = message;
    $('builds-status').classList.toggle('builds-error', error);
  }
  $('builds-routes').innerHTML = roles.map(role => `<label>${title(role)}<select id="builds-${role}-engine" aria-label="${title(role)} engine"><option value="codex">Codex</option><option value="claude">Claude Code</option><option value="grok">Grok</option></select><input id="builds-${role}-model" type="text" maxlength="256" placeholder="Configured default model" aria-label="${title(role)} model" spellcheck="false" /></label>`).join('');

  function renderList() {
    $('builds-list').innerHTML = B.builds.map(w => `<button type="button" class="builds-row${w.id === B.selected ? ' selected' : ''}" data-id="${esc(w.id)}"><span class="builds-row-title">${esc(w.spec.objective)}</span><span class="builds-row-status">${esc(status(w.status))} · repair ${w.round}/${w.spec.max_repairs}</span><span class="builds-row-project">${esc(w.spec.project_root)}</span></button>`).join('') || '<p class="empty-hint">No builds yet. Start with a small change and specific write paths.</p>';
    $('builds-list').querySelectorAll('[data-id]').forEach(button => button.onclick = () => { B.selected = button.dataset.id; renderList(); renderDetail(); });
  }
  function renderDetail() {
    const w = B.builds.find(w => w.id === B.selected);
    if (!w) { $('builds-detail').innerHTML = '<p class="empty-hint">Select a build to inspect its plan, checks and result.</p>'; return; }
    const reviewing = w.status === 'ready_for_review';
    const approving = w.status === 'awaiting_plan_approval';
    const plan = w.plan == null ? '<p class="builds-help">The planner is preparing the plan.</p>' : `<pre class="builds-output">${esc(w.plan)}</pre>`;
    $('builds-detail').innerHTML = `<h2>${esc(w.spec.objective)}</h2><div class="builds-current">${esc(status(w.status))} · repair ${w.round}/${w.spec.max_repairs}</div><dl class="builds-meta"><dt>Project</dt><dd>${esc(w.spec.project_root)}</dd><dt>Write paths</dt><dd>${esc(w.spec.write_set.join(', '))}</dd>${w.worktree ? `<dt>Worktree</dt><dd>${esc(w.worktree)} <button id="builds-folder" class="btn ghost" type="button">Reveal</button></dd>` : ''}${w.base_commit ? `<dt>Baseline</dt><dd>${esc(w.base_commit)}</dd>` : ''}</dl>${w.error ? `<p class="builds-failure" role="status">${esc(w.error)}</p>` : ''}<div class="builds-actions">${w.active_session_id ? `<button class="btn ghost builds-session" data-session="${esc(w.active_session_id)}" type="button">Open active native session</button>` : ''}${approving ? `<button id="builds-approve" class="btn primary" type="button" ${B.busy || !w.approval_digest ? 'disabled' : ''}>Approve plan and implement</button>` : ''}${reviewing ? `<button id="builds-accept" class="btn primary" type="button" ${B.busy ? 'disabled' : ''}>Accept reviewed result</button>` : ''}${!terminal.has(w.status) ? `<button id="builds-cancel" class="btn ghost danger" type="button" ${B.busy ? 'disabled' : ''}>Cancel build</button>` : ''}</div>${approving ? '<p class="builds-help">Approval applies to this exact plan, task and checkout. Tool requests still use the native approval controls.</p>' : reviewing ? '<p class="builds-help">The agent checks passed. Inspect the worktree and verification evidence before accepting. Acceptance keeps the changes in their worktree.</p>' : w.status === 'accepted' ? '<p class="builds-help">You accepted this result. Changes remain available in the worktree.</p>' : ''}<section class="builds-plan"><h3>Plan</h3>${plan}</section>${w.findings ? `<details class="builds-evidence" open><summary>Latest repair findings</summary><pre class="builds-output">${esc(w.findings)}</pre></details>` : ''}<section class="builds-steps"><h3>Role evidence</h3>${w.steps.map((step, index) => `<details class="builds-evidence" ${index === w.steps.length - 1 ? 'open' : ''}><summary>${esc(title(step.role))} · repair ${step.round}</summary>${step.session_id ? `<button class="btn ghost builds-session" data-session="${esc(step.session_id)}" type="button">Open native session</button>` : ''}<pre class="builds-output">${esc(step.output)}</pre></details>`).join('') || '<p class="builds-help">Completed role outputs will appear here. Native tool approval requests appear in Session.</p>'}</section>`;
    if ($('builds-approve')) $('builds-approve').onclick = () => action('approve_build_plan', { id: w.id, digest: w.approval_digest });
    if ($('builds-accept')) $('builds-accept').onclick = () => action('accept_build', { id: w.id });
    if ($('builds-cancel')) $('builds-cancel').onclick = () => action('cancel_build', { id: w.id });
    if ($('builds-folder')) $('builds-folder').onclick = () => invoke('reveal_project', { cwd: w.worktree }).catch(e => notice(String(e), true));
    $('builds-detail').querySelectorAll('[data-session]').forEach(button => button.onclick = () => window.BombBuildsHost.openSession(button.dataset.session).catch(e => notice(String(e), true)));
  }
  async function refresh() {
    if (B.loading) return;
    B.loading = true;
    try {
      const builds = await invoke('list_builds');
      const signature = JSON.stringify(builds);
      B.builds = builds;
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
      await invoke(command, args);
      B.signature = ''; await refresh();
      notice(command === 'approve_build_plan' ? 'Plan approved. Implementation is starting; open its native Session for tool approvals.' : command === 'accept_build' ? 'Result accepted. Inspect or continue from its retained worktree.' : 'Build cancelled. Its worktree and evidence remain available.');
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
    notice('Preparing the checkout and starting a native planning session…');
    try {
      const workflow = await invoke('create_build', { spec });
      B.selected = workflow.id; B.signature = ''; $('builds-new').open = false;
      await refresh(); notice('Planning started. Review the full plan before approving implementation.');
    } catch (e) { notice(String(e), true); }
    finally { B.busy = false; $('builds-create').disabled = false; renderDetail(); }
  };
  $('builds-use-project').onclick = () => {
    $('builds-project').value = $('cwd').value;
    if (!$('builds-project').value) notice('Choose a project with the project selector, or enter its absolute folder path.', true);
  };
  $('builds-refresh').onclick = refresh;
  window.BombBuilds = { refresh: async () => { if (!$('builds-project').value) $('builds-project').value = $('cwd').value; await refresh(); } };
  setInterval(() => { if (visible() && !document.hidden && !B.busy) refresh(); }, 3000);
})();
