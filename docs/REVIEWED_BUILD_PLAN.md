# Local reviewed builds and task coordination

Authorized by Paul's October 6 request to plan, implement, and test capabilities 1 and 2 sequentially in the local Bomb Code app.

Baseline: e650aee6ddff60b73d6ba13400b0e82ea736db18, local 0.1.3 appearance/history update. Isolated worktree: codex/bomb-reviewed-build-coordination. Preserve the native ACP engines, current theme, original transcripts, and existing approval controls.

## Phase 1 — reviewed build

1. Add a small, tested Rust workflow state machine. Each task has a project, objective, declared write paths, four separately configurable role routes, and a bounded repair limit.
2. Plan in a fresh read-only native ACP session. Present the complete plan to the user. Implementation requires an explicit plan approval bound to its content and task revision.
3. Require a clean Git project and pin the baseline. Give the task one dedicated retained worktree; all four role sessions inspect the same task checkout. Planner/auditor/verifier remain in Plan; implementer uses Ask, preserving tool approvals.
4. Execute implementation, audit and verification in separate native sessions. Repairs receive the original plan and actual findings. Parse a strict first-line verdict for reviewers; malformed output, abnormal ACP stops, timeout, event gaps, changed baseline or scope, or out-of-scope writes stop the workflow. Detect repeated findings and cap repair rounds.
5. Persist task state and evidence before acknowledging transitions; interrupted tasks stop after restart and require a new review. Cancellation invalidates in-flight results and cancels role sessions. Worktrees remain available for inspection; no automatic merge/push/deploy.
6. Add a Builds view matching the local app. Show task state, plan, role output, worktree and linked sessions, approval/cancel actions, and the distinction between agent checks and human acceptance.
7. Verify state transitions and failure boundaries, ACP event collection, host persistence and UI behavior; run repository check/clippy/test gates and commit phase 1 before phase 2.

## Phase 2 — coordination

1. Add dependency IDs and a concurrency limit over the reviewed build engine.
2. Scope conflicts compare canonical repository identity and normalized declared write paths (component boundaries, including root). Different linked worktrees still count as the same repository for overlap scheduling.
3. A dependent starts only when every prerequisite has explicit human acceptance. Failure, rejection, cancellation, interruption or an unresolved dependency blocks downstream tasks. Reject missing dependencies, self-dependency and cycles at submission.
4. Atomically reserve slots and write scopes before spawning; include awaiting plan/review states in reservations. Release on a terminal outcome. Keep independent tasks moving if another task fails.
5. Verify overlap serialization, concurrency cap, successful dependency semantics, error isolation, cancellations and restart behavior. Commit phase 2 separately.

## Local delivery

Architecture and simplicity review follow implementation. Build a signed local 0.1.4 bundle; wait for the setup chat's installer to finish, preserve its completed 0.1.3 app and database before replacement, install with rollback on failure, and verify the installed UI and a bounded native fixture workflow. Save source, patch, verification and recovery artifacts in this chat's outputs. Historical governance records from the donor repository grant no authority here.
