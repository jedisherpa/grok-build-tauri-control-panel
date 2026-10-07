//! Permission controller: allow/deny rules, presets, and sandbox policy.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use grok_config::{PermissionDefaults, SandboxProfile};

/// Host-observed effect, rather than an agent's display title or claimed kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Read,
    Control,
    Write,
    Process,
    Unknown,
}

pub fn canonical_tool(tool: &str) -> &str {
    match tool.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "read" | "readfile" | "readtextfile" | "fs/readtextfile" | "fs/read" => "Read",
        "glob" => "Glob",
        "grep" | "search" | "searchfiles" | "fetchrules" => "Grep",
        "list" | "listfiles" | "listdirectory" => "Read",
        "write" | "edit" | "writefile" | "writetextfile" | "fs/writetextfile" | "fs/write" => "Write",
        "bash" | "shell" | "terminal/create" | "runterminalcommand" | "runcommand" => "Bash",
        "delete" | "deletefile" => "Delete",
        "move" | "movefile" | "multiedit" | "applypatch" => "Write",
        "exitplanmode" | "x.ai/exitplanmode" => "ExitPlanMode",
        _ => tool,
    }
}

pub fn operation(tool: &str) -> Operation {
    match canonical_tool(tool) {
        "Read" | "Glob" | "Grep" => Operation::Read,
        "ExitPlanMode" => Operation::Control,
        "Write" | "Delete" | "Move" | "MultiEdit" | "ApplyPatch" => Operation::Write,
        "Bash" => Operation::Process,
        _ => Operation::Unknown,
    }
}

/// One evaluator used by preview, native permission requests and actual host
/// effects. A user Allow/Always/Yolo choice never widens an immutable ceiling.
#[derive(Debug, Clone, Copy)]
pub struct PolicyContext {
    pub read_only: bool,
    pub plan_mode: bool,
    pub cancelled: bool,
    pub always_approve: bool,
    pub auto_allow: bool,
}

pub fn evaluate_policy(
    tool: &str,
    detail: &str,
    effect: Operation,
    context: PolicyContext,
    rules: impl IntoIterator<Item = PermissionRule>,
) -> PermissionDecision {
    let rules: Vec<_> = rules.into_iter().collect();
    if context.cancelled
        || ((context.read_only || context.plan_mode) && !matches!(effect, Operation::Read | Operation::Control))
        || rules.iter().any(|r| r.decision == PermissionDecision::Deny
            && matches_any_pattern(std::slice::from_ref(&r.pattern), tool, detail))
    {
        return PermissionDecision::Deny;
    }
    if effect == Operation::Control { return PermissionDecision::Ask; }
    // Explicit Ask survives allow and automation too; deny is always first.
    if rules.iter().any(|r| r.decision == PermissionDecision::Ask
        && matches_any_pattern(std::slice::from_ref(&r.pattern), tool, detail)) {
        return PermissionDecision::Ask;
    }
    if context.always_approve || context.auto_allow
        || (context.plan_mode && effect == Operation::Read)
        || rules.iter().any(|r| r.decision == PermissionDecision::Allow
            && matches_any_pattern(std::slice::from_ref(&r.pattern), tool, detail))
    {
        PermissionDecision::Allow
    } else {
        PermissionDecision::Ask
    }
}

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
        let effect = operation(tool);
        evaluate_policy(tool, detail, effect, PolicyContext {
            read_only: !self.sandbox.allows_writes(), plan_mode: self.plan_mode,
            cancelled: false, always_approve: self.always_approve,
            auto_allow: self.trust_repo && effect == Operation::Read,
        }, self.session.iter().chain(self.global.iter()).cloned())
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
/// True when any deny pattern matches `tool(detail)` or the bare tool name.
/// Used by the ACP approval path to hard-block denied tools before the
/// approval card is even shown.
pub fn matches_any_pattern(patterns: &[String], tool: &str, detail: &str) -> bool {
    let tool = canonical_tool(tool);
    let candidate = format!("{tool}({detail})");
    patterns
        .iter()
        .any(|p| {
            let normalized = if let Some((name, suffix)) = p.split_once('(') {
                format!("{}({suffix}", canonical_tool(name))
            } else { canonical_tool(p).to_string() };
            pattern_matches(&normalized, &candidate) || pattern_matches(&normalized, tool)
        })
}

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
        let mut ctl = PermissionController::with_preset(&preset);
        ctl.set_plan_mode(false);
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

    #[test]
    fn deny_beats_global_and_session_allows_and_yolo_across_aliases() {
        let defaults = PermissionDefaults { allow:vec!["*".into()],
            deny:vec!["Write(private/**)".into(), "Bash(rm *)".into()], trust_repo:true };
        let mut controller = PermissionController::from_defaults(&defaults, SandboxProfile::Workspace);
        controller.set_session_rules(vec![PermissionRule { pattern:"*".into(), decision:PermissionDecision::Allow }]);
        for yolo in [false, true] {
            controller.set_plan_mode(false);
            controller.set_always_approve(yolo);
            for tool in ["Write", "Edit", "write_file", "fs/write_text_file", "fs/write"] {
                assert_eq!(controller.evaluate(tool, "private/key"), PermissionDecision::Deny);
            }
            for tool in ["Bash", "Shell", "run_command", "run_terminal_command", "terminal/create"] {
                assert_eq!(controller.evaluate(tool, "rm target"), PermissionDecision::Deny);
            }
        }
        controller.set_session_rules(vec![PermissionRule { pattern:"Edit(src/**)".into(), decision:PermissionDecision::Deny }]);
        assert_eq!(controller.evaluate("Write", "src/main.rs"), PermissionDecision::Deny);
    }

    #[test]
    fn read_only_and_plan_are_ceilings_not_approval_suggestions() {
        let defaults = PermissionDefaults { allow:vec!["*".into()], deny:vec![], trust_repo:true };
        let mut readonly = PermissionController::from_defaults(&defaults, SandboxProfile::ReadOnly);
        readonly.set_always_approve(true);
        assert_eq!(readonly.evaluate("Write", "file"), PermissionDecision::Deny);
        assert_eq!(readonly.evaluate("Bash", "echo hi"), PermissionDecision::Deny);
        assert_eq!(readonly.evaluate("Read", "file"), PermissionDecision::Allow);
        assert_eq!(readonly.evaluate("ExitPlanMode", "proposal"), PermissionDecision::Ask);
        let plan = PermissionController::from_defaults(&defaults, SandboxProfile::Workspace);
        assert_eq!(plan.evaluate("Edit", "file"), PermissionDecision::Deny);
        assert_eq!(plan.evaluate("Read", "file"), PermissionDecision::Allow);
    }

    #[test]
    fn cancellation_beats_even_read_and_control_requests() {
        for effect in [Operation::Read, Operation::Write, Operation::Process, Operation::Control] {
            assert_eq!(evaluate_policy("*", "", effect, PolicyContext {
                read_only:false, plan_mode:false, cancelled:true, always_approve:true, auto_allow:true,
            }, []), PermissionDecision::Deny);
        }
    }
}
