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
    pub mcp_servers: Vec<Value>,
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
