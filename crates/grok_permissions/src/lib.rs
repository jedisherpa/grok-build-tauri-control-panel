//! Permission controller: allow/deny rules, presets, and sandbox policy.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use grok_config::{PermissionDefaults, SandboxProfile};

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("denied by rule: {0}")]
    Denied(String),
    #[error("invalid rule: {0}")]
    InvalidRule(String),
}

pub type Result<T> = std::result::Result<T, PermissionError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionRule {
    /// Pattern like `Bash(git *)`, `Write(src/**)`, `Read(**)`.
    pub pattern: String,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPreset {
    pub name: String,
    pub description: String,
    pub sandbox: SandboxProfile,
    pub rules: Vec<PermissionRule>,
    pub always_approve: bool,
    pub plan_mode: bool,
}

#[derive(Debug, Clone)]
pub struct PermissionController {
    global: Vec<PermissionRule>,
    session: Vec<PermissionRule>,
    sandbox: SandboxProfile,
    always_approve: bool,
    plan_mode: bool,
    trust_repo: bool,
}

impl PermissionController {
    pub fn from_defaults(defaults: &PermissionDefaults, sandbox: SandboxProfile) -> Self {
        let mut global = Vec::new();
        for a in &defaults.allow {
            global.push(PermissionRule {
                pattern: a.clone(),
                decision: PermissionDecision::Allow,
            });
        }
        for d in &defaults.deny {
            global.push(PermissionRule {
                pattern: d.clone(),
                decision: PermissionDecision::Deny,
            });
        }
        Self {
            global,
            session: Vec::new(),
            sandbox,
            always_approve: false,
            plan_mode: true,
            trust_repo: defaults.trust_repo,
        }
    }

    pub fn with_preset(preset: &PermissionPreset) -> Self {
        Self {
            global: preset.rules.clone(),
            session: Vec::new(),
            sandbox: preset.sandbox,
            always_approve: preset.always_approve,
            plan_mode: preset.plan_mode,
            trust_repo: false,
        }
    }

    pub fn set_session_rules(&mut self, rules: Vec<PermissionRule>) {
        self.session = rules;
    }

    pub fn set_always_approve(&mut self, v: bool) {
        self.always_approve = v;
        if v {
            self.plan_mode = false;
        }
    }

    pub fn set_plan_mode(&mut self, v: bool) {
        self.plan_mode = v;
        if v {
            self.always_approve = false;
        }
    }

    pub fn sandbox(&self) -> SandboxProfile {
        self.sandbox
    }

    pub fn always_approve(&self) -> bool {
        self.always_approve
    }

    pub fn plan_mode(&self) -> bool {
        self.plan_mode
    }

    /// Evaluate a tool invocation against deny-first, then allow, else ask.
    pub fn evaluate(&self, tool: &str, detail: &str) -> PermissionDecision {
        if self.always_approve {
            return PermissionDecision::Allow;
        }

        let candidate = format!("{tool}({detail})");
        let tool_only = tool.to_string();

        // Session rules override global
        for rule in self.session.iter().chain(self.global.iter()) {
            if pattern_matches(&rule.pattern, &candidate) || pattern_matches(&rule.pattern, &tool_only)
            {
                debug!(pattern = %rule.pattern, decision = ?rule.decision, "rule match");
                return rule.decision;
            }
        }

        // Sandbox restrictions
        if !self.sandbox.allows_writes()
            && matches!(tool, "Write" | "Edit" | "Bash" | "Delete" | "Shell")
        {
            return PermissionDecision::Ask;
        }

        if self.trust_repo && matches!(tool, "Read" | "Glob" | "Grep") {
            return PermissionDecision::Allow;
        }

        PermissionDecision::Ask
    }

    pub fn assert_allowed(&self, tool: &str, detail: &str) -> Result<()> {
        match self.evaluate(tool, detail) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Deny => Err(PermissionError::Denied(format!("{tool}({detail})"))),
            PermissionDecision::Ask => Err(PermissionError::Denied(format!(
                "requires approval: {tool}({detail})"
            ))),
        }
    }
}

/// Built-in presets.
pub fn builtin_presets() -> Vec<PermissionPreset> {
    vec![
        PermissionPreset {
            name: "safe".into(),
            description: "Read-only + explicit asks for writes".into(),
            sandbox: SandboxProfile::ReadOnly,
            rules: vec![
                PermissionRule {
                    pattern: "Read(**)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Glob(**)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Grep(**)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Bash(rm *)".into(),
                    decision: PermissionDecision::Deny,
                },
                PermissionRule {
                    pattern: "Bash(sudo *)".into(),
                    decision: PermissionDecision::Deny,
                },
            ],
            always_approve: false,
            plan_mode: true,
        },
        PermissionPreset {
            name: "workspace".into(),
            description: "Normal interactive coding with plan mode".into(),
            sandbox: SandboxProfile::Workspace,
            rules: vec![
                PermissionRule {
                    pattern: "Read(**)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Write(src/**)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Write(crates/**)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Bash(git *)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Bash(cargo *)".into(),
                    decision: PermissionDecision::Allow,
                },
                PermissionRule {
                    pattern: "Bash(rm -rf *)".into(),
                    decision: PermissionDecision::Deny,
                },
            ],
            always_approve: false,
            plan_mode: true,
        },
        PermissionPreset {
            name: "yolo".into(),
            description: "Always approve — trusted repos only".into(),
            sandbox: SandboxProfile::Unrestricted,
            rules: vec![],
            always_approve: true,
            plan_mode: false,
        },
    ]
}

/// Glob-ish matching for tool rules.
/// Supports `*` (any chars) and exact tool names.
fn pattern_matches(pattern: &str, candidate: &str) -> bool {
    if pattern == candidate || pattern == "*" {
        return true;
    }
    // Convert simple glob to regex-ish manual match
    let pat = pattern.as_bytes();
    let cand = candidate.as_bytes();
    match_glob(pat, cand)
}

fn match_glob(pat: &[u8], cand: &[u8]) -> bool {
    let mut pi = 0;
    let mut ci = 0;
    let mut star_pi = None;
    let mut star_ci = 0;

    while ci < cand.len() {
        if pi < pat.len() && (pat[pi] == cand[ci] || pat[pi] == b'?') {
            pi += 1;
            ci += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_pi = Some(pi);
            star_ci = ci;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ci += 1;
            ci = star_ci;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_rm_rf() {
        let preset = builtin_presets()
            .into_iter()
            .find(|p| p.name == "workspace")
            .unwrap();
        let ctl = PermissionController::with_preset(&preset);
        assert_eq!(
            ctl.evaluate("Bash", "rm -rf /"),
            PermissionDecision::Deny
        );
        assert_eq!(
            ctl.evaluate("Bash", "git status"),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn yolo_allows_all() {
        let preset = builtin_presets()
            .into_iter()
            .find(|p| p.name == "yolo")
            .unwrap();
        let ctl = PermissionController::with_preset(&preset);
        assert_eq!(ctl.evaluate("Bash", "anything"), PermissionDecision::Allow);
    }

    #[test]
    fn glob_match() {
        assert!(pattern_matches("Bash(git *)", "Bash(git status)"));
        assert!(pattern_matches("Write(src/**)", "Write(src/main.rs)"));
        assert!(!pattern_matches("Bash(git *)", "Bash(rm -rf /)"));
    }
}
