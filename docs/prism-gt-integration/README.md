# Package Status and Repository Identity

This is a proposed canonical 11-file documentation package under evidence audit for possible documentation-only draft PR publication for the `jedisherpa/grok-build-tauri-control-panel` repository. The documentation is currently in a **proposed** state, pending independent review and owner approval. It is not published, approved, merged, or live-verified. The repository documents a production-oriented Rust and Tauri 2 desktop control panel for the Grok Build CLI, orchestrating multi-session agent workflows.

## Frozen Source Identity
- **Repository:** `jedisherpa/grok-build-tauri-control-panel`
- **Base Branch:** `main`
- **Frozen Commit:** `994e1e39f39119998bb9a4a3c047128375eb7067`
- **Evidence Limitations:** Executable source at the frozen commit outranks repository documentation. Planning documents (`docs/plan/*`, `docs/mcp_plans/*`) and preparation guides (`README.PRISM-GT-INTEGRATION.md`) are planning/preparation evidence only and must not be cited as proof of implemented current state.

## Repository Role

The repository, also known as "Bomb Code", acts as an orchestrator and graphical interface for the Grok Build CLI. It does not ship the Grok binary itself but requires it to be installed and authenticated. Its primary documented role is to provide Agent Client Protocol (ACP) integration, worktree isolation, MCP management, and multi-session orchestration.

## Package Map

This canonical package consists of exactly 11 files. The target repository paths are `docs/prism-gt-integration/<filename>` and the canonical paths are `repository-docs/jedisherpa__grok-build-tauri-control-panel/<filename>`:
1. `README.md`: Package status, source freeze, and repository role.
2. `CURRENT_STATE.md`: Observed capabilities, architecture, and boundaries.
3. `ROLE_IN_ECOSYSTEM.md`: Upstream/downstream relationships and integration seams.
4. `TARGET_ARCHITECTURE.md`: Proposed target architecture and component flows.
5. `INTEGRATION_CONTRACTS.md`: Observed and proposed interfaces.
6. `IMPLEMENTATION_PLAN.md`: Phased future work and integration steps.
7. `TEST_AND_ACCEPTANCE_PLAN.md`: Baseline commands and proposed test matrix.
8. `RISKS_AND_OPEN_QUESTIONS.md`: Evidence-grounded risk register.
9. `AGENT_HANDOFF.md`: Safe start commands and package map.
10. `implementation-manifest.yaml`: Machine-readable repository metadata.
11. `generation-metadata.json`: Machine-readable generation route, hashes, and provenance.

## Evidence Boundary

The evidence boundary for this package includes the Cargo workspace crates (`crates/*`), the Tauri backend (`src-tauri/`), the frontend assets (`frontend/`), and the extensive multi-agent build plans located in `docs/plan/`. Executable source at the frozen commit outranks repository docs; `docs/plan/*`, `docs/mcp_plans/*`, and `README.PRISM-GT-INTEGRATION.md` are planning/preparation evidence only and must not be cited as proof of implemented current state.

## Frozen source boundary

This document is limited to the exact repository, branch, and frozen source commit recorded in the package metadata and cited references. Later remote changes require a new source freeze and independent review.

## References

1. [Repository README](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/README.md)
2. [Prism GT Integration Guide (Preparation Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/README.PRISM-GT-INTEGRATION.md)
3. [Multi-Agent Build Plan (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/README.md)
