# MCP Setup Examples

This control panel manages MCP servers via the `grok_mcp` crate and Tauri commands.
Secrets live in `~/.grok/mcp_credentials.json` (mode 0600), not in `config.toml`.

## Catalog (7 servers)

| ID | Transport | Notes |
|----|-----------|--------|
| `filesystem` | stdio | `npx -y @modelcontextprotocol/server-filesystem <paths>` |
| `github` | stdio | needs `GITHUB_TOKEN` |
| `linear` | http | `https://mcp.linear.app/mcp` |
| `x_twitter` | http | `https://api.x.com/mcp` — requires approval |
| `browser` | stdio | Playwright — **high risk**, approval required |
| `grok_build` | stdio | self-delegate wrappers — rate limited |
| `custom` | stdio/http | generic internal tools |

## Add filesystem MCP

```js
await invoke("add_mcp_server", {
  request: {
    name: "docs-fs",
    fromCatalog: "filesystem",
    allowedPaths: ["/Users/you/Documents/safe-docs"],
    readOnly: true,
    enabled: true,
    scope: "project",
  },
});
```

## Add GitHub MCP

```js
await invoke("set_mcp_credential", { key: "GITHUB_TOKEN", value: "ghp_..." });
await invoke("add_mcp_server", {
  request: {
    name: "github",
    fromCatalog: "github",
    autoAttach: true,
    enabled: true,
  },
});
```

## Add Linear (HTTP)

```js
await invoke("add_mcp_server", {
  request: { name: "linear", fromCatalog: "linear", enabled: true },
});
```

## Attach MCP when spawning a session

```js
await invoke("start_session", {
  cwd: "/absolute/path/to/repo",
  opts: {
    mode: "acp",
    planMode: true,
    alwaysApprove: false,
    mcpServerNames: ["docs-fs", "github"],
    approvedHighRiskMcp: [], // e.g. ["playwright"] if attaching browser
    includeAutoMcp: true,
    mcpServers: [],
    rules: [],
    permissionAllow: [],
    permissionDeny: [],
    trustRepo: false,
  },
});
```

High-risk servers (`browser`, `grok_build`, `custom`, `x` when flagged) are **not** attached unless listed in `approvedHighRiskMcp`.

## Doctor

```js
await invoke("doctor_mcp_server", { name: null }); // all
await invoke("doctor_mcp_server", { name: "github" });
```

## CLI parity

When `grok` is installed, the manager also mirrors:

- `grok mcp list`
- `grok mcp add …`
- `grok mcp remove …`
- `grok mcp doctor` (best-effort)
