# Final signed candidate acceptance matrix

This matrix implements Paul's required ordering: policy/workspace ownership,
observed process completion, durable events, then packaged workflow, recovery,
accessibility and performance acceptance. It supplements all applicable stories
in USER_STORIES.md; it does not replace provider or clean-account qualification.

## Candidate identity and isolation

Before a run, record the clean source SHA, locked dependencies, build command and
architecture, signed bundle manifest, main executable digest, Developer ID/team,
hardened runtime and timestamp. Keep the test profile, generated repository and
receipts outside Git. A source or bundle change invalidates affected results.
Use the same distribution identifier and executable for all four categories.
Do not change the identifier or re-sign an accessibility/performance copy.

Each test run uses an explicitly created absolute C3_PROFILE_DIR, a distinct
WebKit identifier and a configured generated fake adapter. Verify two profiles
have independent drafts/layout/local storage and that relaunch restores each
one. Original archives, provider credentials and the installed app stay outside
generated mutation tests. Configured external adapters require separate runtime
containment proof; profile path routing alone is insufficient.

## Required observations

| Category | Generated scenario | Required observation |
|---|---|---|
| Workflow | Plan adapter requests file write, shell, unknown tool, forged read label and misleading plan text | No effect or widened authority; explicit failure/approval follows the actual operation. |
| Workflow | Literal filename containing `*`, native numeric/string request IDs, duplicate/stale approval, cancellation and mode changes | One-use response retains exact pending identity; Always does not create an inferred glob; stale grants cannot reach a later turn. |
| Workflow | Two isolated sessions, overlapping/disjoint Builds, alias checkout, dirty Land/Sync | Allowed independent work starts; blocked owners are explained; staged/untracked bytes and existing ownership remain intact. |
| Workflow | One reviewed generated edit with one deliberate failed verification | Plan approval precedes implementation; repair is bounded; human result acceptance precedes dependencies; exact before/after diff survives. |
| Recovery | Long-running/flooding/nonzero/hanging/descendant worker; Stop and app restart | Pipes stay bounded; only observed exit/cleanup permits a terminal state; unresolved cleanup retains ownership. |
| Recovery | Scheduled run stopped or app killed between persisted intent, spawn and outcome | Invocation identity/history survive; interrupted effects remain uncertain and are not automatically rerun. |
| Recovery | UI consumer lags/disconnects; writer fails; second process opens the same profile | Committed events replay in order without duplication; write failure is visible and prevents new consequential work; second writer is refused before mutation. |
| Recovery | Restart with pending approval and removed thread | Historical approvals are inert; removed state cannot be resurrected; complete committed transcript is reconstructed. |
| Accessibility | Keyboard-only composer, approvals, Builds, panels, Joe/play/mute at minimum size | Reachable named controls, visible focus, no trapped focus, correct activation and dismissal. |
| Accessibility | VoiceOver, reduced motion, Pause and background/foreground transition | Useful accessible names/state/order; decorative motion settles; no resume jump or hidden action. |
| Performance | Matched cold/warm startup, 1k/10k/50k transcript, 1/3/8 sessions and repeated navigation | Report startup-to-interactive, scroll/frame/input latency, CPU and main/WebKit/provider RSS separately; no growing retained-state slope. |

## Measurement rules

Retain raw traces, scenario inputs, sample count, machine/display conditions and
measurement method. Compare the installed baseline and final signed candidate
using generated profiles and the same scenarios. Do not infer native frame time
from fake requestAnimationFrame counts or attribute provider RSS to the shell.
Record engineering budgets before measuring acceptance; a missing budget or a
metric that cannot be measured is an open gate, not a pass. A failed budget needs
a diagnosed fix and rerun, or a documented product expectation for Paul's review.

All four categories must pass on the final signed candidate before notarization.
Signing and Apple's Accepted response do not establish feature qualification.
After stapling, recheck signature and ticket, retain the changed bundle manifest,
and verify the downloaded asset has the same executable/resources and passes
Gatekeeper plus the upgrade/rollback story. Include failures, revised stories and
unrun external requirements in QUALIFICATION_REPORT.md.
