// Build output is evidence for human review; it is always rendered as text.
(() => {
  const B = { builds: [], sessions: [], selected: null, busy: false, loading: false, signature: '', dependencySignature: '', limit: 2, limitDirty: false, fresh: false, sampledAt: null, graphPage: 0, edgePage: 0 };
  const C = window.BombCollaboration;
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

  function selectBuild(id) { B.selected = id; const index = B.builds.findIndex(w => w.id === id); if (index >= 0) B.graphPage = Math.floor(index / 24); renderList(); renderGraph(); renderDetail(); }
  function checkpointText(w) {
    const p = C.progress(w);
    return p ? `${p.percent}% workflow checkpoints · ${p.completed}/${p.total} completed` : 'No workflow checkpoints yet';
  }
  function renderGraph() {
    const viewport = $('collaboration-graph');
    const position = [viewport.scrollLeft, viewport.scrollTop];
    const fullGraph = C.buildGraph(B.builds);
    const graph = C.graphPage(fullGraph, B.graphPage); B.graphPage = graph.page;
    const nodeMap = new Map(graph.nodes.map(n => [n.id, n]));
    const lines = graph.edges.map(e => {
      const a = nodeMap.get(e.from), b = nodeMap.get(e.to);
      const x = a.x + 252, y = a.y + 53, end = b.x - 6, targetY = b.y + 53;
      return `<path d="M ${x} ${y} C ${x + 28} ${y}, ${end - 28} ${targetY}, ${end} ${targetY}" />`;
    }).join('');
    viewport.innerHTML = !B.builds.length ? '<p class="empty-hint">Submit a reviewed build to record a workflow. Separate sessions alone establish no collaboration edges.</p>' : `<div class="collaboration-canvas" style="width:${graph.width}px;height:${graph.height}px"><svg width="${graph.width}" height="${graph.height}" aria-hidden="true"><defs><marker id="collaboration-arrow" markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6" /></marker></defs><g class="collaboration-edges" marker-end="url(#collaboration-arrow)">${lines}</g></svg>${graph.nodes.map(n => n.kind === 'missing' ? `<div class="collaboration-node missing" style="left:${n.x}px;top:${n.y}px"><strong>Missing prerequisite</strong><span>${esc(n.id)}</span></div>` : `<button type="button" class="collaboration-node${n.id === B.selected ? ' selected' : ''}" data-graph-build="${esc(n.id)}" style="left:${n.x}px;top:${n.y}px" aria-pressed="${n.id === B.selected}"><span class="collaboration-node-id">${esc(n.id.slice(0, 8))} · ${esc(status(n.build.status))}</span><strong>${esc(n.label)}</strong><span>${esc(checkpointText(n.build))}</span></button>`).join('')}</div>`;
    viewport.scrollLeft = position[0]; viewport.scrollTop = position[1];
    viewport.querySelectorAll('[data-graph-build]').forEach(button => button.onclick = () => selectBuild(button.dataset.graphBuild));
    const edgePages = Math.max(1, Math.ceil(fullGraph.edges.length / 20)); B.edgePage = Math.min(B.edgePage, edgePages - 1);
    const edges = fullGraph.edges.slice(B.edgePage * 20, (B.edgePage + 1) * 20);
    $('collaboration-links').innerHTML = `<div class="collaboration-pagination"><button type="button" class="btn ghost" id="graph-previous" ${graph.page ? '' : 'disabled'}>Previous builds</button><span>Graph page ${graph.page + 1}/${graph.pages} · ${graph.totalNodes} nodes · ${graph.edges.length}/${graph.totalEdges} edges shown</span><button type="button" class="btn ghost" id="graph-next" ${graph.page + 1 < graph.pages ? '' : 'disabled'}>Next builds</button></div><p class="builds-help">Edges crossing graph pages remain in the recorded prerequisite list.</p>${fullGraph.cycle ? '<p class="builds-failure">The stored dependency graph contains a cycle. Layout cannot represent a valid execution order.</p>' : ''}${fullGraph.edges.length ? `<details><summary>Recorded prerequisite edges · ${fullGraph.edges.length}</summary><div class="collaboration-pagination"><button type="button" class="btn ghost" id="edges-previous" ${B.edgePage ? '' : 'disabled'}>Previous edges</button><span>Edge page ${B.edgePage + 1}/${edgePages}</span><button type="button" class="btn ghost" id="edges-next" ${B.edgePage + 1 < edgePages ? '' : 'disabled'}>Next edges</button></div><ul>${edges.map(e => `<li><button class="btn ghost" data-graph-build="${esc(e.from)}">${esc(e.from.slice(0,8))}</button> → <button class="btn ghost" data-graph-build="${esc(e.to)}">${esc(e.to.slice(0,8))}</button> · acceptance required before start</li>`).join('')}</ul></details>` : '<p class="builds-help">No recorded prerequisites between these builds.</p>'}`;
    $('graph-previous').onclick = () => { B.graphPage--; renderGraph(); };
    $('graph-next').onclick = () => { B.graphPage++; renderGraph(); };
    if ($('edges-previous')) $('edges-previous').onclick = () => { B.edgePage--; renderGraph(); };
    if ($('edges-next')) $('edges-next').onclick = () => { B.edgePage++; renderGraph(); };
    $('collaboration-links').querySelectorAll('[data-graph-build]').forEach(button => button.onclick = () => selectBuild(button.dataset.graphBuild));
    const unlinked = B.sessions.filter(s => !C.sessionLink(B.builds, s.id));
    $('collaboration-sessions').innerHTML = unlinked.length ? `<p class="builds-help">No recorded build links for these sessions. Shared names or folders do not establish collaboration.</p>${unlinked.map(s => `<button class="btn ghost builds-session" data-session="${esc(s.id)}">${esc(s.id.slice(0,8))} · ${esc(s.backend || s.mode || 'native')} · ${esc(s.status)} · not linked to a build</button>`).join('')}` : '<p class="builds-help">No other live native sessions in this snapshot.</p>';
    $('collaboration-sessions').querySelectorAll('[data-session]').forEach(button => button.onclick = () => window.BombBuildsHost.openSession(button.dataset.session).catch(e => notice(String(e), true)));
  }
  function renderProgress(w) {
    const p = C.progress(w);
    const graph = C.roleGraph(w);
    const states = { complete: 'Recorded checkpoint complete', waiting: 'Waiting for you', active: 'Native session active', recorded: 'Report recorded; checkpoint not passed', pending: 'No completion evidence' };
    return `<section class="build-progress" aria-label="Workflow completion"><div class="build-progress-heading"><strong>${esc(checkpointText(w))}</strong><span class="builds-help">repair ${w.round}</span></div>${p ? `<progress value="${p.completed}" max="${p.total}" aria-label="${esc(checkpointText(w))}"></progress>` : ''}<p class="builds-help">Six equally weighted checkpoints: plan, approval, implementation, audit, verification and acceptance. This measures recorded workflow completion; remaining time and progress within a role are unknown. Repairs reset implementation and review checkpoints.</p><h3>Role sessions and gates</h3><ol class="collaboration-role-flow" aria-label="Declared workflow order">${graph.nodes.map(n => `<li class="role-node ${n.state}${n.kind === 'human' ? ' human-gate' : ''}"><span class="role-node-title">${esc(n.label)}</span><span>${esc(states[n.state])}</span>${n.route ? `<span class="builds-help">${esc(n.route.backend)} · ${esc(n.route.model || 'configured model')}</span>` : ''}${n.session ? `<button type="button" class="btn ghost builds-session" data-session="${esc(n.session)}">Session ${esc(n.session.slice(0,8))}</button>` : ''}</li>`).join('')}</ol><p class="builds-help">Arrows show declared workflow order. Session links come from recorded role attempts in this repair round; earlier attempts remain in Role evidence below.</p></section>`;
  }
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
    $('builds-list').innerHTML = B.builds.map(w => `<button type="button" class="builds-row${w.id === B.selected ? ' selected' : ''}" data-id="${esc(w.id)}"><span class="builds-row-title">${esc(w.spec.objective)}</span><span class="builds-row-status">${esc(w.cleanup_pending && terminal.has(w.status) ? 'Native cleanup required' : w.queue_state && !['reserved', 'finished'].includes(w.queue_state) ? queueText(w.queue_state) : status(w.status))} · repair ${w.round}/${w.spec.max_repairs}</span><span class="builds-row-progress">${esc(checkpointText(w))}</span><span class="builds-row-id">${esc(w.id.slice(0, 8))}</span><span class="builds-row-project">${esc(w.spec.project_root)}</span></button>`).join('') || '<p class="empty-hint">No builds yet. Start with a small change and specific write paths.</p>';
    $('builds-list').querySelectorAll('[data-id]').forEach(button => button.onclick = () => selectBuild(button.dataset.id));
  }
  function renderDetail() {
    const w = B.builds.find(w => w.id === B.selected);
    if (!w) { $('builds-detail').innerHTML = '<p class="empty-hint">Select a build to inspect its plan, checks and result.</p>'; return; }
    const cleanupRequired = w.cleanup_pending && terminal.has(w.status);
    const reviewing = w.status === 'ready_for_review';
    const approving = w.status === 'awaiting_plan_approval';
    const plan = w.plan == null ? `<p class="builds-help">${terminal.has(w.status) ? 'The build stopped before a plan was recorded.' : w.queue_state && w.queue_state !== 'reserved' ? esc(queueText(w.queue_state)) + '. Planning begins when the task is eligible.' : 'The planner is preparing the plan.'}</p>` : `<pre class="builds-output">${esc(w.plan)}</pre>`;
    $('builds-detail').innerHTML = `<h2>${esc(w.spec.objective)}</h2><div class="builds-current">${esc(w.status === 'planning' && w.queue_state && !['reserved', 'finished'].includes(w.queue_state) ? 'Queued' : status(w.status))} · repair ${w.round}/${w.spec.max_repairs}</div><p class="builds-queue">${esc(queueText(w.queue_state))}</p>${renderProgress(w)}${cleanupRequired ? '<p class="builds-failure" role="status">Bomb Code could not confirm that the native session stopped. This build retains its concurrency slot and write paths until cleanup succeeds. Retry cleanup to release them; the worktree and evidence remain available.</p>' : ''}<dl class="builds-meta"><dt>Build ID</dt><dd><input class="builds-id" aria-label="Build ID" value="${esc(w.id)}" readonly /></dd><dt>Project</dt><dd>${esc(w.spec.project_root)}</dd><dt>Write paths</dt><dd>${esc(w.spec.write_set.join(', '))}</dd>${w.worktree ? `<dt>Worktree</dt><dd>${esc(w.worktree)} <button id="builds-folder" class="btn ghost" type="button">Reveal</button></dd>` : ''}${w.base_commit ? `<dt>Baseline</dt><dd>${esc(w.base_commit)}</dd>` : ''}</dl>${w.dependencies?.length ? `<section class="builds-prerequisites"><h3>Prerequisite builds</h3>${w.dependencies.map(id => { const dependency = B.builds.find(candidate => candidate.id === id); return `<button class="btn ghost builds-dependency" data-build="${esc(id)}" type="button">${esc(id.slice(0, 8))} · ${esc(dependency?.spec.objective || 'Unavailable prerequisite')} · ${esc(dependency ? status(dependency.status) : 'Missing')}</button>`; }).join('')}<p class="builds-help">Every prerequisite requires your acceptance before this build starts. Each result remains in its separate worktree.</p></section>` : ''}${w.error ? `<p class="builds-failure" role="status">${esc(w.error)}</p>` : ''}<div class="builds-actions">${cleanupRequired ? `<button id="builds-retry-cleanup" class="btn primary" type="button" ${B.busy ? 'disabled' : ''}>Retry native cleanup</button>` : ''}${w.active_session_id ? `<button class="btn ghost builds-session" data-session="${esc(w.active_session_id)}" type="button">Open active native session</button>` : ''}${approving ? `<button id="builds-approve" class="btn primary" type="button" ${B.busy || w.cleanup_pending || !w.approval_digest ? 'disabled' : ''}>Approve plan and implement</button>` : ''}${reviewing ? `<button id="builds-accept" class="btn primary" type="button" ${B.busy || w.cleanup_pending ? 'disabled' : ''}>Accept reviewed result</button>` : ''}${!terminal.has(w.status) ? `<button id="builds-cancel" class="btn ghost danger" type="button" ${B.busy ? 'disabled' : ''}>Cancel build</button>` : ''}</div>${approving ? '<p class="builds-help">Approval applies to this exact plan, task and checkout. Tool requests still use the native approval controls.</p>' : reviewing ? '<p class="builds-help">The agent checks passed. Inspect the worktree and verification evidence before accepting. Acceptance keeps the changes in their worktree.</p>' : w.status === 'accepted' ? '<p class="builds-help">You accepted this result. Changes remain available in the worktree.</p>' : ''}<section class="builds-plan"><h3>Plan</h3>${plan}</section>${w.findings ? `<details class="builds-evidence" open><summary>Latest repair findings</summary><pre class="builds-output">${esc(w.findings)}</pre></details>` : ''}<section class="builds-steps"><h3>Role evidence</h3>${w.steps.map((step, index) => `<details class="builds-evidence" ${index === w.steps.length - 1 ? 'open' : ''}><summary>${esc(title(step.role))} · repair ${step.round}</summary>${step.session_id ? `<button class="btn ghost builds-session" data-session="${esc(step.session_id)}" type="button">Open native session</button>` : ''}<pre class="builds-output">${esc(step.output)}</pre></details>`).join('') || '<p class="builds-help">Completed role outputs will appear here. Native tool approval requests appear in Session.</p>'}</section>`;
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
      const [builds, limit, sessions] = await Promise.all([invoke('list_builds'), invoke('get_build_concurrency'), invoke('list_sessions')]);
      if (!C.validSnapshot(builds, sessions) || !Number.isInteger(limit) || limit < 1 || limit > 4) throw new Error('Invalid collaboration snapshot');
      const signature = JSON.stringify([builds, sessions]);
      const previous = { builds: B.builds, sessions: B.sessions, limit: B.limit, selected: B.selected, signature: B.signature, dependencySignature: B.dependencySignature, sampledAt: B.sampledAt, graphPage: B.graphPage, edgePage: B.edgePage };
      try {
        B.builds = builds; B.sessions = sessions; B.limit = limit;
        refreshDependencies();
        if (signature !== B.signature) {
          if (!B.selected && builds.length) B.selected = builds[0].id;
          renderList(); renderGraph(); renderDetail();
        }
        B.signature = signature; B.sampledAt = Date.now(); B.fresh = true;
      } catch (error) {
        Object.assign(B, previous); B.signature = ''; B.fresh = false;
        try { refreshDependencies(); renderList(); renderGraph(); renderDetail(); } catch (_) { /* Last-good data still retained; stale banner remains visible. */ }
        throw error;
      }
      $('collaboration-snapshot').textContent = `Snapshot ${new Date(B.sampledAt).toLocaleTimeString()}`;
      $('collaboration-snapshot').classList.remove('builds-error');
      $('builds-concurrency-current').textContent = `Current limit: ${B.limit}`;
      if (!B.limitDirty && document.activeElement !== $('builds-concurrency')) $('builds-concurrency').value = String(B.limit);
      window.BombBuildsHost.refreshActivity();
    } catch (e) {
      B.fresh = false;
      $('collaboration-snapshot').textContent = 'Refresh failed · graph and percentages are the last known snapshot';
      $('collaboration-snapshot').classList.add('builds-error');
      window.BombBuildsHost.refreshActivity();
      const current = $('builds-status')?.textContent || '';
      if (!current || current.startsWith('Builds unavailable') || current.startsWith('Choose a clean')) notice(`Builds unavailable: ${e}`, true);
    }
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

  const engines = { codex: 'codex', claude: 'claude', 'claude code': 'claude', grok: 'grok' };
  function spokenRoutes(text) {
    const found = {};
    for (const line of String(text || '').split('\n')) {
      const match = line.match(/^\s*(planner|implementer|auditor|verifier)\s*:\s*(codex|claude(?:\s+code)?|grok)(?:\s*\/\s*(\S+))?\s*$/i);
      if (!match) continue;
      const backend = engines[match[2].toLowerCase()];
      if (!backend) continue;
      found[match[1].toLowerCase()] = { backend, model: match[3] || null };
    }
    return found;
  }
  const PART_CAP = 8;
  /** Numbered lines or ## headings become parts. Role lines stay routes, not parts. */
  function planSequence(text) {
    const roleLine = /^\s*(planner|implementer|auditor|verifier)\s*:/i;
    const stepLine = /^(?:#{2,3}\s+(.+)|(\d+)[.)]\s+(.+))\s*$/;
    const preface = [];
    const steps = [];
    let current = null;
    for (const line of String(text || '').split('\n')) {
      if (roleLine.test(line)) continue;
      const match = line.match(stepLine);
      if (match) {
        if (current) steps.push(current);
        current = { title: (match[1] || match[3] || '').trim(), body: [] };
        continue;
      }
      if (current) current.body.push(line);
      else preface.push(line);
    }
    if (current) steps.push(current);
    if (steps.length < 2) return { parts: [], omitted: 0 };
    const lead = preface.map(line => line.trim()).filter(Boolean).join('\n');
    const omitted = Math.max(0, steps.length - PART_CAP);
    const parts = steps.slice(0, PART_CAP).map((step, index) => {
      const detail = step.body.join('\n').trim();
      return {
        index: index + 1,
        title: step.title || `Part ${index + 1}`,
        objective: [lead, step.title, detail].filter(Boolean).join('\n\n'),
      };
    });
    return { parts, omitted };
  }
  function filesNamed(text) {
    const found = [];
    const re = /\b((?:[\w.-]+\/)*[\w.-]+\.(?:py|md|js|mjs|rs|toml|json|css|html|txt|yml|yaml))\b/gi;
    const protect = /\b(?:do not|don't|never)\s+(?:edit|change|touch|modify)\b|\bwithout (?:editing|changing|touching)\b/i;
    for (const sentence of String(text || '').split(/\n|(?<=[.!])\s+/)) {
      if (protect.test(sentence)) continue;
      for (const match of sentence.matchAll(re)) {
        const path = match[1].replace(/^\.\//, '');
        if (!found.includes(path)) found.push(path);
      }
    }
    return found;
  }
  function pathCovered(path, writeSet) {
    const writes = (writeSet || []).map(item => String(item || '').trim().replace(/^\.\//, '').replace(/\/+$/, '')).filter(Boolean);
    if (writes.includes('.')) return true;
    return writes.some(root => path === root || path.startsWith(`${root}/`));
  }
  function uncoveredFiles(part, writeSet) {
    return filesNamed(part.objective).filter(path => !pathCovered(path, writeSet));
  }
  function collectSpec() {
    const spoken = spokenRoutes($('builds-objective').value);
    for (const role of roles) {
      if (!spoken[role]) continue;
      $(`builds-${role}-engine`).value = spoken[role].backend;
      $(`builds-${role}-model`).value = spoken[role].model || '';
    }
    return {
      project_root: $('builds-project').value.trim(), objective: $('builds-objective').value.trim(),
      write_set: $('builds-scope').value.split('\n').map(s => s.trim()).filter(Boolean),
      max_repairs: Number($('builds-repairs').value),
      roles: Object.fromEntries(roles.map(role => [role, { backend: $(`builds-${role}-engine`).value, model: $(`builds-${role}-model`).value.trim() || null }]))
    };
  }
  function renderBindings() {
    const spec = collectSpec();
    const box = $('builds-bindings');
    if (!box) return;
    box.innerHTML = roles.map(role => {
      const route = spec.roles[role];
      return `<li>${esc(title(role))} · ${esc(route.backend)} · ${esc(route.model || 'configured default')}</li>`;
    }).join('');
    const sequence = planSequence($('builds-objective').value);
    const list = $('builds-sequence');
    const help = $('builds-submit-help');
    if (list) {
      if (!sequence.parts.length) {
        list.hidden = true;
        list.innerHTML = '';
      } else {
        list.hidden = false;
        list.innerHTML = sequence.parts.map(part => {
          const when = part.index === 1
            ? 'Starts first. You still approve its plan before any edit.'
            : `Starts after you accept part ${part.index - 1}.`;
          const missing = uncoveredFiles(part, spec.write_set);
          const gap = missing.length
            ? ` ${missing.join(', ')} ${missing.length === 1 ? 'is not a write path' : 'are not write paths'}.`
            : '';
          return `<li><strong>${part.index}. ${esc(part.title)}</strong> · ${esc(when)}${esc(gap)}</li>`;
        }).join('') + (sequence.omitted ? `<li>${sequence.omitted} later parts were left out. Approve eight at a time, then paste the rest.</li>` : '');
      }
    }
    if (help) {
      help.textContent = sequence.parts.length
        ? `Preview checks part 1 and starts nobody. Submit queues ${sequence.parts.length} builds. Only the first planner starts. You approve every plan before edits, and you accept each result before the next part starts.`
        : 'Preview checks the queue and starts nobody. Submit starts the planner. You approve the plan before any edit.';
    }
    const submit = $('builds-create');
    if (submit) submit.textContent = sequence.parts.length > 1 ? `Queue ${sequence.parts.length} builds` : 'Submit reviewed build';
    return sequence;
  }
  function renderPreview(preview, sequence) {
    const panel = $('builds-preview-panel');
    if (!panel) return;
    if (!preview) { panel.hidden = true; panel.innerHTML = ''; return; }
    const queue = queueText(preview.predicted_queue_state) || preview.predicted_queue_state || '';
    const rolesHtml = (preview.roles || []).map(r => `<li>${esc(r.role)} · ${esc(r.backend)}${r.model ? ` · ${esc(r.model)}` : ' · configured default'}</li>`).join('');
    const depsHtml = (preview.dependencies || []).length
      ? (preview.dependencies || []).map(d => `<li>${esc(d.id.slice(0, 8))} · ${esc(d.objective.slice(0, 80))} · ${esc(d.status)}${d.accepted ? ' · accepted' : ''}</li>`).join('')
      : '<li>None selected</li>';
    const notes = (preview.notes || []).map(n => `<li>${esc(n)}</li>`).join('');
    const parts = sequence?.parts || [];
    const missing = [...new Set(parts.flatMap(part => uncoveredFiles(part, preview.write_set || [])))];
    const gapHtml = missing.length
      ? `<p class="builds-help">${esc(missing.join(', '))} ${missing.length === 1 ? 'is not a write path' : 'are not write paths'}. Add ${missing.length === 1 ? 'it' : 'them'} before you queue, or that part cannot edit ${missing.length === 1 ? 'it' : 'them'}.</p>`
      : '';
    const partsHtml = parts.length
      ? `<h3>Sequence · ${parts.length} parts</h3><p class="builds-help">Only part 1 was checked against the project. Nothing was saved. Later parts start only after you accept the previous result.</p>${gapHtml}<ol>${parts.map(part => `<li>${esc(part.title)}</li>`).join('')}</ol>`
      : '';
    panel.hidden = false;
    panel.innerHTML = `<h3>Dry-run preview</h3>
<p class="builds-help">No build was saved. No worktree was created. No agents were started.</p>
${partsHtml}
<div class="builds-preview-meta">
<div><strong>Project</strong> · ${esc(preview.project_root || '')}</div>
<div><strong>HEAD</strong> · ${esc((preview.head_commit || '').slice(0, 12))}</div>
<div><strong>Write paths</strong> · ${esc((preview.write_set || []).join(', '))}</div>
<div><strong>Repairs</strong> · ${esc(String(preview.max_repairs ?? ''))}</div>
<div><strong>Concurrency limit</strong> · ${esc(String(preview.concurrency_limit ?? ''))}</div>
<div><strong>Predicted queue</strong> · ${esc(queue)}</div>
</div>
<details open><summary>Role engines</summary><ul>${rolesHtml}</ul></details>
<details><summary>Prerequisites</summary><ul>${depsHtml}</ul></details>
<ul>${notes}</ul>`;
  }
  async function dryRun() {
    if (B.busy) return;
    const spec = collectSpec();
    const sequence = planSequence(spec.objective);
    const previewSpec = sequence.parts.length ? { ...spec, objective: sequence.parts[0].objective } : spec;
    if (!previewSpec.project_root || !previewSpec.objective || !previewSpec.write_set.length) {
      if ((!previewSpec.project_root || !previewSpec.write_set.length) && $('builds-setup')) $('builds-setup').open = true;
      notice(previewSpec.objective ? 'Add the project folder and one write path. Preview starts nobody.' : 'Paste the work first. Number the parts when it is a sequence.', true);
      return;
    }
    B.busy = true;
    if ($('builds-preview')) $('builds-preview').disabled = true;
    if ($('builds-create')) $('builds-create').disabled = true;
    notice(sequence.parts.length ? `Checking part 1 of ${sequence.parts.length} without starting agents…` : 'Validating build without starting agents…');
    try {
      const dependencies = Array.from($('builds-dependencies').selectedOptions, option => option.value);
      const preview = await invoke('preview_build', { spec: previewSpec, dependencies });
      renderPreview(preview, sequence);
      const missing = [...new Set(sequence.parts.flatMap(part => uncoveredFiles(part, preview.write_set || spec.write_set)))];
      const gap = missing.length
        ? ` ${missing.join(', ')} ${missing.length === 1 ? 'is not a write path.' : 'are not write paths.'}`
        : '';
      notice(sequence.parts.length
        ? `Dry run ok — ${sequence.parts.length} parts, nothing submitted.${gap} Queue ${sequence.parts.length} builds starts the first planner. Later parts wait until you accept the previous result.`
        : 'Dry run ok — review the preview. Nothing was submitted.');
    } catch (e) {
      renderPreview(null);
      notice(String(e), true);
    } finally {
      B.busy = false;
      if ($('builds-preview')) $('builds-preview').disabled = false;
      if ($('builds-create')) $('builds-create').disabled = false;
    }
  }
  $('builds-form').onsubmit = async event => {
    event.preventDefault();
    if (B.busy) return;
    const spec = collectSpec();
    const sequence = planSequence(spec.objective);
    const parts = sequence.parts.length ? sequence.parts : [{ index: 1, title: '', objective: spec.objective }];
    B.busy = true; $('builds-create').disabled = true; if ($('builds-preview')) $('builds-preview').disabled = true;
    notice(parts.length > 1 ? `Queuing ${parts.length} reviewed builds. Only the first planner starts…` : 'Submitting the reviewed build and its prerequisites…');
    const created = [];
    try {
      let previous = null;
      for (const part of parts) {
        const dependencies = previous
          ? [previous]
          : Array.from($('builds-dependencies').selectedOptions, option => option.value);
        const workflow = await invoke('create_build', { spec: { ...spec, objective: part.objective }, dependencies });
        if (!workflow?.id) throw new Error(`Part ${part.index} did not return a build.`);
        previous = workflow.id;
        created.push(workflow.id);
      }
      B.selected = created[0]; B.signature = ''; $('builds-new').open = false; renderPreview(null);
      await refresh();
      notice(created.length > 1
        ? `Queued ${created.length} builds. The first planner can start now. The others wait until you accept the previous result. Approve each plan before any edit.`
        : 'Build submitted. Review its queue state and approve the full plan when planning completes.');
    } catch (e) {
      notice(created.length ? `Queued ${created.length} of ${parts.length} before a stop: ${e}. Later parts were not submitted.` : String(e), true);
    }
    finally { B.busy = false; $('builds-create').disabled = false; if ($('builds-preview')) $('builds-preview').disabled = false; renderDetail(); }
  };
  if ($('builds-preview')) $('builds-preview').onclick = () => dryRun();
  if ($('builds-create')?.addEventListener) $('builds-create').addEventListener('click', () => {
    const project = $('builds-project').value.trim();
    const paths = $('builds-scope').value.trim();
    if ((!project || !paths) && $('builds-setup')) $('builds-setup').open = true;
  });
  $('builds-objective').oninput = renderBindings;
  renderBindings();
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
  window.BombBuilds = { refresh: async () => { if (!$('builds-project').value) $('builds-project').value = $('cwd').value; await refresh(); }, sessionSummary: id => { const linked = C.sessionLink(B.builds, id); return linked ? { id: linked.build.id, role: linked.role, round: linked.round, text: checkpointText(linked.build), fresh: B.fresh && Date.now() - B.sampledAt < 15000 } : null; } };
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
  window.addEventListener('focus', refresh);
  refresh();
  setInterval(() => { if (!document.hidden && !B.busy && (visible() || B.sessions.length)) refresh(); }, 3000);
})();
