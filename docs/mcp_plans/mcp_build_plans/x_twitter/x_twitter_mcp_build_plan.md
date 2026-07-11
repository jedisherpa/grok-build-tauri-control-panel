# Detailed Build Plan: X / Twitter (Official) MCP Server Integration

**MCP Server:** Official X MCP (http)  
**Type:** http  
**Priority:** Medium-High (research + content)

## Purpose
Give agents live access to X (Twitter) for research, trend monitoring, and drafting posts directly from the control panel.

## Key Features
- Real-time X search
- Bookmark management
- Draft and publish Articles
- Fetch trends/news

## Integration
- Dedicated "X Research" panel in the control panel
- When agent needs current events or social proof, it can use `x__search_posts`
- Support for posting drafts created during coding sessions

## Implementation Notes
- Uses official hosted MCP: `https://api.x.com/mcp` (or similar)
- OAuth via X Developer Portal + Grok's credential system
- Add "X Integration" section in Config browser

## Security
- Minimal scopes (read + draft only where possible)

**Use Cases:**
- Research latest discussions on a library
- Draft announcement posts for new features
- Monitor mentions of the project
