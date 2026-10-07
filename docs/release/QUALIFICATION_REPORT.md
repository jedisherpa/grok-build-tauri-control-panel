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

## Fresh verification

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

The architecture findings on policy parity/direct ACP operation authorization, repository reservations/isolation fallback, actual scheduler/headless completion/recovery, durable event ingestion, diff correctness and worker/resource caps remain open. Review the audit reports and staged backlog; unit-suite passes do not override reproduced defects.

The cached baseline Rust SQLite amalgamation is 3.46.0; Persistence uses WAL/NORMAL. It needs a verified patched runtime and checkpoint/ownership/durability qualification. The system Python SQLite reports 3.43.2, and Python/runtime/reference installation and licenses also need a separate distribution check. No installed app SQLite query or corruption incident is asserted here. Current official SQLite guidance fixes its rare multi-connection WAL-reset race in 3.51.3+, with 3.44.6/3.50.7 backports: https://www.sqlite.org/wal.html#walresetbug.

Packaged final-candidate stories, actual provider workflows, deliberate recovery tests, clean-account/prerequisite install, accessibility, matched startup/scroll/frame/RSS measurements, signing/hardened-runtime retest, upgrade/rollback and downloaded-asset checks remain unrun for this new source. The existing installed app was observed read-only; historical native results do not qualify this candidate.

This Mac currently reports zero valid code-signing identities. Apple Developer membership/certificate/notary profile and production architecture/destination remain pending user clarification. Do not place credentials in source or chat. Only the user/account holder can supply the required setup. No signing/notarization submission, production publication, application replacement or private history/provider transmission occurred in this phase.

Private raw logs, fixtures and machine receipts are at the sibling SE outputs/c3-release-qualification directory. The public source contains generated checks and aggregate outcomes only. Subsequent fixes invalidate affected evidence and require the stated retests before production publication.

## Additional verification boundaries

Workspace `cargo check --locked --offline` passed. `cargo fmt --all --check` fails in both a pristine archive of baa8908 and the current source; the baseline/current logs are retained. This phase keeps targeted repairs rather than reformatting unrelated code. Formatting debt remains a recorded source gate.

C3_PROFILE_DIR changes backend paths, not WebKit's persistent browser data identity. Draft/layout/localStorage qualification requires a separately verified WebKit store or isolated macOS account/test bundle identity. The exact signed final artifact must be exercised on an isolated account without substituting another bundle's results. Plain `open` does not by itself select a backend QA profile; follow the explicit process/profile setup and verify it before mutation.
