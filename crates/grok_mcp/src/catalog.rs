//! Built-in catalog for the 7 MCP server integrations.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::{McpScope, McpServerConfigExt, McpTransport};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpServerKind {
    Filesystem,
    Github,
    Linear,
    XTwitter,
    Browser,
    GrokBuild,
    Custom,
}

impl McpServerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Filesystem => "filesystem",
            Self::Github => "github",
            Self::Linear => "linear",
            Self::XTwitter => "x_twitter",
            Self::Browser => "browser",
            Self::GrokBuild => "grok_build",
            Self::Custom => "custom",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "filesystem" | "fs" => Self::Filesystem,
            "github" | "gh" => Self::Github,
            "linear" => Self::Linear,
            "x" | "twitter" | "x_twitter" | "x-twitter" => Self::XTwitter,
            "browser" | "playwright" => Self::Browser,
            "grok_build" | "grok-build" | "grok_build_mcp" => Self::GrokBuild,
            _ => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCatalogEntry {
    pub id: String,
    pub kind: McpServerKind,
    pub title: String,
    pub description: String,
    pub transport: McpTransport,
    pub default_name: String,
    pub high_risk: bool,
    pub requires_approval: bool,
    pub credential_keys: Vec<String>,
    pub docs_url: Option<String>,
    pub example_tools: Vec<String>,
    /// Template used by `instantiate`.
    pub template: McpServerConfigExt,
}

/// Full built-in catalog (all 7 integrations).
pub fn builtin_catalog() -> Vec<McpCatalogEntry> {
    vec![
        filesystem_entry(),
        github_entry(),
        linear_entry(),
        x_twitter_entry(),
        browser_entry(),
        grok_build_entry(),
        custom_entry(),
    ]
}

pub fn catalog_entry(id: &str) -> Option<McpCatalogEntry> {
    builtin_catalog()
        .into_iter()
        .find(|e| e.id == id || e.kind.as_str() == id)
}

fn filesystem_entry() -> McpCatalogEntry {
    McpCatalogEntry {
        id: "filesystem".into(),
        kind: McpServerKind::Filesystem,
        title: "Filesystem".into(),
        description: "Secure read/write access to approved directories outside the project via @modelcontextprotocol/server-filesystem".into(),
        transport: McpTransport::Stdio,
        default_name: "filesystem".into(),
        high_risk: false,
        requires_approval: false,
        credential_keys: vec![],
        docs_url: Some("https://github.com/modelcontextprotocol/servers".into()),
        example_tools: vec![
            "read_file".into(),
            "write_file".into(),
            "list_directory".into(),
            "search_files".into(),
        ],
        template: McpServerConfigExt {
            name: "filesystem".into(),
            transport: McpTransport::Stdio,
            command: Some("npx".into()),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
            ],
            kind: "filesystem".into(),
            description: Some("Filesystem MCP".into()),
            startup_timeout_sec: 90,
            tool_timeout_sec: 60,
            read_only: false,
            scope: McpScope::Project,
            auto_attach: false,
            ..Default::default()
        },
    }
}

fn github_entry() -> McpCatalogEntry {
    let mut env = HashMap::new();
    env.insert("GITHUB_PERSONAL_ACCESS_TOKEN".into(), "${GITHUB_TOKEN}".into());
    McpCatalogEntry {
        id: "github".into(),
        kind: McpServerKind::Github,
        title: "GitHub".into(),
        description: "Issues, PRs, comments, and repo operations via GitHub MCP".into(),
        transport: McpTransport::Stdio,
        default_name: "github".into(),
        high_risk: false,
        requires_approval: false,
        credential_keys: vec!["GITHUB_TOKEN".into()],
        docs_url: Some("https://github.com/github/github-mcp-server".into()),
        example_tools: vec![
            "create_pull_request".into(),
            "create_issue".into(),
            "list_issues".into(),
            "add_issue_comment".into(),
        ],
        template: McpServerConfigExt {
            name: "github".into(),
            transport: McpTransport::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            env,
            kind: "github".into(),
            description: Some("GitHub MCP".into()),
            credential_keys: vec!["GITHUB_TOKEN".into()],
            auto_attach: true,
            scope: McpScope::Project,
            startup_timeout_sec: 60,
            ..Default::default()
        },
    }
}

fn linear_entry() -> McpCatalogEntry {
    McpCatalogEntry {
        id: "linear".into(),
        kind: McpServerKind::Linear,
        title: "Linear".into(),
        description: "Link sessions to Linear issues; create/update/comment via official Linear MCP".into(),
        transport: McpTransport::Http,
        default_name: "linear".into(),
        high_risk: false,
        requires_approval: false,
        credential_keys: vec!["LINEAR_API_KEY".into()],
        docs_url: Some("https://linear.app/docs".into()),
        example_tools: vec![
            "linear_create_issue".into(),
            "linear_update_issue".into(),
            "linear_comment".into(),
            "linear_search_issues".into(),
        ],
        template: McpServerConfigExt {
            name: "linear".into(),
            transport: McpTransport::Http,
            url: Some("https://mcp.linear.app/mcp".into()),
            kind: "linear".into(),
            description: Some("Linear MCP (HTTP)".into()),
            credential_keys: vec!["LINEAR_API_KEY".into()],
            auto_attach: false,
            scope: McpScope::Global,
            startup_timeout_sec: 30,
            ..Default::default()
        },
    }
}

fn x_twitter_entry() -> McpCatalogEntry {
    McpCatalogEntry {
        id: "x_twitter".into(),
        kind: McpServerKind::XTwitter,
        title: "X / Twitter".into(),
        description: "Official X MCP for search, trends, bookmarks, and draft posts".into(),
        transport: McpTransport::Http,
        default_name: "x".into(),
        high_risk: false,
        requires_approval: true,
        credential_keys: vec!["X_API_BEARER".into()],
        docs_url: Some("https://docs.x.com".into()),
        example_tools: vec![
            "x_search_posts".into(),
            "x_get_trends".into(),
            "x_draft_post".into(),
            "x_bookmarks".into(),
        ],
        template: McpServerConfigExt {
            name: "x".into(),
            transport: McpTransport::Http,
            url: Some("https://api.x.com/mcp".into()),
            kind: "x_twitter".into(),
            description: Some("X/Twitter official MCP".into()),
            credential_keys: vec!["X_API_BEARER".into()],
            requires_approval: true,
            auto_attach: false,
            scope: McpScope::Global,
            startup_timeout_sec: 30,
            ..Default::default()
        },
    }
}

fn browser_entry() -> McpCatalogEntry {
    McpCatalogEntry {
        id: "browser".into(),
        kind: McpServerKind::Browser,
        title: "Browser / Playwright".into(),
        description: "Headless browser automation for UI testing and scraping (high risk)".into(),
        transport: McpTransport::Stdio,
        default_name: "playwright".into(),
        high_risk: true,
        requires_approval: true,
        credential_keys: vec![],
        docs_url: Some("https://github.com/microsoft/playwright-mcp".into()),
        example_tools: vec![
            "browser_navigate".into(),
            "browser_click".into(),
            "browser_screenshot".into(),
            "browser_evaluate".into(),
        ],
        template: McpServerConfigExt {
            name: "playwright".into(),
            transport: McpTransport::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into(), "@playwright/mcp@latest".into()],
            kind: "browser".into(),
            description: Some("Playwright browser MCP (headless by default)".into()),
            high_risk: true,
            requires_approval: true,
            auto_attach: false,
            scope: McpScope::Session,
            startup_timeout_sec: 120,
            tool_timeout_sec: 180,
            env: {
                let mut e = HashMap::new();
                e.insert("PLAYWRIGHT_HEADLESS".into(), "1".into());
                e
            },
            ..Default::default()
        },
    }
}

fn grok_build_entry() -> McpCatalogEntry {
    let mut env = HashMap::new();
    env.insert("XAI_API_KEY".into(), "${XAI_API_KEY}".into());
    McpCatalogEntry {
        id: "grok_build".into(),
        kind: McpServerKind::GrokBuild,
        title: "Grok Build Delegate".into(),
        description: "Self-referential MCP wrappers for grok_chat, grok_review, grok_challenge, grok_consult".into(),
        transport: McpTransport::Stdio,
        default_name: "grok-build".into(),
        high_risk: true,
        requires_approval: true,
        credential_keys: vec!["XAI_API_KEY".into()],
        docs_url: None,
        example_tools: vec![
            "grok_chat".into(),
            "grok_review".into(),
            "grok_challenge".into(),
            "grok_consult".into(),
        ],
        template: McpServerConfigExt {
            name: "grok-build".into(),
            transport: McpTransport::Stdio,
            command: Some("grok".into()),
            args: vec!["mcp", "serve", "build-tools"]
                .into_iter()
                .map(String::from)
                .collect(),
            env,
            kind: "grok_build".into(),
            description: Some("Internal Grok Build delegate MCP".into()),
            high_risk: true,
            requires_approval: true,
            rate_limit_per_min: Some(10),
            auto_attach: false,
            scope: McpScope::Session,
            startup_timeout_sec: 60,
            credential_keys: vec!["XAI_API_KEY".into()],
            ..Default::default()
        },
    }
}

fn custom_entry() -> McpCatalogEntry {
    McpCatalogEntry {
        id: "custom".into(),
        kind: McpServerKind::Custom,
        title: "Custom Internal Tools".into(),
        description: "Generic framework for company APIs, databases, CI systems, and internal MCP servers".into(),
        transport: McpTransport::Stdio,
        default_name: "custom".into(),
        high_risk: true,
        requires_approval: true,
        credential_keys: vec![],
        docs_url: None,
        example_tools: vec!["custom_tool".into()],
        template: McpServerConfigExt {
            name: "custom".into(),
            transport: McpTransport::Stdio,
            command: Some("npx".into()),
            args: vec![],
            kind: "custom".into(),
            description: Some("Custom internal MCP".into()),
            high_risk: true,
            requires_approval: true,
            scope: McpScope::Project,
            startup_timeout_sec: 60,
            ..Default::default()
        },
    }
}

impl McpCatalogEntry {
    /// Instantiate a concrete server config from this catalog template.
    pub fn instantiate(
        &self,
        name: Option<String>,
        allowed_paths: Vec<PathBuf>,
        overrides: Option<McpServerConfigExt>,
    ) -> McpServerConfigExt {
        let mut cfg = overrides.unwrap_or_else(|| self.template.clone());
        if let Some(n) = name {
            cfg.name = n;
        } else if cfg.name.is_empty() {
            cfg.name = self.default_name.clone();
        }
        cfg.kind = self.kind.as_str().into();
        cfg.high_risk = self.high_risk;
        cfg.requires_approval = self.requires_approval;
        if !allowed_paths.is_empty() {
            cfg.allowed_paths = allowed_paths.clone();
            if self.kind == McpServerKind::Filesystem {
                // Append paths as CLI args for server-filesystem
                for p in allowed_paths {
                    let s = p.display().to_string();
                    if !cfg.args.iter().any(|a| a == &s) {
                        cfg.args.push(s);
                    }
                }
            }
        }
        cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_seven() {
        assert_eq!(builtin_catalog().len(), 7);
    }

    #[test]
    fn filesystem_instantiate_adds_paths() {
        let e = catalog_entry("filesystem").unwrap();
        let cfg = e.instantiate(
            Some("docs-fs".into()),
            vec![PathBuf::from("/tmp")],
            None,
        );
        assert_eq!(cfg.name, "docs-fs");
        assert!(cfg.args.iter().any(|a| a == "/tmp"));
        assert_eq!(cfg.kind, "filesystem");
    }
}
