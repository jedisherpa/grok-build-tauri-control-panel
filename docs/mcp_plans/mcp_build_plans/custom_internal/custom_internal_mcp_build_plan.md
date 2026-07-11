# Detailed Build Plan: Custom Internal Tools MCP Framework

**MCP Server Type:** Generic (Either stdio or http)  
**Priority:** Foundational for extensibility

## Purpose
Provide a generic framework so users/companies can easily add their own internal MCP servers (company APIs, databases, CI systems, internal tools).

## Key Features to Build
- Generic "Add Custom MCP Server" form
- Support for both stdio and http transports
- Template generators for common patterns (REST API wrapper, database MCP, etc.)
- Validation and doctor support for custom servers
- Documentation generator for new MCP servers

## Architecture
- Extend `McpServerConfig` to be fully generic
- Add "Custom" category in Config & Extensions Browser
- Provide example templates in the repo (e.g., "Internal API MCP template")

## Security
- Strong input validation on command/URL
- Warning banner for custom stdio servers
- Option to run custom servers in stricter sandbox profiles

## Benefits
Makes the control panel future-proof and highly extensible for enterprise use cases.

**Dependencies:** None — this is the extensibility layer
