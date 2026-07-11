//! Comprehensive MCP server management for the Grok Build control panel.
//!
//! Shared infrastructure + catalog presets for:
//! Filesystem, GitHub, Linear, X/Twitter, Browser/Playwright,
//! grok-build-mcp wrappers, and Custom Internal Tools.

mod catalog;
mod credentials;
mod injection;
mod manager;
mod security;
mod types;

pub use catalog::{McpCatalogEntry, McpServerKind, builtin_catalog, catalog_entry};
pub use credentials::{CredentialStore, McpCredential};
pub use injection::{McpAttachment, build_session_mcp_payload, resolve_attachments};
pub use manager::{DoctorReport, DoctorStatus, McpManager, McpToolInfo};
pub use security::{SecurityVerdict, validate_custom_server, validate_filesystem_paths};
pub use types::{
    AddMcpRequest, McpScope, McpServerConfigExt, McpTransport, UpdateMcpRequest,
};
