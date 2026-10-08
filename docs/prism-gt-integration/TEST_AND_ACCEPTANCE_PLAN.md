# Test and Acceptance Plan

This document defines the testing strategy and acceptance criteria for the `jedisherpa/grok-build-tauri-control-panel` repository at the frozen commit `994e1e39f39119998bb9a4a3c047128375eb7067`.

## Frozen Source Identity
- **Repository:** `jedisherpa/grok-build-tauri-control-panel`
- **Base Branch:** `main`
- **Frozen Commit:** `994e1e39f39119998bb9a4a3c047128375eb7067`

## Observed Baseline Commands

The following commands are observed in the repository instructions:
```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo tauri dev
node frontend/presence.test.mjs
```
*Note*: `cargo tauri dev` is an observed development command that requires a desktop environment and Grok CLI installed; it is not safe for headless verification. `node frontend/presence.test.mjs` is an observed repository-local command from the planning/UX implementation log and file inventory, but was not executed in this package.

## Proposed Test Matrix

To ensure robust integration with the Prism GT ecosystem, the following test matrix is proposed for future implementation:
1. **ACP Integration Tests**: Mock the `grok agent stdio` JSON-RPC responses to verify the `grok_acp` client handles initialization, tool calls, and streaming updates correctly.
2. **MCP Doctor Checks**: Execute `grok mcp doctor` equivalents via the `grok_mcp` crate to validate the health and credential resolution of configured MCP servers.
3. **Frontend Presence Logic**: Run `node frontend/presence.test.mjs` to verify the state machine transitions (idle -> send -> think -> tools -> reply -> done) and stall detection logic.
4. **Concurrency Tests**: Simulate high-load multi-session environments to verify synchronization implementations in `grok_control_core` do not deadlock.

## Negative and Security Checks

- **Credential Leakage**: Verify that `XAI_API_KEY` and MCP tokens are never logged to stdout, stderr, or persistent storage. (Planned evidence artifacts: grep/source review paths, command outputs, log locations. *Not verified from supplied evidence*).
- **Permission Enforcement**: Ensure that the `grok_permissions` crate strictly enforces a deny-first policy and that `--always-approve` cannot be set as a default. (Planned evidence artifacts: exact source symbols to inspect. *Not verified from supplied evidence*).
- **Path Traversal**: Validate absolute `cwd` and extension names at boundaries to prevent path traversal attacks. (*Not verified from supplied evidence*).

## Evidence Capture and Acceptance Gates

If a future CI or publisher-run verification is configured, capture logs; no CI workflow or CI logs were evidenced in the frozen packet.

**Observed results:** None provided in this package.

The acceptance gate for advancing the package requires zero blocking findings and zero high-severity security findings from the independent auditor.

## Frozen source boundary

This document is limited to the exact repository, branch, and frozen source commit recorded in the package metadata and cited references. Later remote changes require a new source freeze and independent review.

## References

1. [Agents Security Defaults](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/AGENTS.md)
2. [Frontend Presence Tests](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/frontend/presence.test.mjs)
3. [Security Advisory (Planning Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/docs/plan/advisories/multi_perspective_advisory_core_architecture.md)
