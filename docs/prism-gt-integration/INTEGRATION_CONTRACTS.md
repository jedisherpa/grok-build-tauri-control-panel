# Integration Contracts

This document details the observed and proposed integration contracts for the `jedisherpa/grok-build-tauri-control-panel` repository at the frozen commit `994e1e39f39119998bb9a4a3c047128375eb7067`.

## Frozen Source Identity
- **Repository:** `jedisherpa/grok-build-tauri-control-panel`
- **Base Branch:** `main`
- **Frozen Commit:** `994e1e39f39119998bb9a4a3c047128375eb7067`

## Observed Interfaces

### 1. Agent Client Protocol (ACP)
- **Status**: Observed repository-local interface/crate vocabulary; implementation details not independently verified from supplied source excerpts.
- **Owner**: Unknown/not evidenced at frozen commit.
- **Producers**: Unknown/not evidenced at frozen commit.
- **Consumers**: Unknown/not evidenced at frozen commit.
- **Payload/Protocol**: Unknown/not evidenced at frozen commit.
- **Security**: Unknown/not evidenced at frozen commit.
- **Errors**: Unknown/not evidenced at frozen commit.
- **Versioning**: Unknown/not evidenced at frozen commit.
- **Approval Boundary**: Owner/maintainer approval required.

### 2. Model Context Protocol (MCP) - Local Vocabulary
- **Status**: Observed local crate/interface vocabulary (`crates/grok_mcp`).
- **Owner**: Unknown/not evidenced at frozen commit.
- **Producers**: Unknown/not evidenced at frozen commit.
- **Consumers**: Unknown/not evidenced at frozen commit.
- **Payload/Protocol**: Unknown/not evidenced at frozen commit.
- **Security**: Unknown/not evidenced at frozen commit.
- **Errors**: Unknown/not evidenced at frozen commit.
- **Versioning**: Unknown/not evidenced at frozen commit.
- **Approval Boundary**: Owner/maintainer approval required.

### 3. Tauri IPC Commands
- **Status**: Paths exist (`frontend/app.js`, `src-tauri/src/commands.rs`); concrete IPC contract not evidenced in supplied excerpts.
- **Owner**: Unknown/not evidenced at frozen commit.
- **Producers**: Unknown/not evidenced at frozen commit.
- **Consumers**: Unknown/not evidenced at frozen commit.
- **Payload/Protocol**: Unknown/not evidenced at frozen commit.
- **Security**: Unknown/not evidenced at frozen commit.
- **Errors**: Unknown/not evidenced at frozen commit.
- **Versioning**: Unknown/not evidenced at frozen commit.
- **Approval Boundary**: Owner/maintainer approval required.

## Proposed Interfaces

### 1. Model Context Protocol (MCP) - External Integrations
- **Status**: Planned/proposed from `docs/mcp_plans/`.
- **Owner**: Owner decision required.
- **Producers**: Proposed external MCP servers (e.g., `@modelcontextprotocol/server-filesystem`, GitHub MCP, Linear MCP).
- **Consumers**: `grok-build-tauri-control-panel` (Proposed)
- **Payload/Protocol**: stdio and http transports (Proposed).
- **Security**: TBD (Credentials proposed to be stored in `~/.grok/mcp_credentials.json`).
- **Errors**: TBD.
- **Versioning**: TBD.
- **Approval Boundary**: Owner/maintainer approval required.

### 2. Prism GT Broadcast Telemetry
- **Status**: Proposed/unknown.
- **Owner**: Owner decision required.
- **Producers**: Potential producer may be `crates/grok_events`.
- **Consumers**: Unknown/not evidenced.
- **Payload/Protocol**: Unknown/not evidenced.
- **Security**: TBD.
- **Errors**: TBD.
- **Versioning**: TBD.
- **Approval Boundary**: Owner/maintainer and canonical Prism GT approval required.

## Contract Ownership and Approval

All proposed contracts remain in a `proposed` state. No shared ecosystem interface is classified as `approved` or `locked` without an explicit, signed owner decision. The integration with Prism GT telemetry requires a formal review gate.

## Frozen source boundary

This document is limited to the exact repository, branch, and frozen source commit recorded in the package metadata and cited references. Later remote changes require a new source freeze and independent review.

## References

1. [ACP Crate Definition](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/crates/grok_acp/Cargo.toml)
2. [MCP Build Plans (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/mcp_plans/)
3. [Prism GT Integration Guide (Preparation Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/README.PRISM-GT-INTEGRATION.md)
