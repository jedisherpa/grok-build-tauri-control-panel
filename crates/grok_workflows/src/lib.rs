//! Reviewed builds stop at human review. The host owns execution, scope checks,
//! durable persistence and approval UI; this crate only makes transitions explicit.

pub mod coordination;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
pub const MAX_REPAIRS: u8 = 10;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowError {
    #[error("invalid workflow: {0}")]
    InvalidSpec(String),
    #[error("invalid transition: {0}")]
    InvalidTransition(String),
    #[error("approval no longer matches the task, plan or checkout")]
    StaleApproval,
    #[error("invalid role output: {0}")]
    InvalidOutput(String),
    #[error("cannot encode approval: {0}")]
    Encoding(String),
}

pub type Result<T> = std::result::Result<T, WorkflowError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Planner,
    Implementer,
    Auditor,
    Verifier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleRoute {
    pub backend: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleRoutes {
    pub planner: RoleRoute,
    pub implementer: RoleRoute,
    pub auditor: RoleRoute,
    pub verifier: RoleRoute,
}

impl RoleRoutes {
    pub fn get(&self, role: Role) -> &RoleRoute {
        match role {
            Role::Planner => &self.planner,
            Role::Implementer => &self.implementer,
            Role::Auditor => &self.auditor,
            Role::Verifier => &self.verifier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSpec {
    pub project_root: String,
    pub objective: String,
    pub write_set: Vec<String>,
    pub roles: RoleRoutes,
    pub max_repairs: u8,
}

impl WorkflowSpec {
    pub fn validate(&self) -> Result<()> {
        if !Path::new(&self.project_root).is_absolute() || self.project_root.contains('\0') {
            return Err(WorkflowError::InvalidSpec(
                "project root must be absolute".into(),
            ));
        }
        if self.objective.trim().is_empty() || self.objective.len() > 32 * 1024 {
            return Err(WorkflowError::InvalidSpec(
                "objective must contain 1..32768 bytes".into(),
            ));
        }
        if self.max_repairs > MAX_REPAIRS {
            return Err(WorkflowError::InvalidSpec("repair limit exceeds 10".into()));
        }
        if self.write_set.is_empty() || self.write_set.len() > 256 {
            return Err(WorkflowError::InvalidSpec(
                "declare 1..256 write paths".into(),
            ));
        }
        for path in &self.write_set {
            normalize_write_path(path)?;
        }
        for role in [
            Role::Planner,
            Role::Implementer,
            Role::Auditor,
            Role::Verifier,
        ] {
            let route = self.roles.get(role);
            if route.backend.trim().is_empty()
                || route.backend.len() > 128
                || route.backend.chars().any(char::is_control)
                || route.model.as_ref().is_some_and(|m| {
                    m.trim().is_empty() || m.len() > 256 || m.chars().any(char::is_control)
                })
            {
                return Err(WorkflowError::InvalidSpec("invalid role route".into()));
            }
        }
        Ok(())
    }
}

/// Relative, component-aware paths. `.` explicitly authorizes the entire project.
/// The host must additionally resolve repository identity and symlink escapes.
pub fn normalize_write_path(path: &str) -> Result<String> {
    if path == "." {
        return Ok(path.into());
    }
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(WorkflowError::InvalidSpec(format!(
            "invalid write path: {path:?}"
        )));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part == ".." || part.eq_ignore_ascii_case(".git") {
            return Err(WorkflowError::InvalidSpec(format!(
                "unsafe write path: {path:?}"
            )));
        }
        if !part.is_empty() && part != "." {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        return Err(WorkflowError::InvalidSpec(
            "use '.' for the entire project".into(),
        ));
    }
    Ok(parts.join("/"))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Planning,
    AwaitingPlanApproval,
    Implementing,
    Auditing,
    Verifying,
    ReadyForReview,
    Accepted,
    NeedsChanges,
    Stalled,
    Cancelled,
    Failed,
    Interrupted,
}

impl WorkflowStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Accepted
                | Self::NeedsChanges
                | Self::Stalled
                | Self::Cancelled
                | Self::Failed
                | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStep {
    pub role: Role,
    pub round: u8,
    pub output: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub spec: WorkflowSpec,
    pub status: WorkflowStatus,
    pub revision: u64,
    /// Number of repairs started; initial implementation is round zero.
    pub round: u8,
    pub plan: Option<String>,
    /// Actual latest failed reviewer output, including its verdict.
    pub findings: Option<String>,
    pub steps: Vec<WorkflowStep>,
    pub worktree: Option<String>,
    pub base_commit: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub last_failure_fingerprint: Option<String>,
    #[serde(default)]
    pub approved_content_digest: Option<String>,
}

impl Workflow {
    pub fn new(mut spec: WorkflowSpec) -> Result<Self> {
        spec.validate()?;
        spec.write_set = spec
            .write_set
            .iter()
            .map(|p| normalize_write_path(p))
            .collect::<Result<_>>()?;
        spec.write_set.sort();
        spec.write_set.dedup();
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            spec,
            status: WorkflowStatus::Planning,
            revision: 0,
            round: 0,
            plan: None,
            findings: None,
            steps: Vec::new(),
            worktree: None,
            base_commit: None,
            error: None,
            last_failure_fingerprint: None,
            approved_content_digest: None,
        })
    }

    pub fn role_to_run(&self) -> Option<Role> {
        match self.status {
            WorkflowStatus::Planning => Some(Role::Planner),
            WorkflowStatus::Implementing => Some(Role::Implementer),
            WorkflowStatus::Auditing => Some(Role::Auditor),
            WorkflowStatus::Verifying => Some(Role::Verifier),
            _ => None,
        }
    }

    pub fn set_checkout(&mut self, worktree: String, base_commit: String) -> Result<()> {
        if !matches!(
            self.status,
            WorkflowStatus::Planning | WorkflowStatus::AwaitingPlanApproval
        ) {
            return Err(WorkflowError::InvalidTransition(
                "checkout must be pinned before approval".into(),
            ));
        }
        if !Path::new(&worktree).is_absolute()
            || worktree.contains('\0')
            || !matches!(base_commit.len(), 40 | 64)
            || !base_commit.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(WorkflowError::InvalidSpec(
                "invalid checkout or commit".into(),
            ));
        }
        self.worktree = Some(worktree);
        self.base_commit = Some(base_commit);
        self.revision += 1;
        Ok(())
    }

    pub fn complete_role(&mut self, role: Role, output: impl Into<String>) -> Result<()> {
        if self.role_to_run() != Some(role) {
            return Err(WorkflowError::InvalidTransition(format!(
                "unexpected {role:?} result for {:?}",
                self.status
            )));
        }
        if role != Role::Planner {
            self.check_approval_binding()?;
        }
        let output = output.into();
        if output.trim().is_empty() || output.len() > MAX_OUTPUT_BYTES {
            self.fail("role output was empty or exceeded the evidence limit");
            return Err(WorkflowError::InvalidOutput(
                "empty or oversized output".into(),
            ));
        }
        let verdict = if matches!(role, Role::Auditor | Role::Verifier) {
            match parse_verdict(&output) {
                Ok(verdict) => Some(verdict),
                Err(error) => {
                    self.steps.push(WorkflowStep {
                        role,
                        round: self.round,
                        output,
                        session_id: None,
                    });
                    self.fail("reviewer did not return a valid explicit verdict with evidence");
                    return Err(error);
                }
            }
        } else {
            None
        };
        self.steps.push(WorkflowStep {
            role,
            round: self.round,
            output: output.clone(),
            session_id: None,
        });
        self.revision += 1;
        match role {
            Role::Planner => {
                self.plan = Some(output);
                self.status = WorkflowStatus::AwaitingPlanApproval;
            }
            Role::Implementer => self.status = WorkflowStatus::Auditing,
            Role::Auditor | Role::Verifier => {
                if verdict == Some(true) {
                    self.status = if role == Role::Auditor {
                        WorkflowStatus::Verifying
                    } else {
                        WorkflowStatus::ReadyForReview
                    };
                } else {
                    self.record_failure(role, output);
                }
            }
        }
        Ok(())
    }

    fn record_failure(&mut self, role: Role, output: String) {
        let normalized = output
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let fingerprint = hex::encode(Sha256::digest(format!("{role:?}:{normalized}").as_bytes()));
        self.findings = Some(output);
        if self.last_failure_fingerprint.as_ref() == Some(&fingerprint) {
            self.status = WorkflowStatus::Stalled;
            self.error = Some("reviewer repeated the same findings after repair".into());
        } else if self.round >= self.spec.max_repairs {
            self.status = WorkflowStatus::NeedsChanges;
            self.error = Some("repair limit exhausted".into());
        } else {
            self.round += 1;
            self.status = WorkflowStatus::Implementing;
        }
        self.last_failure_fingerprint = Some(fingerprint);
    }

    pub fn link_latest_session(&mut self, session_id: String) -> Result<()> {
        if session_id.trim().is_empty() {
            return Err(WorkflowError::InvalidSpec("session ID is empty".into()));
        }
        let step = self
            .steps
            .last_mut()
            .ok_or_else(|| WorkflowError::InvalidTransition("no role output to link".into()))?;
        step.session_id = Some(session_id);
        // Session metadata does not alter the authority-bearing task or plan.
        Ok(())
    }

    pub fn plan_digest(&self) -> Result<String> {
        if self.status != WorkflowStatus::AwaitingPlanApproval || self.plan.is_none() {
            return Err(WorkflowError::InvalidTransition(
                "no plan awaiting approval".into(),
            ));
        }
        self.spec.validate()?;
        let data = serde_json::to_vec(&(
            "bomb-reviewed-build-v1",
            &self.id,
            &self.spec,
            &self.plan,
            self.revision,
            &self.worktree,
            &self.base_commit,
        ))
        .map_err(|e| WorkflowError::Encoding(e.to_string()))?;
        Ok(hex::encode(Sha256::digest(data)))
    }

    pub fn approve_plan(&mut self, expected_digest: &str) -> Result<()> {
        if self.plan_digest()? != expected_digest {
            return Err(WorkflowError::StaleApproval);
        }
        if self.worktree.is_none() || self.base_commit.is_none() {
            return Err(WorkflowError::InvalidTransition(
                "pin a retained checkout before approval".into(),
            ));
        }
        self.approved_content_digest = Some(self.content_digest()?);
        self.status = WorkflowStatus::Implementing;
        self.revision += 1;
        Ok(())
    }

    pub fn accept_review(&mut self) -> Result<()> {
        if self.status != WorkflowStatus::ReadyForReview {
            return Err(WorkflowError::InvalidTransition(
                "human acceptance requires completed audit and verification".into(),
            ));
        }
        self.check_approval_binding()?;
        self.status = WorkflowStatus::Accepted;
        self.revision += 1;
        Ok(())
    }

    fn content_digest(&self) -> Result<String> {
        let data = serde_json::to_vec(&(
            &self.id,
            &self.spec,
            &self.plan,
            &self.worktree,
            &self.base_commit,
        ))
        .map_err(|e| WorkflowError::Encoding(e.to_string()))?;
        Ok(hex::encode(Sha256::digest(data)))
    }

    fn check_approval_binding(&mut self) -> Result<()> {
        if self.approved_content_digest.as_ref() != Some(&self.content_digest()?) {
            self.fail("task, plan or checkout changed after approval");
            return Err(WorkflowError::StaleApproval);
        }
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<()> {
        if self.status.is_terminal() {
            return Err(WorkflowError::InvalidTransition(
                "workflow already stopped".into(),
            ));
        }
        self.status = WorkflowStatus::Cancelled;
        self.revision += 1;
        Ok(())
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        if !self.status.is_terminal() {
            self.status = WorkflowStatus::Failed;
            self.error = Some(reason.into());
            self.revision += 1;
        }
    }

    /// Restart never silently restores approval or human acceptance of a run.
    pub fn interrupt_on_restart(&mut self) -> bool {
        if self.status.is_terminal() {
            return false;
        }
        self.status = WorkflowStatus::Interrupted;
        self.error = Some("app restarted; create a new reviewed task to continue".into());
        self.revision += 1;
        true
    }
}

/// Deliberately strict: first line, exact verdict, and nonempty supporting evidence.
pub fn parse_verdict(output: &str) -> Result<bool> {
    let (first, evidence) = output.split_once('\n').ok_or_else(|| {
        WorkflowError::InvalidOutput("verdict requires supporting evidence".into())
    })?;
    let pass = match first.strip_suffix('\r').unwrap_or(first) {
        "VERDICT: PASS" => true,
        "VERDICT: FAIL" => false,
        _ => {
            return Err(WorkflowError::InvalidOutput(
                "first line must be VERDICT: PASS or VERDICT: FAIL".into(),
            ))
        }
    };
    if evidence.trim().is_empty() {
        return Err(WorkflowError::InvalidOutput(
            "verdict requires supporting evidence".into(),
        ));
    }
    Ok(pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(max_repairs: u8) -> WorkflowSpec {
        let route = RoleRoute {
            backend: "codex".into(),
            model: None,
        };
        WorkflowSpec {
            project_root: "/project".into(),
            objective: "Fix the boundary bug".into(),
            write_set: vec!["src".into()],
            max_repairs,
            roles: RoleRoutes {
                planner: route.clone(),
                implementer: route.clone(),
                auditor: route.clone(),
                verifier: route,
            },
        }
    }

    fn planned(max_repairs: u8) -> Workflow {
        let mut task = Workflow::new(spec(max_repairs)).unwrap();
        task.set_checkout("/worktrees/task".into(), "a".repeat(40))
            .unwrap();
        task.complete_role(
            Role::Planner,
            "Plan: repair the boundary; test negative inputs",
        )
        .unwrap();
        task
    }

    fn approved(max_repairs: u8) -> Workflow {
        let mut task = planned(max_repairs);
        let digest = task.plan_digest().unwrap();
        task.approve_plan(&digest).unwrap();
        task
    }

    #[test]
    fn entire_flow_requires_both_human_gates() {
        let mut task = planned(2);
        assert_eq!(task.role_to_run(), None);
        assert!(task.complete_role(Role::Implementer, "done").is_err());
        assert!(task.accept_review().is_err());
        task.approve_plan(&task.plan_digest().unwrap()).unwrap();
        task.complete_role(Role::Implementer, "Changed source and added regression")
            .unwrap();
        assert!(task.accept_review().is_err());
        task.complete_role(
            Role::Auditor,
            "VERDICT: PASS\nScope and implementation inspected",
        )
        .unwrap();
        assert!(task.accept_review().is_err());
        task.complete_role(
            Role::Verifier,
            "VERDICT: PASS\nExecuted regression; 4 tests passed",
        )
        .unwrap();
        assert_eq!(task.status, WorkflowStatus::ReadyForReview);
        assert!(!task.status.is_terminal());
        assert_eq!(task.role_to_run(), None);
        task.accept_review().unwrap();
        assert_eq!(task.status, WorkflowStatus::Accepted);
        assert!(task.accept_review().is_err());
    }

    #[test]
    fn stale_plan_approvals_bind_scope_routes_revision_and_checkout() {
        for change in 0..7 {
            let mut task = planned(2);
            let original = task.plan_digest().unwrap();
            match change {
                0 => task.spec.objective.push_str(" and change auth"),
                1 => task.spec.write_set.push("other".into()),
                2 => task.spec.roles.implementer.backend = "grok".into(),
                3 => task.revision += 1,
                4 => task.plan = Some("Different plan".into()),
                5 => task.worktree = Some("/other".into()),
                _ => task.base_commit = Some("b".repeat(40)),
            }
            assert_eq!(
                task.approve_plan(&original),
                Err(WorkflowError::StaleApproval)
            );
            assert_eq!(task.status, WorkflowStatus::AwaitingPlanApproval);
        }
    }

    #[test]
    fn changing_an_approved_task_rejects_in_flight_output() {
        let mut task = approved(2);
        task.spec.write_set.push("private".into());
        assert_eq!(
            task.complete_role(Role::Implementer, "late result"),
            Err(WorkflowError::StaleApproval)
        );
        assert_eq!(task.status, WorkflowStatus::Failed);
        assert_eq!(task.steps.len(), 1);
    }

    #[test]
    fn checkout_is_required_and_cannot_be_swapped_after_approval() {
        let mut task = Workflow::new(spec(1)).unwrap();
        task.complete_role(Role::Planner, "Plan").unwrap();
        assert!(task.approve_plan(&task.plan_digest().unwrap()).is_err());
        task.set_checkout("/wt".into(), "a".repeat(40)).unwrap();
        task.approve_plan(&task.plan_digest().unwrap()).unwrap();
        assert!(task.set_checkout("/other".into(), "b".repeat(40)).is_err());
    }

    #[test]
    fn failed_verification_repairs_with_exact_findings_then_audits_again() {
        let mut task = approved(2);
        task.complete_role(Role::Implementer, "Initial fix")
            .unwrap();
        task.complete_role(Role::Auditor, "VERDICT: PASS\nInspected fix")
            .unwrap();
        let failure = "VERDICT: FAIL\nRegression fails: empty input returns an error";
        task.complete_role(Role::Verifier, failure).unwrap();
        assert_eq!(task.findings.as_deref(), Some(failure));
        assert_eq!(task.round, 1);
        assert_eq!(task.role_to_run(), Some(Role::Implementer));
        task.complete_role(Role::Implementer, "Addressed exact verification failure")
            .unwrap();
        assert_eq!(task.role_to_run(), Some(Role::Auditor));
        task.complete_role(
            Role::Auditor,
            "VERDICT: PASS\nChecked repaired implementation",
        )
        .unwrap();
        task.complete_role(Role::Verifier, "VERDICT: PASS\nRegression now passes")
            .unwrap();
        assert_eq!(task.status, WorkflowStatus::ReadyForReview);
        assert_eq!(task.steps.len(), 7);
    }

    #[test]
    fn normalized_repeated_findings_stall_before_budget_exhaustion() {
        let mut task = approved(5);
        task.complete_role(Role::Implementer, "Initial").unwrap();
        task.complete_role(Role::Auditor, "VERDICT: FAIL\nMissing scope check")
            .unwrap();
        task.complete_role(Role::Implementer, "Repair").unwrap();
        task.complete_role(Role::Auditor, "VERDICT: FAIL\n  MISSING   scope check  ")
            .unwrap();
        assert_eq!(task.status, WorkflowStatus::Stalled);
        assert_eq!(task.round, 1);
        assert_eq!(task.role_to_run(), None);
    }

    #[test]
    fn repair_cap_is_exact_including_zero() {
        for cap in [0, 1, 3] {
            let mut task = approved(cap);
            for n in 0..=cap {
                task.complete_role(Role::Implementer, format!("Implementation {n}"))
                    .unwrap();
                task.complete_role(Role::Auditor, format!("VERDICT: FAIL\nFinding {n}"))
                    .unwrap();
            }
            assert_eq!(task.round, cap);
            assert_eq!(task.status, WorkflowStatus::NeedsChanges);
            assert_eq!(task.role_to_run(), None);
        }
    }

    #[test]
    fn malformed_review_evidence_stops_instead_of_passing_or_repairing() {
        for output in [
            "PASS",
            "VERDICT: PASS",
            "VERDICT: PASS\n  ",
            "\nVERDICT: PASS\nEvidence",
            "Verdict: PASS\nEvidence",
            "VERDICT: UNKNOWN\nEvidence",
        ] {
            let mut task = approved(2);
            task.complete_role(Role::Implementer, "Fix").unwrap();
            assert!(matches!(
                task.complete_role(Role::Auditor, output),
                Err(WorkflowError::InvalidOutput(_))
            ));
            assert_eq!(task.status, WorkflowStatus::Failed);
            assert_eq!(task.round, 0);
            assert_eq!(task.steps.last().unwrap().output, output);
        }
        assert_eq!(parse_verdict("VERDICT: PASS\r\nChecked"), Ok(true));
    }

    #[test]
    fn oversized_or_empty_output_never_silently_truncates() {
        for output in [" ".into(), "x".repeat(MAX_OUTPUT_BYTES + 1)] {
            let mut task = approved(1);
            assert!(task.complete_role(Role::Implementer, output).is_err());
            assert_eq!(task.status, WorkflowStatus::Failed);
            assert_eq!(task.steps.len(), 1);
        }
    }

    #[test]
    fn cancel_invalidates_late_results_and_cannot_reopen_task() {
        let mut task = approved(2);
        let revision = task.revision;
        task.cancel().unwrap();
        assert!(task.revision > revision);
        assert!(task
            .complete_role(Role::Implementer, "late response")
            .is_err());
        assert!(task.cancel().is_err());
        task.fail("late failure");
        assert_eq!(task.status, WorkflowStatus::Cancelled);
    }

    #[test]
    fn restart_stops_all_unaccepted_states_including_waiting_reviews() {
        for status in [
            WorkflowStatus::Planning,
            WorkflowStatus::AwaitingPlanApproval,
            WorkflowStatus::Implementing,
            WorkflowStatus::Auditing,
            WorkflowStatus::Verifying,
            WorkflowStatus::ReadyForReview,
        ] {
            let mut task = approved(1);
            task.status = status;
            let encoded = serde_json::to_string(&task).unwrap();
            let mut restored: Workflow = serde_json::from_str(&encoded).unwrap();
            assert!(restored.interrupt_on_restart());
            assert_eq!(restored.status, WorkflowStatus::Interrupted);
            assert_eq!(restored.role_to_run(), None);
            assert!(!restored.interrupt_on_restart());
        }
        let mut task = approved(1);
        task.status = WorkflowStatus::Accepted;
        assert!(!task.interrupt_on_restart());
    }

    #[test]
    fn scope_validation_and_normalization_reject_escape_paths() {
        for path in [
            "",
            "/etc",
            "../src",
            "src/../secret",
            ".git",
            "src/.GIT/config",
            "src\\..\\secret",
            "C:/Windows",
            "src\0a",
            "./",
            "////",
        ] {
            assert!(normalize_write_path(path).is_err(), "accepted {path:?}");
        }
        assert_eq!(
            normalize_write_path("./src//components/"),
            Ok("src/components".into())
        );
        assert_eq!(normalize_write_path("."), Ok(".".into()));
        let mut input = spec(1);
        input.write_set = vec!["src/".into(), "./src".into(), "tests".into()];
        assert_eq!(
            Workflow::new(input).unwrap().spec.write_set,
            vec!["src", "tests"]
        );
    }

    #[test]
    fn invalid_specs_do_not_create_workflows() {
        let mut input = spec(1);
        input.project_root = "relative".into();
        assert!(Workflow::new(input).is_err());
        let mut input = spec(1);
        input.write_set.clear();
        assert!(Workflow::new(input).is_err());
        let mut input = spec(1);
        input.objective.clear();
        assert!(Workflow::new(input).is_err());
        assert!(Workflow::new(spec(MAX_REPAIRS + 1)).is_err());
        let mut input = spec(1);
        input.roles.auditor.backend.clear();
        assert!(Workflow::new(input).is_err());
    }

    #[test]
    fn out_of_order_results_cannot_skip_roles_or_change_evidence() {
        let mut task = approved(2);
        let before = serde_json::to_string(&task).unwrap();
        assert!(task
            .complete_role(Role::Verifier, "VERDICT: PASS\nCheck")
            .is_err());
        assert_eq!(serde_json::to_string(&task).unwrap(), before);
        task.complete_role(Role::Implementer, "Fix").unwrap();
        task.link_latest_session("native-session-2".into()).unwrap();
        assert_eq!(
            task.steps.last().unwrap().session_id.as_deref(),
            Some("native-session-2")
        );
    }
}
