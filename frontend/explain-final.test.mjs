import { test } from "node:test";
import assert from "node:assert/strict";

/** Mirror of pushFinalExplainFromReply text shaping in app.js */
function finalExplainText(body) {
  const plain = String(body || "")
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`\n]+)`/g, "$1")
    .replace(/\*\*([^*\n]+)\*\*/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
  const clip = plain.length > 240 ? `${plain.slice(0, 237)}…` : plain;
  return clip
    ? `The agent finished and replied: ${clip}`
    : "The agent finished its reply.";
}

test("final What's happening summarizes reply and strips fences", () => {
  const body =
    "The `add` function currently returns **a - b**.\n\n```python\ndef add(a, b):\n    return a - b\n```\nSo add(5, 3) returns `2`.";
  const text = finalExplainText(body);
  assert.match(text, /^The agent finished and replied:/);
  assert.match(text, /a - b/);
  assert.doesNotMatch(text, /```/);
  assert.doesNotMatch(text, /is writing/i);
});

test("empty body still clears writing state with finished line", () => {
  assert.equal(finalExplainText(""), "The agent finished its reply.");
});
