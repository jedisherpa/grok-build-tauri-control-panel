# Recorded collaboration and completion

## Problem and approved scope

Paul requested a fix for the audit limit: independent session activity does not establish collaboration and output volume cannot measure task completion. Add a read-only projection to the existing reviewed Builds workflow and replace the reply-volume pseudo-percentage. Preserve native approvals, execution and the current Wizard Joe guide.

## Evidence contract

Build arrows come only from persisted prerequisite IDs. Role links come only from persisted role attempt session IDs or an active session owned by the build service. Declared workflow order is labelled separately from observed completion; unknown or independent sessions acquire no inferred edges. Missing links, cycles and stale snapshots are explicit.

Six equal checkpoints are plan recorded, exact plan approval, implementation recorded, audit PASS, verifier PASS and final acceptance. Percentage is floor(completed*100/6), with denominator and basis always visible. Current-round ordered reports, valid approval binding and strict verdicts are required. Repair rounds invalidate earlier implementation/review checkpoints. Cancellation/restart preserves historical evidence but cannot reach acceptance. This percentage describes workflow checkpoints, not remaining time or within-role work. Ordinary native task completion remains unknown.

## Implementation and validation

Pure Rust evidence projection serialized by existing BuildDto. Pure frontend graph model plus accessible native controls in Builds and per-session context in Activity. No new execution service or authority. Remove output-size/tool-count pseudo-progress.

Adversarial Rust tests cover human gates, repairs, malformed/latest failed verdicts, stale approvals, reordered reports and persistence. Frontend tests cover explicit graph edges, missing/cyclic links, no inferred same-folder edges, stale repair rounds and unknown completion. Run full workspace tests/check/strict Clippy plus existing Joe frontend tests. Architecture review and revision follow repo AGENTS. Build and install local0.1.6, verify actual native graphs/session links/progress/unknown states, retain rollback and all current app records/settings.
