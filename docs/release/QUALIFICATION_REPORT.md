# See Cubed qualification status

2026-10-07. Baseline GitHub main baa890860d7f6b703757e62f9b58b33a7fe82648. Local lane codex/c3-release-qualification. **Not production-qualified and not notarized.**

Paul added a rigorous current-stack audit to the release work. Earlier GPUI product-deconstruction/unified-architecture artifacts were read and reconciled with the actual current Tauri stack; the renderer migration remains outside this lane.

## Deliverables

- USER_STORIES.md: 44 stable user-story IDs, observable acceptance/failure cases and explicit required evidence levels.
- STORY_REVISIONS.md: expectation corrections and defects; original failures retained privately.
- STACK_ARCHITECTURE_AUDIT.md: independent architecture/authority/persistence audit and generated adverse-case reproductions.
- STACK_SIMPLICITY_AUDIT.md: independent dependency, resource, rendering and duplication audit.
- REMEDIATION_BACKLOG.md: protected unique features, staged Tauri improvements and story-linked acceptance gates.
- MACOS_DISTRIBUTION.md and scripts/release_preflight.py: Apple/Tauri preparation, signing/ticket/Gatekeeper checks, source and bundle receipts, safe signature-preserving developer install.

## Current source repairs

Current-user frozen semantic/Python paths replace Paul-specific constants; Node resolves from absolute PATH entries rather than a particular NVM version. UI cannot choose runtime/reference paths. Frozen reference and Python packages still require separately qualified installation; this does not ship them automatically.

C3_PROFILE_DIR isolates panel/config/session/worktree/notes/MCP credentials. MCP vendor mirroring is disabled and Haven config uses the profile home. External CLI state, skill/plugin CLI operations and read-only frozen resources are not automatically isolated; native destructive tests must use fake adapters or further verified isolation. No new native QA profile was launched in this phase.

Startup validates base and project configuration before saving discovery; malformed/unreadable existing config fails rather than becoming defaults. Regression preserves exact corrupt base and valid base with corrupt overlay. Native error UX remains untested.

ACP terminal/file clipping respects UTF-8 byte boundaries. The regression covers split multibyte boundaries at both original budgets. Full-file allocation and terminal/resource admission caps remain open.

Note mutations retain an exclusive lock through staged publication, use unique owner-only temporary files and file sync, and expose changed in-memory state only after successful rename. Regressions cover concurrent additions/reopen exact IDs, failed add publication and corrupt-original/prior-backup preservation. This is one-process correctness, not cross-process ownership or a claim of power-loss-safe directory metadata.

Punctuation-only literal history queries return zero; only empty query browses all. The pre-existing failing test passes unchanged after repair. Developer installation now refuses existing destinations, retains the built signature, rejects implicit notarization inputs and neither deletes the installed app nor automatically launches it.

## Previous preparation checkpoint verification

| Check | Observed result | Evidence level |
|---|---|---|
| Rust workspace tests, locked/offline | 230 passed, 0 failed, 6 ignored | Unit/integration fixtures, not all native stories |
| Python discovery across scripts | 136 passed, 0 failures | Includes release preflight failure tests and installed-reference local tests |
| Frontend Node suite | 191 passed, 0 failed/skipped | Pure/mocked frontend checks |
| Workspace strict all-target Clippy | Passed | Compile/static gate |
| Shell installer syntax / diff whitespace | Passed | Static gate |
| Architecture generated reproductions | 9 adverse-case assertions passed; separate concurrent-note failure receipt retained | Confirms baseline defects; not repaired-feature acceptance |
| Installed existing app QA preflight | Passed | Packaging inspection of existing app, not new candidate/source parity |
| Installed existing app Developer ID candidate preflight | Rejected as expected | No Developer ID/team/timestamp/runtime; local source dirty |

Rust failures during implementation (format placeholder and duplicate Clone derive) were repaired and superseded by the full final rerun; their logs remain retained. Initial Python history-search failure and attempted offline all-platform metadata failure are retained. macOS dependency tree and workspace/no-deps metadata succeeded independently; lockfile inventory is parsed separately rather than substituting an empty failed metadata result.

## Open release gates

The authority remediation below addresses host policy parity/direct ACP authorization and managed repository reservations/isolation fallback. The process checkpoint below adds observed scheduler/headless outcomes and retained cleanup ownership. Whole native adapter enforcement, packaged scheduler/process recovery, durable event ingestion, diff correctness and aggregate worker/resource caps remain release gates. Review the audit reports and staged backlog; source-suite passes do not override unrun packaged stories.

The cached baseline Rust SQLite amalgamation is 3.46.0; Persistence uses WAL/NORMAL. It needs a verified patched runtime and checkpoint/ownership/durability qualification. The system Python SQLite reports 3.43.2, and Python/runtime/reference installation and licenses also need a separate distribution check. No installed app SQLite query or corruption incident is asserted here. Current official SQLite guidance fixes its rare multi-connection WAL-reset race in 3.51.3+, with 3.44.6/3.50.7 backports: https://www.sqlite.org/wal.html#walresetbug.

Packaged final-candidate stories, actual provider workflows, deliberate recovery tests, clean-account/prerequisite install, accessibility, matched startup/scroll/frame/RSS measurements, signing/hardened-runtime retest, upgrade/rollback and downloaded-asset checks remain unrun for this new source. The existing installed app was observed read-only; historical native results do not qualify this candidate.

The initial sandboxed code-signing query returned zero identities. A deeper 2026-10-07 check outside the sandbox corrected that result: this Mac has a valid Developer ID Application identity for Paul Cooper, team X8BVJAF8W5, with certificate expiry 2031-03-15. Xcode lists the paid individual team. A disposable executable was successfully signed with that identity, hardened runtime and Apple's secure timestamp, then passed strict signature verification. No private key was exported or credential changed.

The existing iCloud Keychain profile `fisheye-research-feed-notary` successfully authenticated with Apple's read-only notarization history API, returning 100 accepted submissions. The older `prismai-notary` profile returned HTTP 401 and was left unchanged. A previous PrismAI app's stapled ticket also validated outside the sandbox. Existing Apple setup is therefore available; new enrollment, certificate import or profile creation is unnecessary. Production architecture/destination and the product qualification gates above remain unresolved. No C3 signing/notarization submission, production publication, application replacement or private history/provider transmission occurred in this phase. The private discovery receipt is stored outside Git with the other qualification evidence.

Private raw logs, fixtures and machine receipts are at the sibling SE outputs/c3-release-qualification directory. The public source contains generated checks and aggregate outcomes only. Subsequent fixes invalidate affected evidence and require the stated retests before production publication.

## Additional verification boundaries

Workspace `cargo check --locked --offline` passed. `cargo fmt --all --check` fails in both a pristine archive of baa8908 and the current source; the baseline/current logs are retained. This phase keeps targeted repairs rather than reformatting unrelated code. Formatting debt remains a recorded source gate.

C3_PROFILE_DIR now also selects a canonical-path-derived WebKit data-store identifier on macOS 14 or later. A main window is created only after profile admission; older macOS fails closed for this QA path and needs a separate account. Native A/B draft/layout/localStorage and restart qualification remain unrun. Use the exact signed distribution artifact, with unchanged bundle identity, rather than substituting another bundle's results. Plain `open` does not by itself select a backend QA profile; follow explicit process/profile setup and verify it before mutation.

## Authority and ownership source checkpoint

The new shared evaluator is deny-first and applies actual operation effects,
immutable review ceilings, Plan restrictions and epoch-bound pending identities.
Host filesystem writes use anchored paths; host terminals have a generated macOS
containment fixture. Native option IDs are preserved, and the frontend no longer
creates inferred wildcard grants. Transport writes have a bounded deadline and
close the stream after failed or interrupted partial writes.

One process-wide canonical workspace coordinator covers managed ordinary and
Build sessions plus worktree mutations. Requested isolation failures are visible.
Land/Sync require explicit clean commits and preserve dirty work. This coordinator
does not control another process running Git independently.

Native adapters' internal runners remain unverified. Unsupported native Plan,
immutable review, workspace/strict and deny policies are refused before launch;
unsupported live Plan transitions are refused without changing the active mode.
This is an intermediate fail-closed capability boundary, not acceptance of native
Plan/reviewed Builds. Those features must be restored through demonstrated
runtime/broker enforcement and pass their packaged stories before release.

The consolidated locked/offline workspace suite passes 274 tests, with zero
failures and six native-prerequisite fixtures still ignored. This includes 59
ACP, 6 permission, 68 panel, 10 core and 12 worktree tests. Workspace check and
strict all-target Clippy pass. Frontend passes 199 tests, including eight tests
against the actual mode-change owner; Python passes 136 tests. Independent
architecture and performance/simplicity review accept this source checkpoint.
Retained failures include the outer sandbox refusing nested sandbox-exec; the
generated macOS containment test passes in the bounded unsandboxed rerun.

The queue considers independent candidates after ownership or missing-checkout
failure, with visible per-record diagnostics; bounded Git identity discovery
runs before admission locks. Live UI modes require acknowledged host state,
unknown state blocks Send including keyboard entry, and Yolo confirmation is
shared by button and keyboard paths. Historical notes/transcripts are reference
evidence and confer no tool or policy authority.

PACKAGED_CANDIDATE_MATRIX.md records the required
workflow, recovery, accessibility and performance observations on the final
signed candidate. No new candidate was installed, signed, submitted or published.

## Process completion and recovery source checkpoint

One shared supervisor owns the native ACP, hosted terminal and headless worker
lifecycle. Outcomes require observed exit, actual pipe settlement and cleanup of
the recorded scope. Typed results distinguish successful/nonzero exit, signal,
timeout, cancellation, interrupted descendants, I/O failure and unresolved
cleanup. Failed cleanup retains its owner and workspace reservation. Retrying
cleanup preserves the original terminal cause. A one-use host launch proof and
non-reaping child observation handle the fast-exit group-lookup race; an
unidentified, mismatched or reaped child does not become a verified group.

Scheduler snapshots record run/session identities before effects and retain the
observed result/history. Generic handler failure stays uncertain; interruption
never causes automatic replay. Pause affects future attempts, while Stop requests
active cleanup. Cleanup retries cannot remove or overwrite a newer run. Quit
fences admission, reconciles the scheduler before removing session owners, and
retains unresolved bindings and the native window for another recovery attempt.
The user Stop-all operation remains separate from the app-exit lifetime fence.

The final locked/offline workspace suite passes **319 tests, zero failures and
six existing prerequisite-dependent ignored fixtures**. ACP passes73, wrapper21
(13 supervisor), core14, scheduler12 and panel71. Workspace check and strict
all-target Clippy pass. Frontend passes208, including nine tests executing the
actual scheduler controller. Python passes158, including22 generated candidate
evidence-validator tests. Both independent review lanes accept this scoped
checkpoint. Generated auth tests now use temporary files instead of reading the
normal auth store.

Earlier failed receipts remain outside Git. Independent reruns exposed the
fast-exit adoption defect; the corrected implementation passes64 delayed-adoption
and64 production-terminal quick exits. The external-drain fixture's observation
deadline now derives from its explicit cleanup budget. The initial sandboxed
workspace run could not signal its generated process group; the same bounded
Quit fixture and final full suite pass outside that inherited sandbox.

These are macOS source results. Dedicated process-group cleanup explicitly
excludes detached descendants; other targets refuse launch until their process
semantics are verified. Restricted native Plan/reviewed roles remain unavailable
pending complete runtime/broker enforcement. Aggregate ACP queue/task admission,
growing scheduler snapshots and the remaining process-owning utilities still
require their resource/lifecycle qualification. No result here establishes
provider Stop/resume continuity, final packaged recovery or production readiness.

The candidate evidence validator enforces pinned receipt consistency, current
story contracts, independent repeat passes and raw matched native performance
evidence. Its generated tests use mock candidates. It neither independently
verifies observations nor asserts human approval, signs, submits or publishes.

## Durable events, ownership and recovery source checkpoint

Gate3 now uses one retained SQLite writer for committed events and their
transcript/status/metadata projections, workflow/scheduler snapshots and typed
operation records. Publication follows the transaction; subscribers are no
longer responsible for independent best-effort persistence. Runtime generations
fence retired producers. Database, WAL and SHM identity are checked before
transactions and after commit before acknowledgement. A post-commit failure
retains the old receipt and reports uncertainty, without claiming rollback.

The frontend subscribes before consistent paged snapshots, replays above the
watermark, deduplicates sequences and reconciles lag/import/removal. Failed reads
remain retryable. A 2,000-row/4MiB tail and 300-row DOM window expose earlier
coverage rather than using transcript length as completeness. Historical approval
text is inert; current pending controls require the exact host runtime and turn.
Unresolved and explicitly uncertain/dispatched operation outcomes remain visible.

Prompt admission persists each accepted user submission once and preserves rejected
drafts. Completion waits for queued output. Accepted native projection overflow
fails current-runtime coverage instead of disappearing before success. Policy
changes commit intent before mutation and restore local state if their outcome
fails while dispatch remains locked. A secondary metadata failure exposes the
actual already-acknowledged mode locally as Failed and blocks new prompts. Saved
renames preserve flat/nested authority schemas and reject corrupt metadata.
Headless terminal outcomes are saved before completion acknowledgement.

The integrated locked/offline suite passes **380 Rust tests, zero failures and
six existing prerequisite-dependent ignored tests**. ACP86, core22, panel85,
persistence32 and events6 are included. Frontend passes245; Python passes158.
Workspace check, strict all-target Clippy, affected JavaScript syntax and
whitespace checks pass. Both independent review lanes accept the scoped source.
Receipts are `gate3-workspace-tests-final-v2.log`,
`gate3-workspace-check-final-v2.log`, `gate3-workspace-clippy-final-v1.log`,
`gate3-frontend-tests-final-v1.log` and `gate3-python-tests-final-v1.log` in the
local release qualification output directory. Failed probes remain outside Git.

A full-suite failure exposed a kill-versus-fork race. Cleanup now re-signals live
members inside the unchanged deadline, retaining the unreaped leader identity.
Eight repeated parallel wrapper suites pass168 test executions, including256
immediate failure/fork trials with actual group-quiescence checks. This does not
extend the supervisor beyond its documented dedicated process-group scope.

The generated owned-writer probe measured commit p50 170µs, p95 3,775µs,
p99 9,357µs and maximum17,406µs. A deliberately non-yielding1,000-event Tokio
burst delayed its heartbeat584ms. These are source measurements, not accepted
native responsiveness budgets. Final signed real-ingress/frame/RSS testing is
required. Snapshot count/time limits, structured event byte limits, physical
lock scope, clone-generation rotation, untagged late provider attribution and
legacy noncooperating writers remain explicit limits in the implementation plan.

No installed app, original store, credentials, provider workflow, new signature,
Apple submission or production publication was changed by this source checkpoint.
Whole-native enforcement, remaining utility-worker bounds/ownership and final
signed packaged workflow/recovery/accessibility/performance remain release gates.
The installed-adapter follow-up in `docs/plan/native_policy_broker.md` preserves
native authentication and records the concrete suppression/resume/distribution
work required before lifting restrictive-role guards.
