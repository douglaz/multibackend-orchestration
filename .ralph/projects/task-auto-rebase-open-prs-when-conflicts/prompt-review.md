---
artifact: prompt-review
project: task-auto-rebase-open-prs-when-conflicts
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-14T22:36:56Z
---

# Prompt Review

## Issues Found
- The provided text is a change summary, not an executable spec; it does not define a single end-to-end flow that implementers can follow without inference.
- Config keys, types, and defaults are not explicit, which makes CLI plumbing and backward compatibility tests ambiguous.
- State transition rules are incomplete; it is unclear exactly when `last_rebase_at` and `last_rebase_head_sha` must change.
- Task selection semantics for `MAX_REBASES_PER_CYCLE` are not defined, so behavior can vary run-to-run and make tests flaky.
- Failure handling is partly defined, but not split by failure class (query failure, rebase failure, push lease rejection, timeout), which affects idempotency and observability.
- PR comment behavior defines the marker but not a canonical body template, making validate assertions brittle.
- Status output references a `LAST REBASE` column but not an exact timestamp format or `None` rendering.
- The test list is comprehensive, but required assertions and wiring details are not explicit enough to guarantee consistent conformance coverage.

## Refined Prompt
Implement daemon auto-rebase for PR-backed tasks in `ralph` with deterministic behavior, bounded runtime, and full conformance coverage.

### Objective
Automatically rebase task branches for open PRs onto their current PR base branch during daemon cycles, without blocking active workflows and without duplicate failure comments.

### Scope
This feature includes daemon loop behavior, GitHub query/parsing, worktree creation on arbitrary task branches, PR comment posting for failures, config/CLI plumbing, daemon state schema updates, status output, and validate conformance tests.

### Non-Goals
Do not change existing `create_worktree` behavior. Do not use `mergeStateStatus`. Do not introduce new rate-limit infrastructure beyond controlled loop behavior and existing intervals/caps.

### Required Behavior

1. PR mergeability query:
Use `gh pr view <pr_number> --json mergeable,state,baseRefName,headRefOid`.
Only these fields are allowed for this logic.
Map `mergeable` to `PrMergeStatus` exactly as:
`CONFLICTING -> PrMergeStatus::Conflicting`, `MERGEABLE -> PrMergeStatus::Mergeable`, `UNKNOWN -> PrMergeStatus::Unknown`.

2. Base branch source:
Use `baseRefName` from PR metadata every cycle and store it in `PrMergeInfo.base_branch`.
Rebase target must always be `origin/<baseRefName>`.
Never use a hardcoded default branch.
`DaemonTask` does not persist base branch.

3. Worktree creation on actual task branch:
Add `create_worktree_on_branch(repo_root, workspace_root, task_id, branch)`.
It creates/uses `rebase-{task_id}` and checks out the provided branch (which may differ from `ralph/daemon/{task_id}`).
Keep existing `create_worktree` unchanged.

4. Rebase phase placement and bounding:
Run rebase phase after `collect_children`.
Enforce `MAX_REBASES_PER_CYCLE = 3`.
Enforce `REBASE_TIMEOUT_SECONDS = 120` per rebase attempt.
Worst-case rebase phase must stay bounded to about 6 minutes.

5. Eligibility and skip policy:
Skip when auto-rebase is disabled by config.
Skip when task has no PR.
Skip when PR `state` is not open (closed or merged).
Skip when `PrMergeStatus` is `Conflicting` or `Unknown`.
Skip when `last_rebase_at` exists and is within configured rebase interval.
Process eligible tasks in deterministic order (ascending task id) until cap is reached.

6. Rebase execution:
For each eligible task, create worktree on the task’s branch, fetch remotes as needed, run rebase onto `origin/<baseRefName>`, then push with `git push --force-with-lease`.
If rebase succeeds and push succeeds, record success state and continue.
If push is rejected due to lease mismatch, treat as failure for this cycle and continue to next task.

7. Failure comments on PR (not issue):
Add `post_pr_comment(...)` using `gh pr comment`.
Failure comments must include marker:
`<!-- ralph:rebase:{task_id}:failed:{head_sha} -->`.
Dedup rule: same task id + same head sha must not produce another failure comment; new head sha may produce a new comment.
Failure comments are posted on the PR only.

8. Rate-limit and query failure handling:
If `gh pr view` returns non-zero for a task (including 403/rate-limit), log warning with stderr and stop processing further PR rebases for that cycle (`break` per-PR loop).
Do not crash daemon.
Rely on interval + cap + break-on-error for throttling.

9. State model and serialization:
Add `last_rebase_at` and `last_rebase_head_sha` to daemon task state.
`last_rebase_at` controls interval-based skipping.
`last_rebase_head_sha` controls failure-comment deduplication.
Preserve backwards-compatible deserialization for old state files missing these fields.

10. Observability:
`ralph daemon status` must include `LAST REBASE` column.
Format is exact RFC3339 UTC timestamp (example `2026-02-14T19:22:31Z`) or `-` when absent.
Log skip/failure reasons clearly enough for conformance assertions.

11. Config plumbing:
Add config get/set/show support for new auto-rebase settings in both global and project scopes.
Project config overrides global config.

### Implementation Deliverables

1. Runtime code changes implementing all behavior above.
2. New helper APIs:
`create_worktree_on_branch(...)`, `post_pr_comment(...)`, and updated PR merge info query/parsing.
3. State schema updates with backward-compatible serde behavior.
4. CLI/config surface updates for get/set/show and status output column.
5. Validate conformance tests in `src/validate/tests_daemon.rs`, registered in `src/validate/mod.rs`.

### Validate Conformance Coverage

Add tests for all of the following scenarios:
1. Config defaults and overrides.
2. Disabled auto-rebase causes skip.
3. Conflict detection via `mergeable`.
4. Closed/merged PR skip.
5. Unknown mergeability skip.
6. Branch-switched task rebases correct branch.
7. Base branch resolution from `baseRefName`.
8. PR comment posting (not issue comment).
9. Failure comment dedup by `head_sha`.
10. Force-with-lease rejection handling.
11. `gh pr view` failure/rate-limit break behavior.
12. Per-cycle cap (`MAX_REBASES_PER_CYCLE`).
13. Recently-rebased interval skip.
14. `ralph daemon status` `LAST REBASE` format/output.
15. Backward-compatible state serialization/deserialization.

### Acceptance Criteria

1. `nix develop -c cargo check` passes.
2. `nix develop -c cargo test` passes.
3. Validate suite includes and runs new daemon tests.
4. Behavior is deterministic and matches all rules above.
5. No duplicate failure comments for unchanged head SHA.
