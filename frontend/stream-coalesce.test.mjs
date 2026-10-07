import { test } from "node:test";
import assert from "node:assert/strict";

/** Mirror of STREAM_SOFT_ROLES + findStreamCoalesceIndex in app.js */
const STREAM_SOFT_ROLES = {
  agent: new Set(["term", "system", "thought"]),
  thought: new Set(["term", "system"]),
  term: new Set(["system"]),
};

function findStreamCoalesceIndex(list, role, hopsMax = 40) {
  if (!list || !list.length) return -1;
  const soft = STREAM_SOFT_ROLES[role] || new Set(["term", "system"]);
  for (let i = list.length - 1, hops = 0; i >= 0 && hops < hopsMax; i--, hops++) {
    const entry = list[i];
    if (entry.role === role) {
      if (entry.streaming) return i;
      if (role === "agent" || role === "thought") return i;
      return -1;
    }
    if (soft.has(entry.role)) continue;
    return -1;
  }
  return -1;
}

/** Simulate appendTranscript coalesce+reopen for agent stream chunks. */
function applyAgentChunk(list, text, { stream = true } = {}) {
  if (stream) {
    const i = findStreamCoalesceIndex(list, "agent");
    if (i >= 0) {
      const entry = list[i];
      entry.body = (entry.body || "") + text;
      entry.streaming = true;
      return list;
    }
  }
  list.push({ role: "agent", body: text, streaming: !!stream });
  return list;
}

test("agent stream coalesces across interleaved thought/tool/term soft noise while live", () => {
  const list = [
    { role: "agent", body: "Hello ", streaming: true },
    { role: "thought", body: "hmm", streaming: false },
    { role: "term", body: "noise", streaming: false },
  ];
  // tool is a hard separator — but while still streaming and tool not yet
  // endAgentStream'd into a closed bubble, soft roles still join.
  assert.equal(findStreamCoalesceIndex(list, "agent"), 0);
});

test("tool hard-stops: closed agent before tool does not coalesce", () => {
  const list = [
    { role: "agent", body: "part1", streaming: false },
    { role: "tool", body: "fs/read", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "agent"), -1);
});

test("late tiny chunk after prompt_finished joins the same bubble (round4 S5)", () => {
  // Main body streamed, prompt_finished closed streaming, term row appended,
  // then a trailing fragment "2`." arrives — must reopen, not new AGENT row.
  const list = [
    { role: "agent", body: "So for example, add(5, 3) returns `", streaming: false },
    { role: "term", body: "prompt response ended · end_turn", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "agent"), 0);
  applyAgentChunk(list, "2`.");
  assert.equal(list.length, 2, "must not open a third transcript row");
  assert.equal(list[0].role, "agent");
  assert.equal(list[0].body, "So for example, add(5, 3) returns `2`.");
  assert.equal(list[0].streaming, true);
});

test("approval hard-stops a closed agent bubble", () => {
  const list = [
    { role: "agent", body: "before", streaming: false },
    { role: "approval", body: "allow?", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "agent"), -1);
});

test("user message hard-stops so the next turn starts fresh", () => {
  const list = [
    { role: "agent", body: "old reply", streaming: false },
    { role: "user", body: "next question", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "agent"), -1);
  applyAgentChunk(list, "new reply");
  assert.equal(list.length, 3);
  assert.equal(list[2].body, "new reply");
});

test("non-streaming term does not reopen via agent rules", () => {
  const list = [
    { role: "term", body: "done", streaming: false },
    { role: "system", body: "x", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "term"), -1);
});
