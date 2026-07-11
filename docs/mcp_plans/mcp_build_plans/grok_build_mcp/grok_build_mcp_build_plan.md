# Detailed Build Plan: grok-build-mcp Wrappers Integration

**MCP Server:** Various grok-build-mcp / grok-mcp wrappers (stdio)  
**Type:** stdio  
**Priority:** High (Self-referential / Meta)

## Purpose
Allow the control panel's agents to delegate work **back to Grok Build itself** as a specialized sub-agent (e.g., for code review, challenge, or extended tasks).

## Key Tools Typically Exposed
- `grok_chat`
- `grok_review` (structured diff review)
- `grok_challenge` (find bugs, security issues)
- `grok_consult` (multi-turn)

## Integration Strategy
- Add as a special "Internal Grok Delegate" MCP
- When enabled, agents can call Grok Build tools without leaving the session
- Useful for multi-model workflows inside one panel

## Implementation
- Support both CLI-based and direct API-based wrappers
- Add configuration for which Grok model to use for sub-calls
- Expose in tool palette as `grok_build__*`

## Security
- Since it spawns Grok Build, inherit existing sandbox + permission model
- Add rate limiting to prevent recursive explosion

## Value
Enables powerful self-referential and multi-agent patterns inside the control panel.

**Dependencies:** Grok Build CLI installed (already required)
