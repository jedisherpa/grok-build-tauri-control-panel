/** Recorded relationships only. Activity, names and shared folders never imply an edge. */
(function (global) {
  'use strict';
  const keys = ['planner', 'plan_approval', 'implementer', 'auditor', 'verifier', 'acceptance'];
  const names = ['Planner', 'Your plan approval', 'Implementer', 'Auditor', 'Verifier', 'Your acceptance'];
  const activeRoles = { planning: 'planner', implementing: 'implementer', auditing: 'auditor', verifying: 'verifier' };

  function progress(build) {
    const p = build?.progress;
    // A malformed or old host response has no trusted denominator.
    if (!p || p.basis !== 'workflow_checkpoints' || p.total !== 6 || !Number.isInteger(p.round) || p.round < 0 || p.round > 10 || p.round !== build.round ||
        !Number.isInteger(p.completed) || p.completed < 0 || p.completed > 6 || !Number.isInteger(p.percent) ||
        !Array.isArray(p.checkpoints) || p.checkpoints.length !== 6 ||
        !p.checkpoints.every((c, i) => c && typeof c === 'object' && c.key === keys[i] && typeof c.completed === 'boolean' && (c.session_id == null || typeof c.session_id === 'string')) ||
        p.completed !== p.checkpoints.filter(c => c.completed).length ||
        p.percent !== Math.floor(p.completed * 100 / 6) ||
        p.checkpoints.some((c, i) => c.completed && i > 0 && !p.checkpoints[i - 1].completed) ||
        (p.completed === 6 && build.status !== 'accepted')) return null;
    return p;
  }

  function validSnapshot(builds, sessions) {
    const roles = ['planner','implementer','auditor','verifier'];
    const states = ['planning','awaiting_plan_approval','implementing','auditing','verifying','ready_for_review','accepted','needs_changes','stalled','cancelled','failed','interrupted'];
    const text = value => typeof value === 'string';
    const round = value => Number.isInteger(value) && value >= 0 && value <= 10;
    const ids = new Set();
    return Array.isArray(builds) && Array.isArray(sessions) && builds.every(b => {
      if (!b || !text(b.id) || !b.id || ids.has(b.id)) return false;
      ids.add(b.id);
      return states.includes(b.status) && round(b.round) && b.spec && text(b.spec.objective) && text(b.spec.project_root) &&
        b.spec.roles && roles.every(key => b.spec.roles[key] && text(b.spec.roles[key].backend) && (b.spec.roles[key].model == null || text(b.spec.roles[key].model))) &&
        Array.isArray(b.spec.write_set) && b.spec.write_set.every(text) && round(b.spec.max_repairs) &&
        Array.isArray(b.dependencies) && b.dependencies.every(text) && Array.isArray(b.steps) &&
        b.steps.every(s => s && roles.includes(s.role) && round(s.round) && text(s.output) && (s.session_id == null || text(s.session_id))) &&
        (b.active_session_id == null || text(b.active_session_id)) && (b.plan == null || text(b.plan)) &&
        ((b.active_session_role == null && b.active_session_round == null) || (roles.includes(b.active_session_role) && activeRoles[b.status] === b.active_session_role && b.active_session_round === b.round && text(b.active_session_id)));
    }) && sessions.every(s => s && text(s.id) && text(s.status));
  }

  function buildGraph(builds) {
    const nodes = builds.map(b => ({ id: b.id, label: b.spec.objective, build: b, kind: 'build' }));
    const known = new Set(nodes.map(n => n.id));
    const edges = [];
    const seen = new Set();
    for (const b of builds) for (const parent of b.dependencies || []) {
      if (!known.has(parent)) {
        nodes.push({ id: parent, label: 'Missing prerequisite', kind: 'missing' });
        known.add(parent);
      }
      const key = JSON.stringify([parent, b.id]);
      if (!seen.has(key)) { edges.push({ from: parent, to: b.id, kind: 'prerequisite' }); seen.add(key); }
    }
    return layout(nodes, edges);
  }

  function layout(nodes, edges) {
    const index = new Map(nodes.map(n => [n.id, n]));
    const incoming = new Map(nodes.map(n => [n.id, 0]));
    const children = new Map(nodes.map(n => [n.id, []]));
    const level = new Map(nodes.map(n => [n.id, 0]));
    for (const e of edges) {
      if (!index.has(e.from) || !index.has(e.to)) continue;
      incoming.set(e.to, incoming.get(e.to) + 1); children.get(e.from).push(e.to);
    }
    const ready = nodes.filter(n => incoming.get(n.id) === 0).map(n => n.id);
    let visited = 0;
    for (let i = 0; i < ready.length; i++) {
      const id = ready[i]; visited++;
      for (const child of children.get(id)) {
        level.set(child, Math.max(level.get(child), level.get(id) + 1));
        incoming.set(child, incoming.get(child) - 1);
        if (incoming.get(child) === 0) ready.push(child);
      }
    }
    const cycle = visited !== nodes.length;
    const rows = new Map();
    for (const n of nodes) {
      n.level = cycle ? 0 : level.get(n.id);
      const row = rows.get(n.level) || 0; rows.set(n.level, row + 1);
      n.x = 24 + n.level * 292; n.y = 24 + row * 130;
    }
    return { nodes, edges, cycle, width: nodes.reduce((width, n) => Math.max(width, n.x + 280), 300), height: nodes.reduce((height, n) => Math.max(height, n.y + 120), 140) };
  }

  function roleGraph(build) {
    const measured = progress(build);
    const active = activeRoles[build.status];
    const steps = build.steps || [];
    const nodes = keys.map((key, i) => {
      const checkpoint = measured?.checkpoints[i];
      const round = key === 'planner' ? 0 : build.round;
      const step = [...steps].reverse().find(s => s.role === key && s.round === round);
      const owned = active === key && build.active_session_role === key && build.active_session_round === build.round;
      const session = checkpoint?.session_id || step?.session_id || (owned ? build.active_session_id : null);
      const done = !!checkpoint?.completed;
      const waiting = (key === 'plan_approval' && build.status === 'awaiting_plan_approval') || (key === 'acceptance' && build.status === 'ready_for_review');
      const state = done ? 'complete' : waiting ? 'waiting' : owned && build.active_session_id ? 'active' : step ? 'recorded' : 'pending';
      return { id: key, label: names[i], kind: ['plan_approval', 'acceptance'].includes(key) ? 'human' : 'role', state, session, route: build.spec.roles[key] || null };
    });
    return { nodes, edges: keys.slice(1).map((key, i) => ({ from: keys[i], to: key, kind: 'workflow_order' })) };
  }

  function sessionLink(builds, sessionId) {
    for (const build of builds) {
      if (build.active_session_id === sessionId) return { build, role: build.active_session_role || 'cleanup', round: build.active_session_round ?? build.round };
      const step = (build.steps || []).find(s => s.session_id === sessionId);
      if (step) return { build, role: step.role, round: step.round };
    }
    return null;
  }
  function graphPage(graph, page, size = 24) {
    const pages = Math.max(1, Math.ceil(graph.nodes.length / size));
    page = Math.min(Math.max(0, page), pages - 1);
    const nodes = graph.nodes.slice(page * size, (page + 1) * size).map(n => ({ ...n }));
    const ids = new Set(nodes.map(n => n.id));
    const edges = graph.edges.filter(e => ids.has(e.from) && ids.has(e.to));
    return { ...layout(nodes, edges), page, pages, totalNodes: graph.nodes.length, totalEdges: graph.edges.length };
  }
  global.BombCollaboration = { progress, validSnapshot, buildGraph, graphPage, roleGraph, sessionLink };
})(typeof window !== 'undefined' ? window : globalThis);
