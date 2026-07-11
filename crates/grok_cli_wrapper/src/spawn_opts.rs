//! Options for headless CLI spawn.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HeadlessSpawnOptions {
    pub model: Option<String>,
    pub worktree: Option<String>,
    pub always_approve: bool,
    pub plan_mode: bool,
    pub rules: Vec<String>,
    pub sandbox_profile: Option<String>,
    pub timeout_secs: Option<u64>,
}
