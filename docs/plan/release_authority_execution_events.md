# C3 authority, completion and durable-event remediation

Requested by Paul on 2026-10-07. Start from clean local revision `24124f2` in
`codex/c3-release-qualification`. Retain the Rust/Tauri product and its existing
meaning, memory, native-provider and reviewed-workflow contracts.

## Ordered gates

1. **Policy and workspace ownership.** Use one deny-first permission evaluator
   in preview and native ACP approval/dispatch. Enforce immutable review-role
   ceilings independently of mutable approval modes. Preserve actual RPC IDs,
   one-use grants and cancellation. Explicitly distinguish host dispatch controls
   from whole-provider process isolation. Introduce one canonical workspace
   admission owner shared by ordinary/resumed sessions, reviewed Builds and Git
   mutations. Fail requested isolation instead of sharing the project silently.
   Land/Sync require explicitly committed clean work; never stage unrelated work
   automatically. Preserve concurrent isolated checkouts and disjoint Build scopes.
2. **Observed process completion.** After Gate 1 review and checks, supervise
   existing headless sessions through bounded concurrent pipe drain, exit,
   timeout/cancellation and descendant cleanup. Give scheduler runs explicit IDs
   and typed outcomes. Preserve terminal history; interrupted effects require
   deliberate recovery and cannot be blindly rerun. Surface persistence failure.
3. **Durable events.** After Gate 2 review and checks, initialize the authoritative
   persistence ingress before producers. Commit events/projections before UI
   notification and expose sequenced replay/reconciliation. Reject conflicting
   profile writers before mutation. Upgrade and verify the actual linked SQLite
   runtime and effective durability settings; retain WAL-aware recovery evidence.
4. **Final signed candidate.** Build the pinned source with the already verified
   Developer ID identity. Test the exact artifact against packaged workflow,
   restart/failure recovery, keyboard/VoiceOver/reduced motion and measured
   startup/scroll/frame/RSS stories. Generated fake adapters and isolated stores
   come before real provider/private-history tests. Record failures and revised
   stories rather than treating signing as functional qualification. Submit with
   the existing working notarization profile only after required gates pass;
   preserve log, staple, Gatekeeper and downloaded-artifact receipts.

## Implementation and review ownership

The parent owns the plan, phase gates, integration and release evidence.
`release_implementation` owns permissions/ACP in Gate 1.
`stack_performance_audit` owns canonical workspace/core/Git integration in Gate 1.
`stack_architecture_audit` performs independent architecture review and prepares
the later process/event interfaces. Subsequent implementation is dispatched only
after the preceding source gate is reviewed. Shared-file edits require a handoff.

Each gate requires meaningful adversarial regression tests, affected suites,
workspace check and strict all-target Clippy, then a local checkpoint. Keep the
pre-fix receipts. Run the full Rust, Python and frontend suites at integration.
Existing unrelated baseline formatting failures remain separately recorded.

## Preservation and acceptance boundaries

Never mutate the installed app, original sessions/notes/history, CLI credentials,
frozen reference or signing keys during generated tests. Retain CDISS records,
SenseSnap scope, exact E8 coordinates, cited sources and immutable Joe receipts.
No geometry, source text or inferred meaning authorizes an operation.
No spawn acknowledgement, idle state, output activity or audit pass establishes
objective completion or human acceptance. Failed cleanup retains ownership.
No source-only test establishes packaged, signed or production qualification.

The existing developer profile routes backend stores but does not isolate the
WebKit data store or every external CLI. Verify further isolation before native
mutation. An older app that does not participate in a new profile lock must be
closed before shared-store activation. Any remaining unenforceable capability is
explicitly unavailable or an open release gate, never an implicit fallback.
