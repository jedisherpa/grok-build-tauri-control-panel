# Detailed Build Plan: Linear MCP Server Integration

**MCP Server:** Linear (http)  
**Type:** http  
**Priority:** High for project management workflows

## Purpose
Link coding sessions directly to Linear issues and cycles. Agents can create/update issues, add comments, and associate work with tasks.

## Key Integration Points
- Auto-detect Linear issue IDs in commit messages or branch names
- Quick actions: "Link current session to Linear issue #XXX"
- Display linked issues in session sidebar

## Implementation
- Add via `grok mcp add --transport http linear https://mcp.linear.app/mcp`
- Handle Linear OAuth
- Expose tools: `linear_create_issue`, `linear_update_issue`, `linear_comment`, `linear_search_issues`

## UI Features
- Linear integration settings page
- Per-project Linear team/workspace mapping
- "Create Linear issue from plan" button

## Security
- Use official Linear MCP with proper OAuth scopes

**Dependencies:** None special
