import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "fs";
import { createContext, runInContext } from "vm";
import { fileURLToPath } from "url";
import { dirname, join } from "path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const sandbox = { window: {} };
runInContext(readFileSync(join(__dirname, "diagnostics.js"), "utf8"), createContext(sandbox));
const D = sandbox.window.BombDiagnostics;

// Verbatim shape of the play1 "What's happening" / Open face text (screens 12, 20).
const PLAY1 =
  "\x1b[2m2026-10-07T02:21:45.954046Z\x1b[0m \x1b[33m WARN\x1b[0m Failed to fetch models: 403 - " +
  '{"code":"permission-denied","error":"The API key xai-...KwZn is disabled and cannot be used to perform requests. ' +
  'To enable the API key, go to https://console.x.ai/team/2c0a58ed-703f-434e-9191-a3016d3bc641/api-keys."}';

test("strips ANSI colour codes", () => {
  const s = D.stripAnsi(PLAY1);
  assert.ok(!s.includes("\x1b"));
  assert.ok(!s.includes("[2m") && !s.includes("[0m"));
});

test("redacts xAI team id and key fragment", () => {
  const s = D.sanitize(PLAY1);
  assert.ok(!s.includes("2c0a58ed-703f-434e-9191-a3016d3bc641"));
  assert.ok(!s.includes("KwZn"));
  assert.ok(s.includes("/team/[team]/api-keys"));
  assert.equal(D.redactSecrets("Bearer abcdefghijk xai-AAAAAAAAAAAA sk-BBBBBBBBBBBB"), "Bearer [redacted] xai-[redacted] sk-[redacted]");
});

test("composer gate blocks starting and failed threads with a visible reason", () => {
  assert.deepEqual({ ...D.composerGate(null, "") }, { canSend: true, reason: "" });
  assert.equal(D.composerGate("a", "idle").canSend, true);
  const starting = D.composerGate("a", "Starting");
  assert.equal(starting.canSend, false);
  assert.match(starting.reason, /still starting/);
  const failed = D.composerGate("a", "failed");
  assert.equal(failed.canSend, false);
  assert.match(failed.reason, /failed earlier|failed to start/);
});
