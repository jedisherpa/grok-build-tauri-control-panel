# Bomb Code

**Bomb Code** is a Rust **Tauri 2** desktop control panel for [Grok Build](https://x.ai) — multi-session agent orchestration with **ACP-first** integration, worktrees, permissions, MCP/skills, memory, scheduler, and crash recovery.

## Features

- **ACP client** (`grok agent stdio`) — long-lived interactive sessions
- **Headless CLI** fallback for batch / scheduled jobs
- **Multi-session registry** with concurrent `DashMap` access
- **Git worktree** isolation for parallel agents
- **Permission presets** (safe / workspace / yolo) + sandbox profiles
- **MCP management** — 7-server catalog (filesystem, GitHub, Linear, X, Playwright, grok-build, custom), doctor, credentials store, session attachment
- **Extensions** — skills, plugins CRUD (config + CLI)
- **Memory** — structured store + MEMORY.md flush/dream
- **Scheduler** — interval, cron, one-shot routines
- **Persistence** — SQLite session/transcript recovery
- **Diff engine** — before/after capture and summaries
- **Minimal frontend** — sessions, worktrees, extensions, memory, scheduler, system

## Quick start (use it now)

With Grok Build CLI already installed:

```bash
cd ~/grok-build-tauri-control-panel
./scripts/install.sh   # builds + installs to /Applications + opens
# later:
./scripts/run.sh
```

See **[QUICKSTART.md](./QUICKSTART.md)** for first ACP session, MCP setup, and config paths.

### Develop / rebuild

```bash
./scripts/run.sh --dev
# or
cargo tauri dev
cargo tauri build --bundles app
```

The app discovers `~/.grok/bin/grok` even when launched from Finder (PATH is bootstrapped).

## Workspace layout

```
crates/           # backend libraries
src-tauri/        # Tauri host + commands
frontend/         # lightweight control UI
docs/plan/        # original multi-agent build plan
AGENTS.md         # agent operating notes
IMPLEMENTATION_LOG.md
```

## Plan provenance

Scaffolded and executed from `docs/plan/` (multi-phase multi-wave agent swarm plan). See `IMPLEMENTATION_LOG.md` for phase-by-phase audit/revise history.

## License

MIT
