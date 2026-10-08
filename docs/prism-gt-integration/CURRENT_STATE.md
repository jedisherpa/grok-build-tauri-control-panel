# Current State and Capabilities

This document outlines the observed current state of the `jedisherpa/grok-build-tauri-control-panel` repository at the frozen commit `994e1e39f39119998bb9a4a3c047128375eb7067`.

## Frozen Source Identity
- **Repository:** `jedisherpa/grok-build-tauri-control-panel`
- **Base Branch:** `main`
- **Frozen Commit:** `994e1e39f39119998bb9a4a3c047128375eb7067`

## Frozen-Code Capabilities

Repository documentation/manifests describe a Tauri 2 desktop application with a Rust backend divided into a Cargo workspace. Key capabilities described in documentation include:
- **ACP Client**: Long-lived interactive sessions via JSON-RPC over NDJSON stdio (`grok agent stdio`).
- **Multi-Session Registry**: Concurrent access managed via `DashMap`.
- **Worktree Isolation**: Thread-per-worktree isolation for parallel agents.
- **MCP Management**: Cataloging, CRUD operations, doctor checks, and session injection for Model Context Protocol servers.
- **Security Enforcement**: Deny-first permission evaluation, with "plan mode" enabled by default.
- **Frontend Presence**: A state machine in `frontend/presence.js` managing turn phases.

*Evidence Limitation:* The selected source packet did not include enough code excerpts to independently verify these implementations at the symbol level. Crate existence, Cargo descriptions, README claims, and planning documents do not prove the stated runtime capabilities are fully implemented.

## Entry Points

The file inventory contains these likely entry-point paths:
- **Backend**: `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, and `src-tauri/src/commands.rs`.
- **Frontend**: `frontend/index.html` and `frontend/app.js`.
- **Core Logic**: `crates/grok_acp/src/lib.rs`.

*Evidence Limitation:* Initialization and command registration were not independently verified from supplied excerpts.

## Architecture and Boundaries

The architecture is highly modular, consisting of several crates listed in the workspace `Cargo.toml`:
- `grok_config`: Configuration discovery and TOML parsing.
- `grok_cli_wrapper`: Typed async wrapper around the Grok Build CLI.
- `grok_acp`: ACP client implementation.
- `grok_control_core`: Session registry and orchestration.
- `grok_mcp`: MCP manager.
- `grok_permissions`: Sandbox policy engine.

**Data Boundaries**: Repository README documents the intended config locations: configuration in `~/.grok/control-panel/config.toml`, MCP secrets in `~/.grok/mcp_credentials.json` (mode `0600`), and SQLite recovery databases in `~/.grok/control-panel/sessions/`. Runtime filesystem behavior was not independently verified from the supplied source excerpts.

## Observed command instructions, not executed in this package

The repository `AGENTS.md` uses standard Cargo testing and linting commands:
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo tauri dev` (Observed development command; requires a desktop environment and Grok CLI installed)

*Evidence Limitation:* No frozen command output or pass/fail result was provided for these commands. They were not executed during this evidence audit.

## Evidence Limitations

The control panel does not ship the Grok binary; it strictly requires the `grok` CLI to be installed and authenticated on the host system. 

## Frozen source boundary

This document is limited to the exact repository, branch, and frozen source commit recorded in the package metadata and cited references. Later remote changes require a new source freeze and independent review.

## References

1. [Cargo Workspace Definition](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/Cargo.toml)
2. [Agents Documentation](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/AGENTS.md)
3. [Core Architecture Advisory (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/advisories/multi_perspective_advisory_core_architecture.md)
