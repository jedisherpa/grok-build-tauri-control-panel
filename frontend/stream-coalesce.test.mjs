
import { test } from "node:test";
import assert from "node:assert/strict";

/** Mirror of appendTranscript coalesce skip rule. */
function findStreamTarget(list, role, hopsMax = 40) {
  for (let i = list.length - 1, hops = 0; i >= 0 && hops < hopsMax; i--, hops++) {
    const entry = list[i];
    if (entry.role === role && entry.streaming) return i;
    if (entry.role !== role) continue;
    break;
  }
  return -1;
}

test("agent stream coalesces across interleaved thought/tool/term", () => {
  const list = [
    { role: "agent", body: "Hello ", streaming: true },
    { role: "thought", body: "hmm", streaming: false },
    { role: "tool", body: "read", streaming: false },
    { role: "term", body: "noise", streaming: false },
  ];
  assert.equal(findStreamTarget(list, "agent"), 0);
});

test("non-streaming same role does not coalesce", () => {
  const list = [
    { role: "agent", body: "done", streaming: false },
    { role: "term", body: "x", streaming: false },
  ];
  assert.equal(findStreamTarget(list, "agent"), -1);
});
