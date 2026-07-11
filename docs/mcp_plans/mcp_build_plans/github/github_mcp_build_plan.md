# Detailed Build Plan: GitHub MCP Server Integration

**MCP Server:** GitHub (official or community http MCP)  
**Type:** http  
**Priority:** High

## 1. Purpose
Allow Grok Build agents to interact with GitHub (create issues, PRs, comment, manage repos) directly from within coding sessions.

## 2. Integration Points
- Config & Extensions Browser → "GitHub MCP" section with OAuth flow
- Session Orchestrator → Auto-attach if repo is GitHub-linked
- Event Bus → Emit events when PRs/issues are created via agent

## 3. Implementation Steps

### Configuration
- Support OAuth (recommended) or Personal Access Token
- Store credentials securely via Grok's built-in `~/.grok/mcp_credentials.json`
- Project-level linking: Detect `.git` remote and suggest GitHub MCP

### Features to Expose
- Create PR from current branch + plan diff
- Comment on issues/PRs
- List open issues/PRs in repo
- Update issue status

### UI/Backend
- "Connect GitHub" button → triggers browser OAuth
- Per-project GitHub repo mapping
- Quick actions in session view: "Create PR for this change"

## 4. Security
- Scope tokens minimally (repo scope only)
- Never store tokens in plain config — use Grok's credential store

## 5. Testing
- End-to-end: Agent creates a test PR via MCP tool

**Dependencies:** OAuth handling (already in Grok Build)

**Priority:** High for developer workflows
