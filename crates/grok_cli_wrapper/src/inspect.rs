//! Inspect report types for `grok inspect --json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InspectReport {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub config_path: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub sessions: Option<u64>,
    #[serde(default)]
    pub raw: Option<Value>,
}

impl InspectReport {
    pub fn from_text(text: &str) -> Self {
        Self {
            raw: Some(Value::String(text.to_string())),
            ..Default::default()
        }
    }
}

/// Alias used by sketches / higher layers.
pub type GrokInspect = InspectReport;
