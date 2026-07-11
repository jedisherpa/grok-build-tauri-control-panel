# Detailed Build Plan: Browser / Playwright MCP Server Integration

**MCP Server:** Playwright or Browser MCP (stdio)  
**Type:** stdio  
**Priority:** Medium (testing & scraping)

## Purpose
Enable agents to automate browsers for UI testing, documentation scraping, and visual verification.

## Integration Points
- Session Orchestrator → Option to attach browser MCP for testing sessions
- Tool exposure: `browser_navigate`, `browser_click`, `browser_screenshot`, `browser_evaluate`
- Diff viewer integration: Compare screenshots

## Implementation
- Recommend `@modelcontextprotocol/server-playwright` or similar
- Add safety: Headless mode by default, limited permissions
- UI: "Enable Browser Automation" toggle per session

## Security Considerations (Critical)
- High risk server (can control browser)
- Require explicit user approval
- Sandbox the Playwright process
- Limit to specific domains if possible

## Testing
- Agent performs a simple UI test and returns screenshot + result

**Dependencies:** Playwright installed on host
