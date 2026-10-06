//! Completion is a projection of durable workflow evidence, never session activity.
//! Repairs reset the implementation and review checkpoints for the new round.

use crate::{parse_verdict, Role, Workflow, WorkflowStatus, WorkflowStep, MAX_OUTPUT_BYTES};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowProgress {
    pub basis: String,
    pub completed: u8,
    pub total: u8,
    pub percent: u8,
    pub round: u8,
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub key: String,
    pub label: String,
    pub completed: bool,
    pub session_id: Option<String>,
}

impl Workflow {
    /// Six equally weighted checkpoints describe workflow completion. They do
    /// not estimate remaining time, code coverage, or work performed by a model.
    pub fn progress(&self) -> WorkflowProgress {
        let planner = self.latest_step(Role::Planner, 0);
        let plan_complete = planner.is_some_and(|(_, step)| {
            valid_report(step) && self.plan.as_deref() == Some(step.output.as_str())
        });
        let approval_complete = plan_complete
            && self.spec.validate().is_ok()
            && valid_checkout(self)
            && self.steps.iter().all(|step| {
                step.round <= self.round && (step.role != Role::Planner || step.round == 0)
            })
            && self
                .steps
                .windows(2)
                .all(|pair| pair[0].round <= pair[1].round)
            && self.round <= self.spec.max_repairs
            && self
                .approved_content_digest
                .as_ref()
                .is_some_and(|approved| {
                    self.content_digest()
                        .is_ok_and(|current| current == *approved)
                });

        let implementer = self.latest_step(Role::Implementer, self.round);
        let implementation_complete = approval_complete
            && follows(implementer, planner)
            && implementer.is_some_and(|(_, step)| valid_report(step));
        let auditor = self.latest_step(Role::Auditor, self.round);
        let audit_complete = implementation_complete
            && follows(auditor, implementer)
            && auditor.is_some_and(|(_, step)| passing_review(step));
        let verifier = self.latest_step(Role::Verifier, self.round);
        let verification_complete = audit_complete
            && follows(verifier, auditor)
            && verifier.is_some_and(|(_, step)| passing_review(step));
        let acceptance_complete = verification_complete && self.status == WorkflowStatus::Accepted;

        let checkpoints = vec![
            checkpoint("planner", "Plan recorded", plan_complete, planner),
            checkpoint("plan_approval", "Plan approved", approval_complete, None),
            checkpoint(
                "implementer",
                "Implementation recorded",
                implementation_complete,
                implementer,
            ),
            checkpoint("auditor", "Audit passed", audit_complete, auditor),
            checkpoint(
                "verifier",
                "Verification passed",
                verification_complete,
                verifier,
            ),
            checkpoint("acceptance", "Result accepted", acceptance_complete, None),
        ];
        let completed = checkpoints.iter().filter(|step| step.completed).count() as u8;
        WorkflowProgress {
            basis: "workflow_checkpoints".into(),
            completed,
            total: 6,
            percent: (u16::from(completed) * 100 / 6) as u8,
            round: self.round,
            checkpoints,
        }
    }

    fn latest_step(&self, role: Role, round: u8) -> Option<(usize, &WorkflowStep)> {
        self.steps
            .iter()
            .enumerate()
            .rev()
            .find(|(_, step)| step.role == role && step.round == round)
    }
}

fn valid_report(step: &WorkflowStep) -> bool {
    !step.output.trim().is_empty() && step.output.len() <= MAX_OUTPUT_BYTES
}

fn passing_review(step: &WorkflowStep) -> bool {
    valid_report(step) && parse_verdict(&step.output) == Ok(true)
}

fn valid_checkout(workflow: &Workflow) -> bool {
    workflow
        .worktree
        .as_ref()
        .is_some_and(|path| Path::new(path).is_absolute() && !path.contains('\0'))
        && workflow.base_commit.as_ref().is_some_and(|commit| {
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn follows(later: Option<(usize, &WorkflowStep)>, earlier: Option<(usize, &WorkflowStep)>) -> bool {
    matches!((later, earlier), (Some((later, _)), Some((earlier, _))) if later > earlier)
}

fn checkpoint(
    key: &str,
    label: &str,
    completed: bool,
    step: Option<(usize, &WorkflowStep)>,
) -> Checkpoint {
    Checkpoint {
        key: key.into(),
        label: label.into(),
        completed,
        // Failed/current evidence remains inspectable without counting as done.
        session_id: step.and_then(|(_, step)| step.session_id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RoleRoute, RoleRoutes, WorkflowSpec};

    fn planned() -> Workflow {
        let route = RoleRoute {
            backend: "codex".into(),
            model: None,
        };
        let mut workflow = Workflow::new(WorkflowSpec {
            project_root: "/project".into(),
            objective: "Fix greeting; run existing tests".into(),
            write_set: vec!["greeting.py".into()],
            roles: RoleRoutes {
                planner: route.clone(),
                implementer: route.clone(),
                auditor: route.clone(),
                verifier: route,
            },
            max_repairs: 2,
        })
        .unwrap();
        workflow
            .set_checkout("/worktrees/test".into(), "a".repeat(40))
            .unwrap();
        workflow
            .complete_role(Role::Planner, "Correct greeting and run unittest")
            .unwrap();
        workflow
            .link_latest_session("planner-session".into())
            .unwrap();
        workflow
    }

    fn approved() -> Workflow {
        let mut workflow = planned();
        workflow
            .approve_plan(&workflow.plan_digest().unwrap())
            .unwrap();
        workflow
    }

    fn ready() -> Workflow {
        let mut workflow = approved();
        workflow
            .complete_role(Role::Implementer, "Corrected greeting")
            .unwrap();
        workflow
            .complete_role(Role::Auditor, "VERDICT: PASS\nOnly declared file changed")
            .unwrap();
        workflow
            .complete_role(Role::Verifier, "VERDICT: PASS\nTwo unittest cases passed")
            .unwrap();
        workflow
    }

    #[test]
    fn success_is_below_one_hundred_until_explicit_acceptance() {
        let mut workflow = ready();
        assert_eq!(workflow.progress().completed, 5);
        assert_eq!(workflow.progress().percent, 83);
        assert_eq!(
            workflow.progress().checkpoints[0].session_id.as_deref(),
            Some("planner-session")
        );
        workflow.accept_review().unwrap();
        assert_eq!(workflow.progress().completed, 6);
        assert_eq!(workflow.progress().percent, 100);
        let restored: Workflow =
            serde_json::from_str(&serde_json::to_string(&workflow).unwrap()).unwrap();
        assert_eq!(restored.progress(), workflow.progress());
        workflow.steps.clear();
        assert_eq!(
            workflow.progress().completed,
            0,
            "accepted status alone is not evidence"
        );
    }

    #[test]
    fn verifier_failure_rolls_back_old_round_implementation_and_passing_audit() {
        let mut workflow = approved();
        workflow
            .complete_role(Role::Implementer, "First implementation")
            .unwrap();
        workflow
            .complete_role(Role::Auditor, "VERDICT: PASS\nScope inspected")
            .unwrap();
        assert_eq!(workflow.progress().completed, 4);
        workflow
            .complete_role(Role::Verifier, "VERDICT: FAIL\nNegative input still fails")
            .unwrap();
        assert_eq!(workflow.round, 1);
        assert_eq!(workflow.progress().completed, 2);
        assert_eq!(workflow.progress().percent, 33);
        workflow
            .complete_role(Role::Implementer, "Repaired negative input")
            .unwrap();
        assert_eq!(workflow.progress().completed, 3);
        assert!(
            !workflow.progress().checkpoints[3].completed,
            "old audit must not satisfy new round"
        );
        workflow.status = WorkflowStatus::Accepted;
        assert_eq!(
            workflow.progress().completed,
            3,
            "forged status cannot reuse old reviews"
        );
    }

    #[test]
    fn malformed_or_latest_failed_review_never_counts_an_earlier_pass() {
        for bad_output in [
            "VERDICT: PASS",
            "Progress\nVERDICT: PASS\nChecked",
            "VERDICT: FAIL\nRegression fails",
        ] {
            let mut workflow = ready();
            workflow.steps.push(WorkflowStep {
                role: Role::Auditor,
                round: 0,
                output: bad_output.into(),
                session_id: Some("failed-audit".into()),
            });
            workflow.status = WorkflowStatus::Accepted;
            let progress = workflow.progress();
            assert_eq!(progress.completed, 3);
            assert_eq!(
                progress.checkpoints[3].session_id.as_deref(),
                Some("failed-audit")
            );
            assert!(!progress.checkpoints[4].completed);
        }
    }

    #[test]
    fn altered_plan_or_approval_invalidates_all_later_checkpoints() {
        for change in 0..5 {
            let mut workflow = ready();
            workflow.status = WorkflowStatus::Accepted;
            match change {
                0 => workflow.approved_content_digest = Some("stale".into()),
                1 => workflow.spec.write_set.push("outside.py".into()),
                2 => workflow.plan = Some("Unreviewed replacement".into()),
                3 => workflow.base_commit = Some("b".repeat(40)),
                _ => workflow.steps[0].output = "Altered planner evidence".into(),
            }
            assert_eq!(
                workflow.progress().completed,
                if change == 2 || change == 4 { 0 } else { 1 }
            );
        }
    }

    #[test]
    fn cancelled_and_interrupted_records_keep_only_completed_evidence() {
        let planned = planned();
        assert_eq!(planned.progress().completed, 1);
        let mut cancelled = approved();
        cancelled
            .complete_role(Role::Implementer, "Changed declared path")
            .unwrap();
        cancelled.cancel().unwrap();
        assert_eq!(cancelled.progress().completed, 3);
        let mut interrupted = ready();
        interrupted.interrupt_on_restart();
        assert_eq!(interrupted.progress().completed, 5);
        assert!(!interrupted.progress().checkpoints[5].completed);
    }

    #[test]
    fn reordered_or_invalid_evidence_cannot_complete_the_chain() {
        let mut reordered = ready();
        reordered.status = WorkflowStatus::Accepted;
        reordered.steps.swap(1, 3);
        assert_eq!(reordered.progress().completed, 3);
        let mut oversized = ready();
        oversized.steps[1].output = "x".repeat(MAX_OUTPUT_BYTES + 1);
        assert_eq!(oversized.progress().completed, 2);
        let mut invalid = ready();
        invalid.round = 3;
        assert_eq!(invalid.progress().completed, 1);
        let mut future_evidence = ready();
        future_evidence.status = WorkflowStatus::Accepted;
        future_evidence.steps.push(WorkflowStep {
            role: Role::Auditor,
            round: 1,
            output: "VERDICT: PASS\nEvidence from an impossible future round".into(),
            session_id: None,
        });
        assert_eq!(future_evidence.progress().completed, 1);
    }
}
