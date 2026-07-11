# AGENTS.md — Bomb Code

## Project

**Bomb Code** — production-oriented **Rust + Tauri 2** desktop control panel for **Grok Build** (xAI agentic coding CLI).

Primary integration path: **ACP** (`grok agent stdio`, JSON-RPC over NDJSON stdio).  
Fallback: headless CLI (`grok -p ...`) for batch/scheduler jobs.

## Architecture (crates)

| Crate | Role |
|-------|------|
| `grok_config` | Paths, TOML config, sandbox profiles |
| `grok_cli_wrapper` | Typed async `grok` CLI wrapper |
| `grok_acp` | ACP client (initialize → auth → session/new → prompt + event loop) |
| `grok_events` | Broadcast event bus for UI + services |
| `grok_control_core` | SessionRegistry, multi-session orchestration |
| `grok_worktree` | Git/Grok worktree lifecycle |
| `grok_permissions` | Allow/deny rules, presets, sandbox policy |
| `grok_extensions` | Skills / plugins CRUD (MCP legacy helpers) |
| `grok_mcp` | Full MCP manager: catalog (7 servers), CRUD, doctor, credentials, session injection |
| `grok_memory` | Cross-session memory + flush/dream |
| `grok_scheduler` | Interval/cron/once jobs |
| `grok_persistence` | SQLite crash recovery + transcripts |
| `grok_diff` | Diff capture / summaries |
| `src-tauri` | Tauri app, invoke commands, event bridge |

## Build commands

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo tauri dev   # full desktop UI
```

## Security defaults

- **Never** default to `--always-approve` / yolo.
- Plan mode on by default.
- Validate absolute `cwd`, extension names, and prompts at boundaries.
- Do not log `XAI_API_KEY`.
- Deny-first permission evaluation; presets in `grok_permissions`.

## Agent conventions

- Prefer ACP sessions for interactive work.
- Use headless mode only for scheduler/batch routines.
- Keep crates small and cohesive; expand sketches rather than inventing parallel APIs.
- After each phase: `cargo check` + `cargo clippy`, then commit.
- Record waves/fixes in `IMPLEMENTATION_LOG.md`.

## Multi-agent build process

This repo was built with the plan in `docs/plan/`:

1. Planning wave  
2. Implementation wave  
3. Audit wave  
4. Revise wave (loop until zero Critical/High)

Orchestrator role owns phase gates and the implementation log.
