# Local primary coding interface — 2026-10-06

Bomb Code 0.1.1 keeps the existing Tauri ACP cockpit and adds a local History reference library. The central workspace presents the AI activity explanation above conversation and decisions; technical tool, terminal, and thought rows appear in the narrower right inspector. Approvals and the composer remain in the main workspace.

Grok runs its native `agent stdio` endpoint. Claude and Codex run their own coding engines through pinned ACP adapters, rather than being model providers inside the Grok engine. `CLAUDE_CODE_EXECUTABLE` and `CODEX_PATH` select the native executables; login status uses those same paths even when Finder omits nvm from PATH. Current adapters configure the selected model through `session/set_config_option` when advertised.

The History bridge embeds `scripts/history_library.py` and uses local Python 3 and SQLite FTS5. It reads Codex active/archived JSONL, Claude Code project/subagent JSONL and desktop session metadata, Grok JSONL, and saved zsh commands. Source records are never modified. The derived database is separate from live Bomb Code sessions and approvals, with private file permissions. Source IDs, paths, coverage, skipped-record receipts, pagination, and subagent separation are visible. `Scan local histories` refreshes changed local files; it does not perform ongoing cloud synchronization.

`Import ChatGPT / Claude export` accepts recognized JSON exports or ZIPs containing `conversations.json`; archives are read without extracting paths. ChatGPT exports retain all branches, displayed by timestamp. Text excerpts are capped and labelled; binary attachments, tool payloads, and hidden reasoning remain in their original sources. Saved shell commands cannot recover terminal output that was never recorded. Credential-shaped shell values receive best-effort redaction.

`Draft a new coding thread` creates only a bounded composer draft in Plan mode. It explicitly marks imported text as historical reference, asks for the next task and project, and never automatically sends it or treats earlier approvals as current authorization. Cross-provider history reading is not native agent-session resumption.

Validation: Python migration fixtures cover unchanged originals, idempotency, changed files, namespaces, subagent isolation, malformed records, all export branches, ZIP traversal avoidance, literal FTS queries, hidden-content exclusion, terminal redaction, and metadata-only scope. Rust workspace tests, strict Clippy, JS syntax checks, release build, provider connectivity, and installed UI smoke checks are recorded in the setup report.

This local adaptation does not implement Cloud World governance, Ratchet, Orchestrator, War Room integration, or external scheduling. The supplied integration document is ecosystem context. Those wider interfaces remain separate projects and need their own explicit implementation work.
