//! Pure admission decisions for reviewed builds. The host must atomically persist
//! every returned reservation before spawning; this module does not run or merge code.

use crate::{normalize_write_path, Result, WorkflowError, WorkflowStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinationTask {
    pub id: String,
    pub dependencies: Vec<String>,
    /// Canonical Git common directory supplied by the host, shared by linked worktrees.
    pub repository: String,
    pub write_set: Vec<String>,
    pub status: WorkflowStatus,
    pub reserved: bool,
    pub order: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    Reserved,
    Finished,
    WaitingDependencies,
    BlockedDependencies,
    WaitingScope,
    WaitingSlot,
    Queued,
}

fn invalid(message: impl Into<String>) -> WorkflowError {
    WorkflowError::InvalidSpec(message.into())
}

fn validate_limit(limit: usize) -> Result<()> {
    if !(1..=4).contains(&limit) {
        return Err(invalid("coordination concurrency limit must be 1..4"));
    }
    Ok(())
}

/// Validates all records, including completed ones, so no hidden invalid edge can
/// become executable after a later state change. Uses Kahn's algorithm without recursion.
pub fn validate_graph(tasks: &[CoordinationTask]) -> Result<()> {
    let mut indices = HashMap::new();
    for (index, task) in tasks.iter().enumerate() {
        if task.id.trim().is_empty() || task.id.chars().any(char::is_control) {
            return Err(invalid("coordination task ID is empty or invalid"));
        }
        if indices.insert(task.id.as_str(), index).is_some() {
            return Err(invalid(format!(
                "duplicate coordination task ID: {}",
                task.id
            )));
        }
        if !Path::new(&task.repository).is_absolute()
            || task.repository.chars().any(char::is_control)
        {
            return Err(invalid(format!(
                "task {} has no canonical repository identity",
                task.id
            )));
        }
        if task.write_set.is_empty() {
            return Err(invalid(format!(
                "task {} has no declared write paths",
                task.id
            )));
        }
        for path in &task.write_set {
            normalize_write_path(path)?;
        }
    }
    let mut incoming = vec![0usize; tasks.len()];
    let mut dependents = vec![Vec::new(); tasks.len()];
    for (index, task) in tasks.iter().enumerate() {
        let mut seen = HashSet::new();
        for dependency in &task.dependencies {
            if dependency == &task.id {
                return Err(invalid(format!("task {} depends on itself", task.id)));
            }
            if !seen.insert(dependency) {
                return Err(invalid(format!(
                    "task {} repeats dependency {dependency}",
                    task.id
                )));
            }
            let parent = *indices.get(dependency.as_str()).ok_or_else(|| {
                invalid(format!(
                    "task {} has missing dependency {dependency}",
                    task.id
                ))
            })?;
            incoming[index] += 1;
            dependents[parent].push(index);
        }
    }
    let mut ready: Vec<_> = incoming
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (*n == 0).then_some(i))
        .collect();
    let mut visited = 0;
    while let Some(index) = ready.pop() {
        visited += 1;
        for &dependent in &dependents[index] {
            incoming[dependent] -= 1;
            if incoming[dependent] == 0 {
                ready.push(dependent);
            }
        }
    }
    if visited != tasks.len() {
        return Err(invalid("coordination dependency graph contains a cycle"));
    }
    Ok(())
}

/// Conservative case folding matches macOS's common case-insensitive filesystems.
/// Comparisons respect components, so `src` conflicts with `src/a` but not `src-old`.
pub fn scopes_overlap(left: &[String], right: &[String]) -> Result<bool> {
    let left: Vec<_> = left
        .iter()
        .map(|p| normalize_write_path(p).map(|p| p.to_lowercase()))
        .collect::<Result<_>>()?;
    let right: Vec<_> = right
        .iter()
        .map(|p| normalize_write_path(p).map(|p| p.to_lowercase()))
        .collect::<Result<_>>()?;
    Ok(left.iter().any(|a| {
        right.iter().any(|b| {
            a == "."
                || b == "."
                || a == b
                || a.strip_prefix(b).is_some_and(|tail| tail.starts_with('/'))
                || b.strip_prefix(a).is_some_and(|tail| tail.starts_with('/'))
        })
    }))
}

fn conflict(left: &CoordinationTask, right: &CoordinationTask) -> Result<bool> {
    if left.repository.to_lowercase() != right.repository.to_lowercase() {
        return Ok(false);
    }
    scopes_overlap(&left.write_set, &right.write_set)
}

fn dependencies_accepted(task: &CoordinationTask, tasks: &[CoordinationTask]) -> bool {
    task.dependencies.iter().all(|id| {
        tasks
            .iter()
            .any(|t| &t.id == id && t.status == WorkflowStatus::Accepted)
    })
}

/// Returns only new reservations. Existing nonterminal reservations keep their
/// slot and scope while awaiting human plan approval or final review.
pub fn ready_tasks(tasks: &[CoordinationTask], limit: usize) -> Result<Vec<String>> {
    validate_limit(limit)?;
    validate_graph(tasks)?;
    let mut reserved: Vec<_> = tasks
        .iter()
        .filter(|t| t.reserved && !t.status.is_terminal())
        .collect();
    let mut candidates: Vec<_> = tasks
        .iter()
        .filter(|t| {
            !t.reserved && t.status == WorkflowStatus::Planning && dependencies_accepted(t, tasks)
        })
        .collect();
    candidates.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
    let mut selected = Vec::new();
    for candidate in candidates {
        if reserved.len() >= limit {
            break;
        }
        if reserved
            .iter()
            .map(|other| conflict(candidate, other))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .any(|c| c)
        {
            continue;
        }
        // Include earlier selections before evaluating the next candidate.
        reserved.push(candidate);
        selected.push(candidate.id.clone());
    }
    Ok(selected)
}

fn blocked_dependency(task: &CoordinationTask, tasks: &[CoordinationTask]) -> bool {
    let by_id: HashMap<_, _> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut pending: Vec<_> = task.dependencies.iter().map(String::as_str).collect();
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let dependency = by_id[id]; // validate_graph already established every edge.
        if dependency.status == WorkflowStatus::Accepted {
            continue;
        }
        if dependency.status.is_terminal() {
            return true;
        }
        pending.extend(dependency.dependencies.iter().map(String::as_str));
    }
    false
}

pub fn queue_state(task_id: &str, tasks: &[CoordinationTask], limit: usize) -> Result<QueueState> {
    let selected = ready_tasks(tasks, limit)?;
    let task = tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| invalid(format!("unknown coordination task: {task_id}")))?;
    if task.status.is_terminal() {
        return Ok(QueueState::Finished);
    }
    if task.reserved {
        return Ok(QueueState::Reserved);
    }
    if blocked_dependency(task, tasks) {
        return Ok(QueueState::BlockedDependencies);
    }
    if !dependencies_accepted(task, tasks) {
        return Ok(QueueState::WaitingDependencies);
    }
    if selected.iter().any(|id| id == task_id) {
        return Ok(QueueState::Queued);
    }
    let reservations: Vec<_> = tasks
        .iter()
        .filter(|t| (t.reserved && !t.status.is_terminal()) || selected.contains(&t.id))
        .collect();
    if reservations
        .iter()
        .map(|other| conflict(task, other))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .any(|c| c)
    {
        return Ok(QueueState::WaitingScope);
    }
    if reservations.len() >= limit {
        return Ok(QueueState::WaitingSlot);
    }
    Ok(QueueState::Queued)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, path: &str, order: u64) -> CoordinationTask {
        CoordinationTask {
            id: id.into(),
            dependencies: Vec::new(),
            repository: "/repo/.git".into(),
            write_set: vec![path.into()],
            status: WorkflowStatus::Planning,
            reserved: false,
            order,
        }
    }
    fn active(mut task: CoordinationTask, status: WorkflowStatus) -> CoordinationTask {
        task.status = status;
        task.reserved = true;
        task
    }

    #[test]
    fn deterministic_selection_counts_virtual_reservations_in_same_pass() {
        let tasks = vec![
            task("late", "src", 3),
            task("first", "./src/a", 1),
            task("independent", "tests", 2),
            task("another", "docs", 4),
        ];
        assert_eq!(
            ready_tasks(&tasks, 2).unwrap(),
            vec!["first", "independent"]
        );
        assert_eq!(
            queue_state("late", &tasks, 2).unwrap(),
            QueueState::WaitingScope
        );
        assert_eq!(
            queue_state("another", &tasks, 2).unwrap(),
            QueueState::WaitingSlot
        );
        assert_eq!(queue_state("first", &tasks, 2).unwrap(), QueueState::Queued);
    }

    #[test]
    fn concurrency_limit_counts_all_nonterminal_reservations() {
        for status in [
            WorkflowStatus::Planning,
            WorkflowStatus::AwaitingPlanApproval,
            WorkflowStatus::Implementing,
            WorkflowStatus::Auditing,
            WorkflowStatus::Verifying,
            WorkflowStatus::ReadyForReview,
        ] {
            let tasks = vec![
                active(task("active", "src", 0), status),
                task("next", "tests", 1),
            ];
            assert!(ready_tasks(&tasks, 1).unwrap().is_empty());
            assert_eq!(
                queue_state("active", &tasks, 1).unwrap(),
                QueueState::Reserved
            );
            assert_eq!(
                queue_state("next", &tasks, 1).unwrap(),
                QueueState::WaitingSlot
            );
        }
        assert!(ready_tasks(&[], 0).is_err());
        assert!(ready_tasks(&[], 5).is_err());
    }

    #[test]
    fn overlap_has_component_boundaries_root_and_conservative_case_folding() {
        for (left, right) in [
            ("src", "src/a.rs"),
            ("src/a", "src"),
            (".", "docs"),
            ("docs", "."),
            ("SRC", "src/a"),
            ("./src//a/", "src/a"),
        ] {
            assert!(scopes_overlap(&[left.into()], &[right.into()]).unwrap());
        }
        assert!(!scopes_overlap(&["src".into()], &["src-other/a".into()]).unwrap());
        assert!(!scopes_overlap(&["src/a".into()], &["src/b".into()]).unwrap());
        assert!(scopes_overlap(&["../secret".into()], &["src".into()]).is_err());
    }

    #[test]
    fn canonical_repository_identity_conflicts_across_linked_worktrees() {
        let running = active(task("worktree-one", "src", 0), WorkflowStatus::Auditing);
        let same_repo = task("worktree-two", "src/a", 1);
        let mut other_repo = task("other-repo", "src", 2);
        other_repo.repository = "/other/.git".into();
        let tasks = vec![running, same_repo, other_repo];
        assert_eq!(ready_tasks(&tasks, 3).unwrap(), vec!["other-repo"]);
        assert_eq!(
            queue_state("worktree-two", &tasks, 3).unwrap(),
            QueueState::WaitingScope
        );
    }

    #[test]
    fn dependent_requires_human_acceptance_and_never_agent_pass_alone() {
        let mut child = task("child", "tests", 1);
        child.dependencies = vec!["parent".into()];
        for status in [
            WorkflowStatus::AwaitingPlanApproval,
            WorkflowStatus::Implementing,
            WorkflowStatus::Auditing,
            WorkflowStatus::Verifying,
            WorkflowStatus::ReadyForReview,
        ] {
            let tasks = vec![active(task("parent", "src", 0), status), child.clone()];
            assert!(ready_tasks(&tasks, 4).unwrap().is_empty());
            assert_eq!(
                queue_state("child", &tasks, 4).unwrap(),
                QueueState::WaitingDependencies
            );
        }
        let tasks = vec![
            active(task("parent", "src", 0), WorkflowStatus::Accepted),
            child,
        ];
        assert_eq!(ready_tasks(&tasks, 1).unwrap(), vec!["child"]);
    }

    #[test]
    fn every_prerequisite_must_be_accepted() {
        let mut child = task("child", "docs", 3);
        child.dependencies = vec!["a".into(), "b".into()];
        let mut tasks = vec![
            active(task("a", "src", 1), WorkflowStatus::Accepted),
            active(task("b", "tests", 2), WorkflowStatus::ReadyForReview),
            child,
        ];
        assert!(ready_tasks(&tasks, 4).unwrap().is_empty());
        tasks[1].status = WorkflowStatus::Accepted;
        assert_eq!(ready_tasks(&tasks, 1).unwrap(), vec!["child"]);
    }

    #[test]
    fn failed_dependencies_block_transitively_without_stopping_independent_work() {
        for status in [
            WorkflowStatus::Failed,
            WorkflowStatus::Cancelled,
            WorkflowStatus::Interrupted,
            WorkflowStatus::NeedsChanges,
            WorkflowStatus::Stalled,
        ] {
            let root = active(task("root", "src", 0), status);
            let mut child = task("child", "tests", 1);
            child.dependencies = vec!["root".into()];
            let mut grandchild = task("grandchild", "docs", 2);
            grandchild.dependencies = vec!["child".into()];
            let independent = task("independent", "src", 3);
            let tasks = vec![root, child, grandchild, independent];
            assert_eq!(ready_tasks(&tasks, 1).unwrap(), vec!["independent"]);
            assert_eq!(
                queue_state("child", &tasks, 1).unwrap(),
                QueueState::BlockedDependencies
            );
            assert_eq!(
                queue_state("grandchild", &tasks, 1).unwrap(),
                QueueState::BlockedDependencies
            );
        }
    }

    #[test]
    fn all_terminal_outcomes_release_slot_and_scope_even_with_stale_reserved_flag() {
        for status in [
            WorkflowStatus::Accepted,
            WorkflowStatus::Cancelled,
            WorkflowStatus::Failed,
            WorkflowStatus::Interrupted,
            WorkflowStatus::NeedsChanges,
            WorkflowStatus::Stalled,
        ] {
            let tasks = vec![
                active(task("stopped", ".", 0), status),
                task("next", "src", 1),
            ];
            assert_eq!(ready_tasks(&tasks, 1).unwrap(), vec!["next"]);
            assert_eq!(
                queue_state("stopped", &tasks, 1).unwrap(),
                QueueState::Finished
            );
        }
    }

    #[test]
    fn invalid_graphs_are_rejected_before_any_admission() {
        let a = task("a", "src", 0);
        let mut b = task("b", "tests", 1);
        b.dependencies = vec!["missing".into()];
        assert!(ready_tasks(&[a.clone(), b.clone()], 4).is_err());
        b.dependencies = vec!["b".into()];
        assert!(validate_graph(&[a.clone(), b.clone()]).is_err());
        b.dependencies = vec!["a".into(), "a".into()];
        assert!(validate_graph(&[a.clone(), b.clone()]).is_err());
        assert!(validate_graph(&[a.clone(), a.clone()]).is_err());
        b.dependencies = vec!["a".into()];
        let mut cycle = a;
        cycle.dependencies = vec!["b".into()];
        assert!(validate_graph(&[cycle, b]).is_err());
    }

    #[test]
    fn a_long_cycle_and_malformed_scopes_fail_without_recursion() {
        let mut tasks: Vec<_> = (0..1000)
            .map(|i| task(&format!("t{i}"), "src", i))
            .collect();
        for (i, task) in tasks.iter_mut().enumerate() {
            task.dependencies = vec![format!("t{}", (i + 1) % 1000)];
        }
        assert!(validate_graph(&tasks).is_err());
        let mut task = task("bad", "src", 0);
        task.write_set = vec!["/outside".into()];
        assert!(validate_graph(&[task]).is_err());
    }

    #[test]
    fn stable_ties_unknown_ids_and_serialized_queue_state_are_explicit() {
        let tasks = vec![task("b", "tests", 0), task("a", "src", 0)];
        assert_eq!(ready_tasks(&tasks, 1).unwrap(), vec!["a"]);
        assert!(queue_state("missing", &tasks, 1).is_err());
        assert_eq!(
            serde_json::to_string(&QueueState::BlockedDependencies).unwrap(),
            "\"blocked_dependencies\""
        );
    }
}
