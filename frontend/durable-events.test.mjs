import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createContext, runInContext } from "node:vm";

const source = readFileSync(new URL("./durable-events.js", import.meta.url), "utf8");
const identity = { store_id: "store", generation: "generation" };
const healthy = { ...identity, durable: true, healthy: true, last_seq: 10, error: null };
const row = (n, id = "a", body = `row ${n}`) => ({ session_id: id, seq: n, kind: "agent", payload: body, at: "now" });
const envelope = (n, projection = {}) => ({ ...identity, seq: n, origin: { session_id: "a", runtime_id: "runtime" }, event: { type: "agent_message", session_id: "a", text: "B" }, projection });
function fixture({ rows = [row(1)], watermark = 10, events = [], override } = {}) {
  const listeners = new Map(), calls = [], presented = [], deleted = [], statuses = [];
  let health = { ...healthy };
  const context = createContext({ TextEncoder }); runInContext(source, context);
  const owner = context.BombDurableEvents.createOwner({
    listen: async (name, handler) => { listeners.set(name, handler); return () => listeners.delete(name); },
    invoke: async (name, args) => {
      calls.push({ name, args });
      if (override) { const result = await override(name, args, listeners); if (result !== undefined) return result; }
      if (name === "event_health") return health;
      if (name === "get_pending_approvals") return [];
      if (name === "release_event_snapshot") return;
      if (name === "get_event_snapshot") {
        assert.equal(listeners.size, 3, "all listeners precede snapshot");
        const after = args.cursor?.transcript_after || 0;
        const items = rows.filter(r => r.session_id === args.sessionId && r.seq > after).slice(0, args.limit);
        const more = rows.some(r => r.session_id === args.sessionId && r.seq > (items.at(-1)?.seq || after));
        return { ...identity, health, watermark, session_id: args.sessionId,
          sessions: [{ id: args.sessionId, status: "idle" }], transcripts: items, operations: [],
          sessions_truncated: false, transcripts_truncated: more, operations_truncated: false,
          next: more ? { ...identity, snapshot_id: "lease", transcript_after: items.at(-1).seq } : null };
      }
      if (name === "replay_events") {
        const items = events.filter(e => e.seq > args.cursor.after_seq).slice(0, args.limit);
        const after = items.at(-1)?.seq ?? args.cursor.after_seq;
        const end = Math.max(watermark, events.at(-1)?.seq || 0);
        return { ...identity, watermark: end, events: items, next: { ...identity, after_seq: after }, has_more: after < end };
      }
      throw new Error(`Unexpected command ${name}`);
    },
    onEvent: e => presented.push(e), onDeleted: id => deleted.push(id), onStatus: p => statuses.push(p),
  });
  return { owner, listeners, calls, presented, deleted, statuses, setHealth: value => { health = value; },
    emit: (name, payload) => listeners.get(name)?.({ payload }) };
}
function deferred() { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; }

test("subscribe before consistent snapshot; notification during hydrate replays once with canonical append", async () => {
  const events = [envelope(11, { transcripts: [{ session_id: "a", seq: 1, role: "agent", body: "B", at: "later", append: true }] })];
  let fired = false;
  const h = fixture({ rows: [row(1, "a", "A")], events, override: (name, args, listeners) => {
    if (name === "get_event_snapshot" && !fired) { fired = true; listeners.get("committed-event")({ payload: events[0] }); listeners.get("committed-event")({ payload: events[0] }); }
  } });
  await h.owner.select("a"); await h.owner.reconcile();
  assert.equal(h.owner.rows("a")[0].body, "AB");
  assert.equal(h.presented.length, 1); assert.equal(h.owner.coverage("a").cursor, 11);
  assert.equal(h.owner.gate("a").canSend, true);
});

test("50,000-row paged snapshot retains only bounded tail and reports omitted coverage", async () => {
  const h = fixture({ rows: Array.from({ length: 50000 }, (_, i) => row(i + 1)), watermark: 50000 });
  await h.owner.select("a");
  assert.equal(h.owner.rows("a").length, 2000);
  assert.equal(h.owner.rows("a")[0].seq, 48001);
  assert.equal(h.owner.coverage("a").omitted, 48000);
  assert.equal(h.owner.coverage("a").scanned, 50000);
  assert.ok(h.owner.debug().bytes <= 4 * 1024 * 1024);
  assert.equal(h.calls.filter(c => c.name === "release_event_snapshot").length, 1);
});

test("byte budget bounds retained text independently of row count", async () => {
  const h = fixture({ rows: Array.from({ length: 150 }, (_, i) => row(i + 1, "a", "☃".repeat(21000))), watermark: 150 });
  await h.owner.select("a");
  assert.ok(h.owner.rows("a").length < 150); assert.ok(h.owner.debug().bytes <= 4 * 1024 * 1024);
  assert.ok(h.owner.coverage("a").omitted > 0);
});

test("failed history load remains retryable and never advertises loaded coverage", async () => {
  let fail = true;
  const h = fixture({ override: name => { if (name === "get_event_snapshot" && fail) throw new Error("read failed"); } });
  await h.owner.select("a");
  assert.equal(h.owner.coverage("a").loaded, false); assert.equal(h.owner.gate("a").canSend, false);
  fail = false; await h.owner.retry();
  assert.equal(h.owner.coverage("a").loaded, true); assert.equal(h.owner.gate("a").canSend, true);
});

test("snapshot lease is released when a later page fails", async () => {
  const h = fixture({ rows: Array.from({ length: 257 }, (_, i) => row(i + 1)), watermark: 257,
    override: (name, args) => { if (name === "get_event_snapshot" && args.cursor) throw new Error("expired"); } });
  await h.owner.select("a");
  assert.equal(h.owner.coverage("a").loaded, false);
  assert.equal(h.calls.filter(c => c.name === "release_event_snapshot").length, 1);
});

test("unknown pending coverage blocks Send; exact live query recovers it", async () => {
  let fail = true;
  const h = fixture({ override: name => { if (name === "get_pending_approvals" && fail) throw new Error("transport missing"); } });
  await h.owner.select("a");
  assert.equal(h.owner.coverage("a").pendingKnown, false); assert.equal(h.owner.gate("a").canSend, false);
  fail = false; await h.owner.retry(); assert.equal(h.owner.gate("a").canSend, true);
});

test("historical approvals never become presentation events or live grants", async () => {
  const approval = envelope(11); approval.event = { type: "approval_required", session_id: "a", request_id: "old" };
  const h = fixture({ events: [approval] }); await h.owner.select("a");
  assert.equal(h.presented.length, 0); assert.equal(h.owner.pending("a").length, 0);
});

test("tombstone drops cached rows and ignores late patches for the deleted session", async () => {
  const h = fixture({ events: [envelope(11, { deleted_sessions: ["a"] }), envelope(12, { transcripts: [{ session_id: "a", seq: 2, body: "late", role: "agent", append: false }] })] });
  await h.owner.select("a");
  assert.equal(h.owner.rows("a").length, 0); assert.deepEqual(h.deleted, ["a"]);
  await h.owner.select("a", { force: true }); assert.equal(h.owner.rows("a").length, 0);
});

test("store mismatch cannot switch identity using a stale notification", async () => {
  const h = fixture(); await h.owner.select("a");
  h.emit("committed-event", { ...envelope(11), generation: "retired" }); await h.owner.reconcile();
  assert.equal(h.owner.coverage("a").store.generation, "generation");
});

test("volatile snapshot health blocks send despite an earlier healthy report", async () => {
  const h = fixture({ override: (name, args) => name === "get_event_snapshot" ? { ...identity,
    health: { ...healthy, durable: false }, watermark: 10, session_id: args.sessionId,
    sessions: [], transcripts: [], operations: [], next: null } : undefined });
  await h.owner.select("a"); assert.equal(h.owner.gate("a").canSend, false);
});

test("thread switching during paged load retains correct cache and selected coverage", async () => {
  const wait = deferred(); let first = true;
  const h = fixture({ rows: [row(1), row(2, "b")], override: async name => {
    if (name === "get_event_snapshot" && first) { first = false; await wait.promise; }
  } });
  const a = h.owner.select("a"); await new Promise(r => setImmediate(r));
  const b = h.owner.select("b"); wait.resolve(); await a; await b;
  assert.equal(h.owner.rows("a")[0].body, "row 1"); assert.equal(h.owner.rows("b")[0].body, "row 2");
  assert.equal(h.owner.coverage().pendingKnown, true);
});

test("malformed or nonadvancing replay is visible and blocks Send", async () => {
  const h = fixture({ override: name => name === "replay_events" ? { ...identity, watermark: 11, events: [], next: { ...identity, after_seq: 10 }, has_more: true } : undefined });
  await h.owner.select("a"); assert.equal(h.owner.gate("a").canSend, false);
  assert.match(h.owner.coverage("a").error, /advance/);
});

test("cache count stays bounded across many selected sessions", async () => {
  const h = fixture({ rows: [] });
  for (let n = 0; n < 20; n++) await h.owner.select(`thread-${n}`);
  assert.equal(h.owner.debug().caches, 8);
});

test("unresolved typed intent remains uncertain until its exact outcome arrives", async () => {
  const record = { operation_id: "op", session_id: "a", intent_seq: 11, outcome_seq: null };
  const events = [envelope(11, { operations: [record] })];
  const h = fixture({ events }); await h.owner.select("a");
  assert.equal(h.owner.coverage("a").uncertain, 1);
  events.push(envelope(12, { operations: [{ ...record, outcome_seq: 12, result: "observed_success" }] }));
  await h.owner.reconcile(); assert.equal(h.owner.coverage("a").uncertain, 0);
});

test("storage kinds restore the same user, agent and tool roles as committed deltas", async () => {
  const rows = [row(1), row(2), row(3)];
  rows[0].kind = "prompt"; rows[1].kind = "assistant"; rows[2].kind = "tool_call";
  const h = fixture({ rows }); await h.owner.select("a");
  assert.deepEqual(Array.from(h.owner.rows("a"), r => r.role), ["user", "agent", "tool"]);
});

test("notification overflow retains bounded metadata and recovers from journal instead of inferring grants", async () => {
  const events = Array.from({ length: 700 }, (_, n) => envelope(n + 11));
  let once = false, observedBuffered = 0;
  const h = fixture({ events, override: (name, args, listeners) => {
    if (name === "get_event_snapshot" && !once) {
      once = true;
      for (const event of events) listeners.get("committed-event")({ payload: event });
      observedBuffered = h.owner.debug().buffered;
    }
  } });
  await h.owner.select("a"); await h.owner.reconcile();
  assert.ok(observedBuffered <= 512); assert.equal(h.owner.coverage("a").cursor, 710);
  assert.equal(h.owner.coverage("a").pendingKnown, true);
  assert.equal(h.owner.debug().buffered, 0);
});

test("an imported baseline invalidates the cached rows and loads its committed replacement", async () => {
  let imported = false;
  const event = envelope(11, { baseline_changed_sessions: ["a"] });
  const h = fixture({ events: [event], override: name => {
    if (name === "replay_events") imported = true;
    if (name === "get_event_snapshot" && imported) return { ...identity, health: healthy, watermark: 11, session_id: "a",
      sessions: [{ id: "a", status: "idle" }], transcripts: [row(1), row(2, "a", "imported original")], operations: [], next: null };
  } });
  await h.owner.select("a"); assert.equal(h.owner.rows("a")[1].body, "imported original");
  assert.equal(h.owner.coverage("a").loaded, true);
});

test("a fresh generation health check invalidates old cursor and cached transcript", async () => {
  let current = identity;
  const h = fixture({ override: (name, args) => {
    if (current === identity) return;
    if (name === "get_event_snapshot") return { ...current, health: { ...healthy, ...current }, watermark: 20, session_id: args.sessionId,
      sessions: [{ id: "a", status: "idle" }], transcripts: [row(2, "a", "new generation")], operations: [], next: null };
    if (name === "replay_events") return { ...current, watermark: 20, events: [], next: { ...current, after_seq: 20 }, has_more: false };
  } });
  await h.owner.select("a"); current = { ...identity, generation: "new-generation" };
  h.setHealth({ ...healthy, ...current }); h.emit("event-health", { ...healthy, ...current });
  await h.owner.reconcile();
  assert.equal(h.owner.coverage("a").store.generation, "new-generation");
  assert.equal(h.owner.rows("a")[0].body, "new generation");
});

test("snapshot-loaded status cannot be regressed by an older replay delta", async () => {
  const events = [envelope(11, { statuses: [{ session_id: "a", status: "running" }] })];
  const h = fixture({ watermark: 20, events }); await h.owner.select("a");
  assert.equal(h.owner.status("a"), "idle"); assert.equal(h.statuses.length, 0);
});

test("malformed live approval epoch leaves controls unavailable", async () => {
  const h = fixture({ override: name => name === "get_pending_approvals" ? [{ runtimeId: "runtime", requestId: "request", hostEpoch: -1, options: [] }] : undefined });
  await h.owner.select("a"); assert.equal(h.owner.pending("a").length, 0);
  assert.equal(h.owner.coverage("a").pendingKnown, false); assert.equal(h.owner.gate("a").canSend, false);
});

test("partial listener setup is cleaned up and can be retried", async () => {
  const context = createContext({ TextEncoder }); runInContext(source, context);
  let fail = true, active = 0;
  const owner = context.BombDurableEvents.createOwner({ listen: async name => {
    if (name === "event-health" && fail) throw new Error("listen failed");
    active++; return () => active--;
  }, invoke: async () => healthy });
  await assert.rejects(owner.start(), /listen failed/); assert.equal(active, 0);
  fail = false; await owner.start(); assert.equal(active, 3); owner.stop(); assert.equal(active, 0);
});

test("a later selected baseline does not skip another cached thread's intervening events", async () => {
  const events = [];
  const h = fixture({ rows: [row(1), row(1, "b")], events, override: (name, args) => {
    if (name === "get_event_snapshot" && args.sessionId === "b") return { ...identity, health: healthy, watermark: 20, session_id: "b",
      sessions: [{ id: "b", status: "completed" }], transcripts: [row(1, "b", "B baseline at twenty")], operations: [], next: null };
  } });
  await h.owner.select("a");
  events.push(envelope(15, { statuses: [{ session_id: "b", status: "running" }] }),
    envelope(16, { transcripts: [{ session_id: "a", seq: 1, role: "agent", body: " A later", at: "now", append: true }] }), envelope(20));
  await h.owner.select("b");
  assert.equal(h.owner.status("b"), "completed");
  assert.equal(h.owner.rows("a")[0].body, "row 1 A later");
  assert.equal(h.owner.coverage("b").cursor, 20);
});

test("a thread absent from the snapshot remains unknown instead of admitting a ghost session", async () => {
  const h = fixture({ override: (name, args) => name === "get_event_snapshot" ? { ...identity, watermark: 10, session_id: args.sessionId,
    sessions: [], transcripts: [], operations: [], next: null } : undefined });
  await h.owner.select("a"); assert.equal(h.owner.coverage("a").loaded, false);
  assert.equal(h.owner.gate("a").canSend, false); assert.match(h.owner.coverage("a").error, /absent/);
});

test("observed uncertain outcomes and dispatched grants retain visible external-effect uncertainty", async () => {
  for (const result of ["uncertain_stopped", "uncertain_dispatch", "failed_or_uncertain: partial write", "dispatched", "unknown_future_result"]) {
    const h = fixture({ events: [envelope(11, { operations: [{ operation_id: "op", session_id: "a", intent_seq: 10, outcome_seq: 11, result, kind: "permission_response", target: "native tool" }] })] });
    await h.owner.select("a");
    assert.equal(h.owner.coverage("a").uncertain, 1, result);
    assert.equal(h.owner.uncertain("a")[0].result, result);
  }
});

test("live control query accepts the same 128-card/128-option/512KiB bounds as ACP", async () => {
  const cards = Array.from({ length: 128 }, (_, n) => ({ runtimeId: "runtime", hostEpoch: 1, requestId: `request-${n}`, options: [] }));
  cards[0].options = Array.from({ length: 128 }, (_, n) => ({ id: `option-${n}`, label: "Allow", kind: "allow_once" }));
  const h = fixture({ override: name => name === "get_pending_approvals" ? cards : undefined });
  await h.owner.select("a"); assert.equal(h.owner.pending("a").length, 128);
  assert.equal(h.owner.coverage("a").pendingKnown, true);
  assert.equal(h.owner.gate("a").canSend, false, "active controls still block another prompt");
});

test("a delayed live finish behind another thread's newer snapshot cannot regress its presence", async () => {
  const events = [], oldFinished = { ...envelope(50), origin: { session_id: "b", runtime_id: "old-runtime" },
    event: { type: "prompt_finished", session_id: "b", stop_reason: "end_turn" } };
  const h = fixture({ events, override: (name, args, listeners) => {
    if (name === "get_event_snapshot" && args.sessionId === "b") {
      listeners.get("committed-event")({ payload: oldFinished });
      return { ...identity, health: healthy, watermark: 100, session_id: "b",
        sessions: [{ id: "b", status: "running" }], transcripts: [row(1, "b", "newer runtime body")], operations: [], next: null };
    }
  } });
  await h.owner.select("a"); events.push(oldFinished, envelope(100));
  await h.owner.select("b"); await h.owner.reconcile();
  assert.equal(h.presented.length, 0); assert.equal(h.owner.status("b"), "running");
  assert.equal(h.owner.coverage("b").cursor, 100);
});
