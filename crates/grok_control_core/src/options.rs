use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    #[default]
    Acp,
    Headless,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SpawnOptions {
    pub model: Option<String>,
    pub worktree: Option<String>,
    pub mode: AgentMode,
    pub prompt: Option<String>,
    pub rules: Vec<String>,
    pub always_approve: bool,
    pub plan_mode: bool,
    pub sandbox_profile: Option<String>,
    /// Raw ACP mcpServers payloads (advanced). Prefer `mcp_server_names`.
    pub mcp_servers: Vec<Value>,
    /// Names of configured MCP servers to attach (resolved by McpManager).
    pub mcp_server_names: Vec<String>,
    /// High-risk MCP servers explicitly approved for this spawn.
    pub approved_high_risk_mcp: Vec<String>,
    /// Include servers with `auto_attach = true`.
    pub include_auto_mcp: bool,
    pub permission_allow: Vec<String>,
    pub permission_deny: Vec<String>,
    pub trust_repo: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            model: None,
            worktree: None,
            mode: AgentMode::Acp,
            prompt: None,
            rules: Vec::new(),
            always_approve: false,
            plan_mode: true,
            sandbox_profile: Some("workspace".into()),
            mcp_servers: Vec::new(),
            mcp_server_names: Vec::new(),
            approved_high_risk_mcp: Vec::new(),
            include_auto_mcp: true,
            permission_allow: Vec::new(),
            permission_deny: Vec::new(),
            trust_repo: false,
        }
    }
}

impl SpawnOptions {
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.mode, AgentMode::Headless) && self.prompt.as_ref().map(|p| p.trim().is_empty()).unwrap_or(true)
        {
            return Err("headless mode requires a non-empty prompt".into());
        }
        if self.always_approve && self.plan_mode {
            // Explicit always_approve wins; plan_mode soft-disabled
        }
        Ok(())
    }
}
