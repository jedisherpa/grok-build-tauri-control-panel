# Bomb Code

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-edition%202021-orange.svg)](./Cargo.toml)
[![Tauri](https://img.shields.io/badge/tauri-2-blue.svg)](https://tauri.app)

**Bomb Code** is an open-source **Tauri 2** desktop control panel for [Grok Build](https://x.ai) — multi-session agent orchestration with **ACP-first** integration, worktrees, permissions, MCP/skills, memory, scheduler, and crash recovery.

> **Requires** the [Grok Build CLI](https://x.ai) (`grok`) installed and authenticated. This panel does not ship the Grok binary.

## Features

- **ACP client** (`grok agent stdio`) — long-lived interactive sessions
- **Non-blocking prompts** — long coding turns stream via notifications (no ~120s UI timeout)
- **Headless CLI** fallback for batch / scheduled jobs
- **Multi-session registry** with concurrent `DashMap` access
- **Git worktree** isolation for parallel agents
- **Interactive tool approvals** (allow once / always / deny per request), deny rules enforced ahead of yolo, permission presets (safe / workspace / yolo) + sandbox profiles
- **Thread-per-worktree isolation** — each new thread in a git project gets its own worktree + `thread/<id>` branch (pure git, works with every backend), grouped by project in the sidebar. **Land** merges a thread back into the project branch; conflicts route through **Sync**, which pulls main into the worktree so the thread's own agent can resolve them. Threads get smart names from their first prompt.
- **MCP management** — catalog (filesystem, GitHub, Linear, X, Playwright, custom), doctor, credentials store, pre-spawn health checks, session attachment. Servers needing credentials (e.g. `GITHUB_TOKEN`, `LINEAR_API_KEY`, `X_API_BEARER`) are skipped with a visible reason until the secret is set.
- **Extensions** — skills, plugins CRUD (config + CLI)
- **Memory** — structured store + MEMORY.md flush/dream, cited local keyword/vector recall, and source-linked meaning profiles
- **Scheduler** — interval, cron, one-shot routines (persisted; survive restart; each job needs an explicit working directory)
- **Persistence** — SQLite session/transcript recovery
- **Diff engine** — before/after capture and summaries
- **Live Dev Server** dock for project preview
- **macOS app** install under `/Applications/Bomb Code.app`

## Quick start

### Prerequisites

1. [Rust](https://rustup.rs/) (stable) + Xcode CLT on macOS
2. [Tauri 2 CLI](https://v2.tauri.app/start/prerequisites/): `cargo install tauri-cli --version "^2"`
3. Grok Build CLI on `PATH` or at `~/.grok/bin/grok`
4. Grok auth (`grok` login / panel Login)

### Install from source (macOS)

```bash
git clone https://github.com/jedisherpa/grok-build-tauri-control-panel.git
cd grok-build-tauri-control-panel
./scripts/install.sh --development --destination "/Applications/Bomb Code Dev.app"
# Destination must be new. This preserves the built signature and does not launch.
```

Later launches:

```bash
./scripts/run.sh
# or
open "/Applications/Bomb Code Dev.app"
```

See **[QUICKSTART.md](./QUICKSTART.md)** for first ACP session, MCP setup, and config paths.

Production signing/notarization and iterative feature qualification are tracked in
[the release status](docs/release/QUALIFICATION_REPORT.md) and
[the macOS distribution procedure](docs/release/MACOS_DISTRIBUTION.md).
The developer installer does not replace an existing application or certify a release.

### Develop

```bash
./scripts/run.sh --dev
# or
cargo tauri dev
cargo tauri build --bundles app
```


### Develop on Linux

System libraries (Debian/Ubuntu):

```bash
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev libayatana-appindicator3-dev \
  librsvg2-dev libgtk-3-dev libssl-dev pkg-config build-essential
cargo install tauri-cli --version "^2"
```

Then from the repo root:

```bash
# Serve the static frontend (Tauri devUrl is http://localhost:1420)
(cd frontend && python3 -m http.server 1420 --bind 127.0.0.1) &
CARGO_BUILD_JOBS=4 cargo build -p grok-build-control-panel
./target/debug/grok-build-control-panel
# or: cargo tauri dev
```

Install the [Grok Build CLI](https://x.ai/cli) (`curl -fsSL https://x.ai/cli/install.sh | bash`) and set `XAI_API_KEY`, or use **Log in with Grok** in Services. This panel does not ship the `grok` binary.


The app discovers `~/.grok/bin/grok` even when launched from Finder (PATH is bootstrapped).

## Meaning memory

Memory can search source-linked meanings alongside its existing keyword/vector
recall. Choose a dictionary definition, inspect the unselected source-linked
passages, and prepare a passage in Joe when you want it interpreted. Preparation,
lookup, saved-profile import and comparison stay local. The existing explicit
**Analyze** action sends the displayed passage/context to the configured provider.

Successful Joe analyses accumulate private immutable profile references to their
original receipts. Exact cited passage profiles remain distinct from questions
analyzed with historical context. Saved analyses can be imported locally. Compare
profiles to inspect selected senses, asserted concepts, scoped context, event
roles/negation/conditions, references and native root/fine positions separately.
Clarification drafts remain unsent. Coverage shows how much interpreted evidence
has accumulated; this release does not assign a combined relevance/confidence
score.

The feature requires the existing local Wizard Joe reference/runtime and a current
recall index for cited memories. Missing or changed evidence is displayed as
unavailable. Raw histories, indexes, profiles and provider credentials are not
shipped in the repository. See [integration guide](docs/cdiss/MEANING_MEMORY_INTEGRATION.md),
[integration plan](docs/plan/meaning_memory.md) and
[full-chain research](docs/cdiss/FULL_CHAIN_RETRIEVAL_RESULTS.md).

## Config locations

| Path | Purpose |
|------|---------|
| `~/.grok/control-panel/config.toml` | Panel settings only |
| `~/.grok/config.toml` | Grok CLI config (**never overwritten** by panel) |
| `~/.grok/mcp_credentials.json` | MCP secrets (mode `0600`) |
| `~/.grok/control-panel/sessions/` | Panel SQLite recovery DB |

## Workspace layout

```
crates/           # backend libraries
src-tauri/        # Tauri host + commands
frontend/         # lightweight control UI
docs/plan/        # original multi-agent build plan
scripts/          # install / run helpers
```

## Security notes

- No API keys or credentials are committed to this repository.
- Secrets live under `~/.grok/` with restricted permissions.
- Prefer **plan mode** for untrusted repos; use **yolo** only when you accept the risk.
- Do not log `XAI_API_KEY` or MCP tokens.

## Contributing

Issues and PRs welcome. Prefer conventional commits (`feat:`, `fix:`, `docs:`, …). Keep changes focused; run `cargo test` and `cargo clippy` before submitting.

## License

[MIT](./LICENSE) © 2026 Bomb Code contributors
