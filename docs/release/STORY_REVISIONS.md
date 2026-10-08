# Story revisions and defects

## Initial discoveries (2026-10-07)

- C3-039: prior local installation worked because Joe/dictionary paths named `/Users/paulcooper` and a particular NVM Node version. Revised expectation: resolve the current user's host-owned frozen package and Node from executable PATH without accepting UI-supplied runtime paths. Local source is being repaired; availability of the frozen reference/Python/model on a clean Mac remains an independent gate.
- C3-043: prior installer deleted the destination and force re-signed it ad-hoc while swallowing signing errors. Expected release behavior requires retaining the tested Developer ID signature and a safe, reviewable install/rollback path. Implementation is being repaired.
- C3-018: preview is implemented for the first part of a numbered sequence. The story explicitly exposes this limitation, rather than claiming all parts were previewed. Later actual submission still validates each part in the native host; partial submission is independently recorded in C3-019.
- C3-040: current UI says See Cubed while package name/docs remain Bomb Code. Complete visible branding with unchanged storage/bundle identity where required for migration.

Future revisions must name story ID, original expected behavior, observed evidence, whether this is a defect or an expectation correction, exact change and retest result. A passing smoke check never replaces an unrun required story.

## Repair and audit checkpoint

- C3-002/039: QA path selection alone is insufficient isolation because vendor CLI authority and WebKit localStorage remain separate. Main MCP mirroring is disabled and Haven config now routes to the profile; use fake adapters or a separately isolated account/browser store before native mutation tests.
- C3-032: observed source changed punctuation-only search to browse-all while the unchanged literal-search test expected zero. Restore zero matches for a nonempty query without searchable tokens; empty query still browses. Initial failing receipt retained; complete Python retest passes.
- C3-036: corrupt startup configuration is preserved; named regression covers corrupt base and corrupt overlay before discovery save. Native recovery/error UX still open.
- C3-025: generated concurrent notes exposed shared-temp publication races. Named regressions now verify 32 simultaneous adds and exact reopen IDs, failed-add visibility, and preservation of corrupt source/prior backup; multi-process and directory/power-loss durability still open.
- C3-012/038: byte clipping now retains UTF-8 boundaries at terminal/file caps; named regression passes, but whole-file allocation and admission bounds remain open.
- C3-004: paused/reduced-motion sheets still do work; platonic solid auto-rotation does not honor application Pause/hidden lifecycle. Keep this as an implementation defect; do not redefine Pause to match it.

## Authority and ownership remediation (2026-10-07)

- C3-014/015: the old permission preset test expected `git add` to auto-run even while Plan was active. Correct expectation: Plan forbids process/write effects regardless of a permissive preset; only an explicit mode transition can widen mutable authority. Immutable review-role ceilings remain in effect across that transition. Tool labels and plan text cannot redefine the operation's effect. Generated native ACP regressions exercise the actual host dispatch and pending-request identities; packaged provider enforcement remains a separate gate.
- C3-014: the frontend also synthesized a second Always button that installed `Tool(*)` or a program-prefix glob before granting one request. Remove that duplication. Approval cards now render the host/peer's original options and return the selected ID. Host Always grants retain the literal request identity; intentionally authored policy patterns remain a separate configuration feature.
- C3-014/015: a refused host mode change previously left the UI pill changed, and Shift+Tab could enter Yolo without the button's confirmation. Live changes now read back actual host state; unknown state blocks button and keyboard Send until confirmed. One controller owns Yolo confirmation for both paths and ignores late results for a different selected thread. Eight tests execute that actual owner and pass.
- C3-021: Land/Sync previously staged every outstanding edit before integrating. Revised workflow requires an explicitly committed clean checkout. A dirty checkout produces a visible error and preserves staged, unstaged and untracked work. This corrects an unsafe expectation; it does not remove worktree review or integration.
- C3-002/039: isolated native paths must also have a distinct WebKit store before window creation. On macOS, persistent QA store identifiers require macOS 14 or later; older systems must use a separate account rather than silently falling back to the normal browser store. Six generated path/identity fixtures pass, including optional stores and canonical aliases. Actual packaged restart and browser-store separation are pending.
- C3-009/034: discovery must preserve an explicitly configured test/backend program in both base and project configuration. Automatically replacing it with an installed real CLI would invalidate generated-fixture isolation. A startup regression checks both configurations and preservation of the base selection.

Consolidated source checks pass: 274 Rust tests, six ignored native-prerequisite fixtures; 199 frontend tests; 136 Python tests; workspace check and strict all-target Clippy. These corrections do not close whole-native enforcement, process-supervision, durable-event or final signed-candidate stories.

## Process-completion expectation corrections (2026-10-07)

- C3-034: the previous scheduler story called Stop “delete” and did not distinguish launch/admission from successful completion. Revision 2 requires retained run history, an admitted run/session binding before the effect, an observed worker outcome, and visible interrupted/uncertain recovery. Pause controls future attempts; Stop also requests current cleanup. Explicit restart creates a new attempt without asserting that previous external effects were undone. Generated scheduler tests cover those transitions; nine tests execute the actual frontend controller, including declined restart, unconfirmed cleanup, active paused work, failed Stop recording, duplicate confirmations and preservation of a newer draft. Final signed packaged observations remain required.
- C3-009/010/011/013/036: a parent exiting zero does not establish successful process completion when its owned group still contains a running worker. Typed completion records distinguish exit, signal, timeout, cancellation, interrupted descendants, pipe/read errors and unconfirmed cleanup. Cleanup receipts state their scope. Dedicated process-group cleanup cannot certify containment of a detached child; whole-native containment and provider workflow qualification remain independent release gates.
- C3-012/038: malformed or oversized native frames must trigger a visible transport failure and bounded cleanup, while readers continue discarding until actual EOF. Stop cannot wait behind a terminal wait mutex, and a late turn completion cannot restore Idle after Stop. Generated native-transport/terminal fixtures exercise these boundaries. They do not establish final package recovery or performance.

## Durable-event expectation corrections (2026-10-07)

- C3-012/038: the previous renderer preferred whichever transcript had more rows and marked a failed database load as loaded. Neither establishes completeness. Revision 3 requires a consistent baseline watermark, ordered replay, sequence deduplication, retryable failures, explicit earlier-window coverage and removal/import reconciliation. Generated tests use 50,000 authored rows and concurrent snapshot notifications; they do not qualify native frame time or the signed package.
- C3-014: stored approval text previously lacked a host runtime/turn binding. Revision 3 keeps every historical card inert; missed current controls come from a bounded host query and checked response carrying the exact runtime and host epoch. Queued native frames receive that epoch at parsing, rather than when dequeued. Untagged peer messages emitted after another turn begins remain a protocol attribution limit; the host does not invent a peer causal tag.
- C3-038: losing the process-lifetime ownership lock proves that the prior writer exited, not that its native children or external effects finished. The startup transition records unobserved prior activity as recovering, retains unresolved operation IDs and never replays them. Generated OS-lock, SQL-abort, restart and WAL-backup fixtures are source evidence; packaged crash recovery and power-loss durability remain separate gates.
- C3-020/038: workflow snapshots previously committed outside the event journal. Reviewed-build and scheduler snapshot mutations now need the same durable transaction as their host event. Failed commits retain reservations and do not publish an admitted state. An external effect interrupted before its correlated outcome remains uncertain and requires inspection before retry.

The final source counts and independent review result will be recorded after the Gate 3 freeze. All revised contracts must be run and repeated on the unchanged final signed candidate; existing source fixtures are not packaged acceptance.

- C3-014/015: failed policy publication must not retain a new allow rule, mode or exact Always grant. Commit intent/outcome under dispatch, restore local policy on failed outcome, and show the actual acknowledged mode as Failed if a later metadata receipt fails; reject future effects while coverage is unhealthy.
- C3-012/038: a valid native frame can still exceed the structured replay-envelope budget after JSON escaping. Accepted canonical output loss fails current-runtime coverage before completion acknowledgement; stale producers cannot poison a successor.
- C3-013/034/038: a single group signal can race a fork. Repeat verified live-group signalling within the original cleanup deadline while retaining leader identity; preserve the failure receipt. Group cleanup still cannot certify a detached child.

Final Revision3 source checks:380 Rust passes/0 failures/6 existing ignored;
245 frontend passes;158 Python passes; workspace check, strict all-target Clippy,
JavaScript syntax and diff-whitespace pass. Signed packaged observations remain
unrun; these source results do not close the final-candidate matrix.
