//! Session MCP attachment / injection helpers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::McpServerConfigExt;

/// How an MCP server is attached to a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAttachment {
    pub name: String,
    pub kind: String,
    pub approved: bool,
    pub auto: bool,
}

/// Resolve which servers to attach given requested names + auto_attach flags.
pub fn resolve_attachments(
    available: &[McpServerConfigExt],
    requested_names: &[String],
    approved_high_risk: &[String],
    include_auto: bool,
) -> Vec<McpServerConfigExt> {
    let mut out = Vec::new();
    for srv in available {
        if !srv.enabled {
            continue;
        }
        let requested = requested_names.iter().any(|n| n == &srv.name);
        let auto = include_auto && srv.auto_attach;
        if !requested && !auto {
            continue;
        }
        if srv.requires_approval || srv.high_risk {
            if !approved_high_risk.iter().any(|n| n == &srv.name) && !requested {
                // Auto-attach of high-risk requires explicit approval list
                if auto && !approved_high_risk.iter().any(|n| n == &srv.name) {
                    continue;
                }
            }
            if srv.requires_approval
                && !approved_high_risk.iter().any(|n| n == &srv.name)
                && !requested
            {
                continue;
            }
            // If explicitly requested, still require approval flag for high-risk
            if (srv.requires_approval || srv.high_risk)
                && !approved_high_risk.iter().any(|n| n == &srv.name)
            {
                // Explicit request without approval: skip with safety
                if requested && !approved_high_risk.iter().any(|n| n == &srv.name) {
                    continue;
                }
            }
        }
        out.push(srv.clone());
    }

    // Simpler pass: if name is requested AND (not high-risk OR approved), include
    // Re-do with clearer logic:
    out.clear();
    for srv in available {
        if !srv.enabled {
            continue;
        }
        let requested = requested_names.iter().any(|n| n == &srv.name);
        let auto = include_auto && srv.auto_attach;
        if !requested && !auto {
            continue;
        }
        let approved = approved_high_risk.iter().any(|n| n == &srv.name);
        if (srv.high_risk || srv.requires_approval) && !approved {
            continue;
        }
        out.push(srv.clone());
    }
    out
}

/// Build ACP `mcpServers` JSON array for session/new.
pub fn build_session_mcp_payload(servers: &[McpServerConfigExt]) -> Vec<Value> {
    servers.iter().map(|s| s.to_acp_payload()).collect()
}

/// Detect GitHub remote from `git remote -v` style text.
pub fn suggest_github_mcp(git_remote_output: &str) -> bool {
    git_remote_output.contains("github.com")
}

/// Extract Linear issue IDs like ENG-123 from branch/commit text.
pub fn detect_linear_issue_ids(text: &str) -> Vec<String> {
    let re = regex::Regex::new(r"\b([A-Z]{2,10}-\d+)\b").expect("regex");
    re.captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::McpTransport;

    fn sample(name: &str, high_risk: bool, auto: bool) -> McpServerConfigExt {
        McpServerConfigExt {
            name: name.into(),
            enabled: true,
            high_risk,
            requires_approval: high_risk,
            auto_attach: auto,
            transport: McpTransport::Stdio,
            command: Some("npx".into()),
            kind: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn skips_high_risk_without_approval() {
        let servers = vec![sample("playwright", true, false), sample("github", false, true)];
        let attached = resolve_attachments(&servers, &["playwright".into()], &[], true);
        assert!(attached.iter().all(|s| s.name != "playwright"));
        let attached = resolve_attachments(
            &servers,
            &["playwright".into()],
            &["playwright".into()],
            true,
        );
        assert!(attached.iter().any(|s| s.name == "playwright"));
    }

    #[test]
    fn auto_attach_safe() {
        let servers = vec![sample("github", false, true)];
        let attached = resolve_attachments(&servers, &[], &[], true);
        assert_eq!(attached.len(), 1);
    }

    #[test]
    fn linear_ids() {
        let ids = detect_linear_issue_ids("fix/ENG-42-and-also-ABC-9");
        assert!(ids.contains(&"ENG-42".into()));
        assert!(ids.contains(&"ABC-9".into()));
    }
}
