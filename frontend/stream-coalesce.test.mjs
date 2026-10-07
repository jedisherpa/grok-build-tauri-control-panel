import { test } from "node:test";
import assert from "node:assert/strict";

/** Mirror of STREAM_SOFT_ROLES + MD bridge + findStreamCoalesceIndex in app.js */
const STREAM_SOFT_ROLES = {
  agent: new Set(["term", "system", "thought"]),
  thought: new Set(["term", "system"]),
  term: new Set(["system"]),
};
const STREAM_MD_BRIDGE_ROLES = new Set(["tool", "plan"]);

function incompleteMarkdownTail(body) {
  const t = String(body || "");
  if (!t) return false;
  const fences = (t.match(/```/g) || []).length;
  if (fences % 2 === 1) return true;
  const bolds = (t.match(/\*\*/g) || []).length;
  if (bolds % 2 === 1) return true;
  const noFences = t.replace(/```[\s\S]*?```/g, "");
  const ticks = (noFences.match(/`/g) || []).length;
  if (ticks % 2 === 1) return true;
  return false;
}

function findStreamCoalesceIndex(list, role, hopsMax = 40) {
  if (!list || !list.length) return -1;
  const soft = STREAM_SOFT_ROLES[role] || new Set(["term", "system"]);
  let bridged = false;
  for (let i = list.length - 1, hops = 0; i >= 0 && hops < hopsMax; i--, hops++) {
    const entry = list[i];
    if (entry.role === role) {
      if (entry.streaming) return i;
      if (role === "agent" || role === "thought") {
        if (!bridged) return i;
        if (role === "agent" && incompleteMarkdownTail(entry.body)) return i;
        return -1;
      }
      return -1;
    }
    if (soft.has(entry.role)) continue;
    if (role === "agent" && STREAM_MD_BRIDGE_ROLES.has(entry.role)) {
      bridged = true;
      continue;
    }
    return -1;
  }
  return -1;
}

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
  assert.equal(findStreamCoalesceIndex(list, "agent"), 0);
});

test("tool hard-stops: closed agent before tool does not coalesce when markdown complete", () => {
  const list = [
    { role: "agent", body: "part1 done.", streaming: false },
    { role: "tool", body: "fs/read", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "agent"), -1);
});

test("late tiny chunk after prompt_finished joins the same bubble (round4 S5)", () => {
  const list = [
    { role: "agent", body: "So for example, add(5, 3) returns `", streaming: false },
    { role: "term", body: "prompt response ended · end_turn", streaming: false },
  ];
  assert.equal(findStreamCoalesceIndex(list, "agent"), 0);
  applyAgentChunk(list, "2`.");
  assert.equal(list.length, 2, "must not open a third transcript row");
  assert.equal(list[0].body, "So for example, add(5, 3) returns `2`.");
  assert.equal(list[0].streaming, true);
});

test("bold markers do not break the bubble across a tool row (round4b)", () => {
  // Play A/B: "It returns **" then tool/noise then "a - b** …"
  const list = [
    { role: "agent", body: "It returns **", streaming: false },
    { role: "tool", body: "fs/read calc.py", streaming: false },
  ];
  assert.equal(incompleteMarkdownTail("It returns **"), true);
  assert.equal(findStreamCoalesceIndex(list, "agent"), 0);
  applyAgentChunk(list, "a - b** (the difference of the two arguments), not their sum.");
  assert.equal(list.length, 2);
  assert.equal(
    list[0].body,
    "It returns **a - b** (the difference of the two arguments), not their sum."
  );
});

test("complete markdown before tool still starts a new segment", () => {
  const list = [
    { role: "agent", body: "First thought **done**.", streaming: false },
    { role: "tool", body: "fs/read", streaming: false },
  ];
  assert.equal(incompleteMarkdownTail("First thought **done**."), false);
  assert.equal(findStreamCoalesceIndex(list, "agent"), -1);
  applyAgentChunk(list, "After the tool…");
  assert.equal(list.length, 3);
  assert.equal(list[2].body, "After the tool…");
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
