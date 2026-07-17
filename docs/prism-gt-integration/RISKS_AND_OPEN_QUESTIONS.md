# Risks and Open Questions

This document maintains the evidence-grounded risk register and open questions for the `jedisherpa/grok-build-tauri-control-panel` repository at the frozen commit `994e1e39f39119998bb9a4a3c047128375eb7067`.

## Frozen Source Identity
- **Repository:** `jedisherpa/grok-build-tauri-control-panel`
- **Base Branch:** `main`
- **Frozen Commit:** `994e1e39f39119998bb9a4a3c047128375eb7067`

## Evidence-Grounded Risk Register

1. **External Dependency on Grok Build CLI**
   - *Risk*: The control panel relies entirely on the external `grok` binary for agentic reasoning and execution. Changes to the Grok Build CLI's Agent Client Protocol (ACP) could break the `grok_acp` crate.
   - *Mitigation*: Pin the expected Grok Build version in configuration and implement robust error handling for JSON-RPC parsing failures.

2. **Concurrency and Lock Contention**
   - *Risk*: The workspace depends on `DashMap` and planning/advisory docs discuss `DashMap`/`RwLock` for session registry; actual `SessionRegistry` synchronization primitives were not verified from supplied source excerpts. High concurrency could lead to lock contention or deadlocks.
   - *Mitigation*: Ensure all handles are `Send + Sync` and avoid nested locks. Implement timeouts for all spawn/prompt operations.

3. **Security of MCP Servers**
   - *Risk*: Integrating external MCP servers (e.g., Filesystem, Playwright) introduces severe security risks if path validation or sandboxing is bypassed.
   - *Mitigation*: Enforce deny-first permission evaluation. Require explicit user approval for high-risk servers and validate all absolute paths.

## Contradictions

- The file `README.PRISM-GT-INTEGRATION.md` exists in the repository root and claims the repository is "pending documentation integration." This file is considered preparation evidence and does not constitute a canonical package or an approved integration contract. Current package references to the preparation guide must not support implementation or contract claims.

## Owner Decisions Required

- **Prism GT Integration**: The repository owner (`jedisherpa`) must explicitly approve the proposed Prism GT telemetry and integration contracts before they can be marked as locked.
- **Sandbox Enforcement**: A decision is required on whether to implement OS-level sandboxing (e.g., macOS seatbelt) in future phases, as proposed in the architecture advisories.

## Stop Conditions

Publication and integration will halt if any unresolved high-severity security findings (e.g., credential logging, sandbox escapes) are identified during the independent audit.

## Frozen source boundary

This document is limited to the exact repository, branch, and frozen source commit recorded in the package metadata and cited references. Later remote changes require a new source freeze and independent review.

## References

1. [Core Architecture Advisory (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/advisories/multi_perspective_advisory_core_architecture.md)
2. [Extensions Advisory (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/advisories/multi_perspective_advisory_extensions_memory_scheduler.md)
3. [Prism GT Integration Guide (Preparation Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/README.PRISM-GT-INTEGRATION.md)
