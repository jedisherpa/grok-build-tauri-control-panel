/* Canonical committed projection owner. No provider calls or effect replay. */
(function (global) {
  "use strict";
  const LIMITS = Object.freeze({ rows: 2000, bytes: 4 * 1024 * 1024, sessions: 8, notifications: 512, pages: 512, operations: 2000 });
  const size = text => new TextEncoder().encode(String(text)).length;
  const sameStore = (a, b) => a && b && a.store_id === b.store_id && a.generation === b.generation;
  const seq = n => { if (!Number.isSafeInteger(n) || n < 0) throw new Error("Invalid event sequence"); return n; };
  // Matches persistence's kind_to_role; snapshot rows carry storage kinds,
  // while replay deltas already carry presentation roles.
  const roleForKind = kind => ({ prompt: "user", user: "user", agent: "agent", message: "agent", assistant: "agent",
    thought: "thought", tool: "tool", tool_call: "tool", plan: "plan", error: "error", term: "term", approval: "approval" }[kind] || "system");
  function createOwner(options) {
    const { invoke, listen } = options;
    const caches = new Map(), live = new Set(), deleted = new Set(), unlisteners = [];
    let identity = null, cursor = null, health = null, selected = null, epoch = 0;
    let chain = Promise.resolve(), replayQueued = false, stopped = false, started = false, starting = null, gap = false;
    const statusWatermarks = new Map();
    const call = (name, ...args) => options[name]?.(...args);
    function cache(id) {
      if (!caches.has(id)) caches.set(id, { rows: [], rowBySeq: new Map(), bytes: 0, omitted: 0, scanned: 0, watermark: 0, loaded: false,
        loading: false, error: null, status: null, pendingKnown: false, pending: [], uncertain: new Map(), operationOverflow: false });
      const entry = caches.get(id);
      caches.delete(id); caches.set(id, entry);
      while (caches.size > LIMITS.sessions) {
        const oldest = [...caches.keys()].find(key => key !== selected && key !== id);
        if (!oldest) break;
        caches.delete(oldest); call("onEvicted", oldest);
      }
      return entry;
    }
    function trim(c) {
      while (c.rows.length > LIMITS.rows || c.bytes > LIMITS.bytes) {
        const row = c.rows.shift(); c.rowBySeq.delete(row.seq); c.bytes -= size(row.body); c.omitted++;
      }
    }
    function put(c, row, append = false) {
      seq(row.seq);
      const found = c.rowBySeq.get(row.seq);
      if (found) {
        c.bytes -= size(found.body);
        found.body = append ? found.body + String(row.body || "") : String(row.body || "");
        found.at = row.at; c.bytes += size(found.body);
      } else if (append) {
        // A missing append target is not a complete row. Rebase explicitly.
        c.loaded = false; c.error = "A transcript patch needs a fresh snapshot. Retry history.";
      } else {
        const next = { seq: row.seq, role: row.role || "system", body: String(row.body || ""), at: row.at || "" };
        c.rows.push(next); c.rowBySeq.set(next.seq, next); c.bytes += size(next.body); c.scanned++;
      }
      trim(c);
    }
    function operation(c, record) {
      if (typeof record.operation_id !== "string") throw new Error("Invalid operation identity");
      seq(record.intent_seq);
      if (record.outcome_seq != null) seq(record.outcome_seq);
      // An observed outcome can explicitly report uncertain external effects.
      // Dispatching a permission response is not proof of tool execution.
      const settled = record.outcome_seq != null && ["observed_success", "completed", "completed_mock", "answer_recorded"].includes(record.result);
      if (settled) c.uncertain.delete(record.operation_id);
      else if (c.uncertain.size < LIMITS.operations || c.uncertain.has(record.operation_id)) c.uncertain.set(record.operation_id, {
        operation_id: record.operation_id, intent_seq: record.intent_seq, kind: String(record.kind || "operation").slice(0, 120), target: String(record.target || "").slice(0, 256),
        result: record.outcome_seq == null ? "outcome missing" : String(record.result || "outcome unspecified").slice(0, 256),
      });
      else c.operationOverflow = true;
    }
    function coverage(id = selected) {
      const c = id && caches.get(id);
      return { store: identity, cursor: cursor?.after_seq ?? null, durable: !!health?.durable, healthy: !!health?.healthy,
        loaded: !!c?.loaded, loading: !!c?.loading, error: c?.error || health?.error || null,
        retained: c?.rows.length || 0, omitted: c?.omitted || 0, scanned: c?.scanned || 0,
        pendingKnown: !!c?.pendingKnown, pendingCount: c?.pending.length || 0,
        uncertain: c?.uncertain.size || 0, uncertaintyTruncated: !!c?.operationOverflow, gap };
    }
    function notify(id) {
      if (id && caches.has(id)) call("onRows", id, caches.get(id).rows);
      call("onCoverage", selected, coverage());
    }
    function gate(id = selected) {
      const c = id && caches.get(id);
      const reason = !started ? "Connecting to durable event history…" : !health?.durable || !health?.healthy
        ? (health?.error || "Durable event storage is unavailable. Retry before sending.")
        : gap ? "Reconciling committed events…" : id && (!c?.loaded || c.loading)
          ? (c?.error || "Loading committed history…") : id && !c.pendingKnown
            ? "Live approval coverage is unavailable. Retry before sending." : id && c.pending.length
              ? "Answer the current approval before sending another message." : "";
      return { canSend: !reason, reason };
    }
    function reset(store) {
      for (const id of caches.keys()) call("onEvicted", id);
      caches.clear(); deleted.clear(); live.clear(); statusWatermarks.clear(); identity = { store_id: store.store_id, generation: store.generation };
      cursor = null; gap = true; epoch++;
    }
    function acceptStore(store) {
      if (!store || typeof store.store_id !== "string" || typeof store.generation !== "string") throw new Error("Missing event store identity");
      if (!identity) identity = { store_id: store.store_id, generation: store.generation };
      else if (!sameStore(identity, store)) throw new Error("Event store generation changed. Retry history.");
    }
    function acceptHealth(value) {
      acceptStore(value);
      if (typeof value.durable !== "boolean" || typeof value.healthy !== "boolean") throw new Error("Invalid event health report");
      health = value;
    }
    async function pending(id, capturedEpoch) {
      const c = caches.get(id); if (!c || deleted.has(id)) return;
      c.pendingKnown = false;
      try {
        const result = await invoke("get_pending_approvals", { id });
        if (capturedEpoch !== epoch || c !== caches.get(id)) return;
        if (!Array.isArray(result) || result.length > 128 || size(JSON.stringify(result)) > 512 * 1024 || result.some(r => !r.runtimeId || !Number.isSafeInteger(r.hostEpoch) || r.hostEpoch < 0 || !r.requestId || !Array.isArray(r.options) || r.options.length > 128))
          throw new Error("Invalid live approval coverage");
        c.pending = result; c.pendingKnown = true;
        call("onPending", id, result);
      } catch (error) {
        if (capturedEpoch === epoch && c === caches.get(id)) { c.pending = []; c.error = String(error?.message || error); call("onPending", id, []); }
      }
      notify(id);
    }
    async function snapshot(id) {
      if (!id || deleted.has(id)) return;
      const capturedEpoch = epoch, c = cache(id);
      gap = true;
      c.loading = true; c.loaded = false; c.pendingKnown = false; c.error = null; notify(id);
      const fresh = { ...c, rows: [], rowBySeq: new Map(), bytes: 0, omitted: 0, scanned: 0, uncertain: new Map(), operationOverflow: false, sessions: [] };
      let next = null, lease = null, watermark = null;
      try {
        for (let page = 0; page < LIMITS.pages; page++) {
          const response = await invoke("get_event_snapshot", { cursor: next, sessionId: id, limit: 256 });
          if (response.next) lease = response.next;
          if (capturedEpoch !== epoch || deleted.has(id)) return;
          acceptStore(response); seq(response.watermark);
          if (response.health) {
            if (!sameStore(response, response.health)) throw new Error("Snapshot health identity mismatch");
            acceptHealth(response.health);
          }
          if (response.session_id !== id || !Array.isArray(response.transcripts) || !Array.isArray(response.sessions)) throw new Error("Invalid event snapshot");
          if (watermark != null && watermark !== response.watermark) throw new Error("Snapshot watermark changed between pages");
          watermark = response.watermark;
          for (const row of response.transcripts) {
            if (row.session_id !== id) throw new Error("Snapshot returned another thread's transcript");
            put(fresh, { seq: row.seq, role: roleForKind(row.kind), body: row.payload, at: row.at });
          }
          for (const record of response.operations || []) operation(fresh, record);
          // Status baseline is applied only after the complete consistent snapshot.
          for (const session of response.sessions) if (session.id === id) fresh.sessions = [{ id: session.id, status: session.status }];
          next = response.next;
          if (!next) {
            if (response.transcripts_truncated || response.sessions_truncated || response.operations_truncated) throw new Error("Snapshot coverage is incomplete without a continuation");
            break;
          }
          if (!sameStore(response, next)) throw new Error("Invalid snapshot continuation identity");
          if (page === LIMITS.pages - 1) throw new Error("History exceeds the bounded snapshot pass. Coverage remains incomplete.");
        }
        if (capturedEpoch !== epoch) return;
        if (!fresh.sessions.length) throw new Error("Thread is absent from the committed snapshot. Refresh Threads before sending.");
        Object.assign(c, fresh, { watermark, loaded: true, loading: false, error: null });
        c.status = c.sessions[0]?.status || null;
        if (!cursor) cursor = { ...identity, after_seq: watermark };
        for (const session of c.sessions || []) statusWatermarks.set(session.id, watermark);
        call("onSessions", c.sessions || []);
        notify(id);
      } catch (error) {
        if (capturedEpoch === epoch && c === caches.get(id)) { c.loading = false; c.loaded = false; c.error = String(error?.message || error); notify(id); }
      } finally {
        if (lease) await invoke("release_event_snapshot", { cursor: lease }).catch(() => {});
      }
    }
    function apply(envelope) {
      const projection = envelope.projection || {};
      for (const id of projection.deleted_sessions || []) {
        deleted.add(id); caches.delete(id); call("onDeleted", id);
        // Older envelopes cannot replay behind the global cursor; the host
        // rejects effects for deleted sessions. Retain a bounded recent guard.
        if (deleted.size > 8192) deleted.delete(deleted.values().next().value);
      }
      for (const id of projection.baseline_changed_sessions || []) {
        const c = caches.get(id); if (c && envelope.seq > c.watermark) { c.loaded = false; c.pendingKnown = false; c.error = "Imported history changed. Retry to load its committed baseline."; }
      }
      const touched = new Set();
      for (const patch of projection.transcripts || []) {
        const c = caches.get(patch.session_id);
        if (c && !deleted.has(patch.session_id) && envelope.seq > c.watermark) { put(c, patch, !!patch.append); touched.add(patch.session_id); }
      }
      for (const patch of projection.statuses || []) {
        const c = caches.get(patch.session_id);
        if (!deleted.has(patch.session_id) && envelope.seq > (statusWatermarks.get(patch.session_id) || 0) && (!c || envelope.seq > c.watermark)) {
          if (c) c.status = patch.status;
          statusWatermarks.set(patch.session_id, envelope.seq); call("onStatus", patch);
          if (statusWatermarks.size > 8192) {
            const oldest = [...statusWatermarks.keys()].find(id => !caches.has(id));
            if (oldest) statusWatermarks.delete(oldest);
          }
        }
      }
      for (const record of projection.operations || []) {
        const c = caches.get(record.session_id);
        if (c && envelope.seq > c.watermark) { operation(c, record); touched.add(record.session_id); }
      }
      const historical = !live.delete(envelope.seq);
      for (const id of touched) notify(id);
      // Derived observers read the newly committed canonical row, never the
      // preceding token. Historical events do not recreate activity or grants.
      const observerSession = envelope.origin?.session_id || envelope.event?.session_id;
      const baseline = caches.get(observerSession)?.watermark ?? 0;
      if (!historical && !deleted.has(observerSession) && envelope.seq > baseline) call("onEvent", envelope.event, envelope);
    }
    async function replay() {
      if (!cursor) return;
      gap = true; call("onCoverage", selected, coverage());
      let target = null;
      try {
        for (let page = 0; page < LIMITS.pages; page++) {
          const response = await invoke("replay_events", { cursor, limit: 256 });
          acceptStore(response); seq(response.watermark);
          if (!Array.isArray(response.events) || !sameStore(response, response.next)) throw new Error("Invalid replay page");
          if (target == null) target = response.watermark;
          let previous = cursor.after_seq;
          for (const envelope of response.events) {
            acceptStore(envelope); seq(envelope.seq);
            if (envelope.seq > response.watermark) throw new Error("Replay event exceeds its committed watermark");
            if (envelope.seq <= previous) throw new Error("Replay is not strictly ordered");
            apply(envelope); previous = envelope.seq;
          }
          seq(response.next.after_seq);
          if (response.next.after_seq > response.watermark) throw new Error("Replay cursor exceeds its committed watermark");
          if (response.next.after_seq < previous || (response.has_more && response.next.after_seq <= cursor.after_seq)) throw new Error("Replay cursor did not advance");
          cursor = { ...response.next };
          if (!response.has_more || cursor.after_seq >= target) { gap = false; break; }
          if (page === LIMITS.pages - 1) throw new Error("Replay exceeds the bounded pass. Retry recovery.");
        }
        for (const n of live) if (n <= cursor.after_seq) live.delete(n);
        const c = selected && caches.get(selected);
        if (c && !c.loaded && !deleted.has(selected)) await snapshot(selected);
        if (selected && caches.get(selected)?.loaded) await pending(selected, epoch);
      } catch (error) {
        gap = true;
        if (selected) { const c = cache(selected); c.pendingKnown = false; c.error = String(error?.message || error); }
      }
      notify(selected);
    }
    function enqueue(task) {
      const result = chain.then(() => stopped ? undefined : task());
      chain = result.catch(error => { if (selected) { cache(selected).error = String(error); notify(selected); } });
      return result;
    }
    function scheduleReplay() {
      if (replayQueued || stopped || !started) return;
      replayQueued = true;
      enqueue(async () => { replayQueued = false; await replay(); });
    }
    async function start() {
      if (started) return;
      if (starting) return starting;
      starting = (async () => {
      try {
      // Await every subscription before opening the first snapshot transaction.
      for (const [name, handler] of [
        ["committed-event", envelope => {
          // A delayed notification may belong to a retired generation. Only a
          // fresh health query can authorize switching the renderer's store.
          if (identity && !sameStore(identity, envelope)) { gap = true; enqueue(retryHealth); return; }
          seq(envelope.seq);
          if (live.size >= LIMITS.notifications) { live.clear(); gap = true; for (const c of caches.values()) c.pendingKnown = false; }
          if (!cursor || envelope.seq > cursor.after_seq) live.add(envelope.seq);
          scheduleReplay();
        }],
        ["event-reconcile-required", () => { live.clear(); gap = true; for (const c of caches.values()) c.pendingKnown = false; scheduleReplay(); }],
        ["event-health", value => {
          if (identity && !sameStore(identity, value)) { gap = true; enqueue(retryHealth); return; }
          acceptHealth(value); notify(selected);
        }],
      ]) unlisteners.push(await listen(name, event => { try { handler(event.payload); } catch (error) { gap = true; if (selected) cache(selected).error = String(error); notify(selected); } }));
      acceptHealth(await invoke("event_health")); started = true; gap = false; notify(selected);
      } catch (error) { for (const off of unlisteners.splice(0)) off(); throw error; }
      })();
      try { await starting; } finally { starting = null; }
    }
    async function select(id, { force = false } = {}) {
      selected = id || null;
      if (!id) { notify(null); return; }
      return enqueue(async () => {
        if (!started) await start();
        if (!cache(id).loaded || force) await snapshot(id);
        await replay();
        notify(id);
      });
    }
    async function retry() {
      return enqueue(async () => {
        if (!started) await start();
        await retryHealth();
        if (selected) await snapshot(selected);
        await replay(); notify(selected);
      });
    }
    async function retryHealth() {
      const fresh = await invoke("event_health");
      const changed = identity && !sameStore(identity, fresh);
      if (changed) reset(fresh);
      acceptHealth(fresh);
      if (changed && selected) await snapshot(selected);
      if (changed) await replay();
      else if (gap && cursor) await replay();
      notify(selected);
    }
    function stop() { stopped = true; for (const off of unlisteners.splice(0)) off(); }
    return { start, select, retry, reconcile: () => enqueue(replay), coverage, gate, stop,
      pending: id => caches.get(id)?.pending || [],
      status: id => caches.get(id)?.status || null,
      isDeleted: id => deleted.has(id),
      uncertain: id => [...(caches.get(id)?.uncertain.values() || [])].slice(-4),
      rows: id => caches.get(id)?.rows || [],
      debug: () => ({ caches: caches.size, buffered: live.size, bytes: [...caches.values()].reduce((sum, c) => sum + c.bytes, 0) }) };
  }
  global.BombDurableEvents = Object.freeze({ createOwner, LIMITS });
})(typeof window === "undefined" ? globalThis : window);
