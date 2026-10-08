# Proposed Target Architecture

This document outlines the proposed target architecture for integrating the `jedisherpa/grok-build-tauri-control-panel` repository into the Prism GT ecosystem, based on the frozen commit `994e1e39f39119998bb9a4a3c047128375eb7067`.

## Frozen Source Identity
- **Repository:** `jedisherpa/grok-build-tauri-control-panel`
- **Base Branch:** `main`
- **Frozen Commit:** `994e1e39f39119998bb9a4a3c047128375eb7067`

## Component and Data Flows

The proposed architecture maintains the existing Tauri-based structure while formalizing the integration seams:
1. **Frontend UI (`frontend/`)**: Lightweight HTML/JS/CSS interface communicating via Tauri IPC commands.
2. **Tauri Host (`src-tauri/`)**: Bridges the frontend to the Rust backend crates.
3. **Orchestration Core (`grok_control_core`)**: May continue or introduce the use of `DashMap` or `Arc<RwLock>` for the `SessionRegistry` (planning/advisory recommendations discuss this, but exact current implementation was not verified from supplied excerpts).
4. **Protocol Layer (`grok_acp`)**: Handles JSON-RPC over NDJSON stdio communication with the `grok` CLI.
5. **Extension Management (`grok_mcp`, `grok_extensions`)**: Manages external tools and Model Context Protocol servers.

Data flows from the user interface, through the Tauri IPC, into the orchestration core, which then spawns or communicates with isolated `grok` agent processes via ACP.

## Migration Boundary

The migration to the Prism GT ecosystem involves standardizing the `grok_events` broadcast bus to emit events that can be consumed by other Prism GT telemetry and monitoring tools. The existing SQLite persistence (`grok_persistence`) will be evaluated for schema compatibility with broader ecosystem logging.

## Security and Operations Controls

- **Strict Sandboxing**: Worktrees are isolated per thread. OS-level sandboxing (macOS seatbelt, Linux seccomp) is proposed for future phases.
- **Deny-First Permissions**: `AGENTS.md` documents a deny-first security default; enforcement in `grok_permissions` was not independently verified from the supplied code excerpts. The `--always-approve` flag is strictly prohibited as a default.
- **Credential Management**: MCP secrets and API keys are documented to be stored in `~/.grok/mcp_credentials.json` with `0600` permissions and must never be logged.

## Approval Dependencies

The target architecture is currently in a **proposed** state. Full integration requires explicit owner/maintainer approval for this repository and the responsible Prism GT/canonical documentation approver identified by the publication governance; no named architecture review board is evidenced here.

## Frozen source boundary

This document is limited to the exact repository, branch, and frozen source commit recorded in the package metadata and cited references. Later remote changes require a new source freeze and independent review.

## References

1. [Core Architecture Advisory (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/advisories/multi_perspective_advisory_core_architecture.md)
2. [Agent Swarm Architecture (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/agent_swarm_architecture.md)
3. [Security Defaults](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/AGENTS.md)
