# Agent Handoff

This document provides instructions and context for the next agent in the publication workflow for the `jedisherpa/grok-build-tauri-control-panel` repository.

## Exact Source Freeze

- **Repository**: `jedisherpa/grok-build-tauri-control-panel`
- **Branch**: `main`
- **Commit**: `994e1e39f39119998bb9a4a3c047128375eb7067`

## Safe Start Commands

The following commands are observed repository commands for a trusted verifier/owner-controlled environment; they were not executed during evidence audit:
```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
*Warning*: Running tests/builds executes repository code and should only be done in an appropriate sandbox. The command `cargo tauri dev` requires a desktop environment and the external `grok` CLI installed; it should not be run in headless CI environments without proper mocking.

## Package Map

The canonical package consists of exactly the following 11 target paths:
1. `docs/prism-gt-integration/README.md`
2. `docs/prism-gt-integration/CURRENT_STATE.md`
3. `docs/prism-gt-integration/ROLE_IN_ECOSYSTEM.md`
4. `docs/prism-gt-integration/TARGET_ARCHITECTURE.md`
5. `docs/prism-gt-integration/INTEGRATION_CONTRACTS.md`
6. `docs/prism-gt-integration/IMPLEMENTATION_PLAN.md`
7. `docs/prism-gt-integration/TEST_AND_ACCEPTANCE_PLAN.md`
8. `docs/prism-gt-integration/RISKS_AND_OPEN_QUESTIONS.md`
9. `docs/prism-gt-integration/AGENT_HANDOFF.md`
10. `docs/prism-gt-integration/implementation-manifest.yaml`
11. `docs/prism-gt-integration/generation-metadata.json`

## Prohibited Actions

- **Do not modify application code**: No changes may be made to `crates/*`, `src-tauri/`, `frontend/`, or any other source files outside the `docs/prism-gt-integration/` directory.
- **Do not invent contracts**: All cross-repository interfaces must remain in a `proposed` state unless explicit owner approval is evidenced.
- **Do not execute untrusted code**: Do not run the application or MCP servers during the documentation phase.

## Remaining Work and Done Conditions

- **Remaining Work**: The publisher agent must create a documentation-only draft pull request containing exactly these 11 files. The remote verifier must then confirm the exact head and branch state.
- **Done Conditions**: The repository transitions from `documentation_pending` to published only when the canonical package, branch, draft PR, exact remote head, changed-file scope, canonical manifest entry, publication ledger, and independent remote verification all pass. This package alone does not change status.

## References

1. [Agents Documentation](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/AGENTS.md)
2. [Cargo Workspace](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/Cargo.toml)
3. [Prism GT Integration Guide (Preparation Evidence)](https://github.com/jedisherpa/grok-build-tauri-control-panel/blob/994e1e39f39119998bb9a4a3c047128375eb7067/README.PRISM-GT-INTEGRATION.md)
