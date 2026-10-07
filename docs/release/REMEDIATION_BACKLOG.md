# Tauri reliability and simplification backlog

2026-10-07, baseline baa8908. This applies the existing product deconstruction/unified-architecture analysis to the current Tauri product. No renderer migration is required. Findings are in STACK_ARCHITECTURE_AUDIT.md and STACK_SIMPLICITY_AUDIT.md; story IDs refer to USER_STORIES.md.

## Protected product contracts

Preserve CDISS exact records and frames, dictionary/cross-language/sense/usage chain, fine E8 positions and collisions, scoped SenseSnap, all proposed readings, source hashes/citations, immutable Joe receipts/profile references, local hybrid recall, native provider/session IDs, worktree isolation and explicit plan/result review. Separate proposed meaning from authority. Keep source notes, imported history, derived retrieval indexes and interpreted profiles distinct behind one facade. Do not substitute a single vector store for these evidence types. Keep the frozen unsuccessful retrieval experiments as research rather than promoting their ranking proxies.

## Batch 0: bounded repairs and qualification preparation (current local work)

- Host-owned current-user semantic paths and PATH Node discovery; no client-selected reference/runtime. Optional QA stores preserve original default data locations. Disable MCP vendor mirroring and profile Haven config when isolated. Remaining external CLI state is explicitly outside complete profile isolation.
- Fail startup before saving when existing base or project config is corrupt/unreadable. Preserve original bytes and report errors; native failure UX is still a packaged story.
- Byte-boundary-safe UTF-8 clipping at ACP terminal/file responses.
- Serialize note mutation/publication; unique owner-only temporary files, file sync, publish in-memory only after successful rename. Preserve corrupt original and prior backup. One-process serialization does not yet exclude another app instance or prove power-loss directory durability.
- Literal punctuation-only history query has zero matches; empty query browses. Retain and repair the failing pre-existing test.
- Signature-preserving developer install; read-only artifact/source/signing/notary preflight and receipt capture.

Required checks: full Rust workspace tests/check/strict all-target Clippy, frontend/Python suites, focused regression review. These repairs are not production qualification or notarization.

## Batch 1: one policy and workspace admission owner

A02/A03/A08, C3-014–021. Reuse grok_permissions and BuildService coordination logic instead of adding another evaluator. Deny-first rules and immutable role ceilings must run at actual ACP file/process dispatch, named/raw MCP attachment, Git and headless boundaries. Preserve native pending approval identity. Requested isolation failure must fail visibly. Land/Sync/prune/remove and ordinary sessions must honor canonical repo/workspace reservations, exact dirty scope and cleanup ownership.

Acceptance: fake ACP adversarial Plan/deny/yolo/write/terminal tests; native generated repo approval/deny workflow; concurrent plain sessions/Builds/Land/Sync and symlink alias tests; unrelated manual/staged/untracked changes unchanged. Never treat cwd confinement as OS sandboxing.

## Batch 2: one supervised execution lifecycle and real scheduler runs

A04/A05, C3-009–020/034–035/038. Keep SessionRegistry and grok_workflows. Headless routines need bounded stdout/stderr drainage, observed exit, timeout, descendant cancellation, explicit job-run IDs/outcomes and overlap/misfire policy. Spawn is Running; objective acceptance and process exit remain distinct. Restore completed/failed/cancelled history without runners; old running effects become interrupted/uncertain rather than automatically replayed. Surface schedule persistence errors.

Acceptance: fake workers emitting beyond pipe capacity, nonzero exit, hang, child descendants, cancellation and host restart at each transition; actual generated scheduled prompt with explicit cwd; no false Completed or duplicated run.

## Batch 3: one durable persistence ingress and bounded read models

A06/A07 plus dependency gate, C3-002/013/025–033/038. Journal-before-notify or a lossless supervised persistence queue commits authoritative records before UI notifications. Renderer lag must reconcile through a consistent snapshot/replay; boot/store identity invalidates stale cursors/grants. Use the existing app SQLite owner and bounded typed projections, not the GPUI slice's second database. Preserve IDs and source evidence. Enforce single physical-store writer/process ownership. Verify a patched SQLite runtime before WAL activation; keep backups WAL-aware.

Acceptance: slow/dropped UI consumers, no-subscriber startup, killed writer, disk-full/commit error, concurrent app instance, corruption, checkpoint/restart, exact transcript reconstruction and stale gate rejection. A broadcast capacity increase is not a durability fix.

## Batch 4: one memory/runtime facade and coherent activity/motion

C3-003–007/013/026–031/039/041–042. Keep existing math reducers and Python/JS binder until exact parity justifies change. One runtime service owns versioned reference verification and bounded subprocess transport. One memory facade exposes distinct source/proposal types. Cache only a verified basis with explicit manifest invalidation and measured bounds. One canonical Activity reducer supplies all status surfaces. One motion lifecycle owns E8/Joe/sheets/model-viewer with pause/reduced/hidden/typing guards; paused idle should stop redundant DOM work.

Acceptance: complete frozen-reference and IEEE/profile/citation parity, tamper/stale/Unicode tests, different username and clean-account prerequisite install, local authored-provider interpretation, keyboard/VoiceOver, source-backed unsent draft. Never delete provenance or infer linguistic accuracy from coordinates.

## Batch 5: measured trims and distribution

A11–A15 and simplicity findings, C3-021/039–044. Correct duplicated diff and propagate Git failures; per-file exact before/after evidence replaces ambiguous aggregate capture. Retire duplicate internal MCP implementation only after compatibility calls all delegate to manager and config parity passes. Exclude test modules from production frontend assets through a verified staging build; preserve artwork/source attribution. Page long transcripts, bound cache eviction and avoid full-list redraw. Trim selected dependency features only after a target-specific feature tree/build/runtime comparison proves no lost capability; lockfile multi-version counts alone do not justify removals. Complete visible See Cubed metadata/docs with deliberate stable bundle/data identifiers.

Acceptance: matched startup/scroll/frame/RSS scenarios and agreed budgets; unchanged feature stories; audited third-party/reference distribution licenses; source/build receipt binding; exact signed candidate retest; Developer ID/hardened runtime, notarization log, staple, Gatekeeper and clean downloaded upgrade/rollback. No production publication while required stories or critical findings remain open.

## Change/evidence discipline

Keep a failing story/reproducer before each fix, add meaningful regression coverage and rerun affected plus integration suites. Record original/revised expectations and reasons. Large service consolidation happens incrementally behind current interfaces: no blanket crate merge, wholesale rewrite, database collapse, dependency removal by count, or change of mathematical meaning claims. Reopen evidence after relevant changes. Public summaries contain generated examples and aggregate results; private archives/credentials/receipts stay outside Git.
