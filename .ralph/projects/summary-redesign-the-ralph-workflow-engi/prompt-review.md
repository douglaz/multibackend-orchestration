---
artifact: prompt-review
project: summary-redesign-the-ralph-workflow-engi
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-19T16:30:15Z
---

# Prompt Review

## Issues Found

- The prompt starts with implementation narration instead of requirements, making intent unclear for downstream loops.
- Scope is very broad and "big-bang" across many modules without incremental delivery gates, increasing regression risk.
- "Zero local files required" is ambiguous because loop artifacts are still local files (even if committed); only durable state files should be banned.
- Commit parsing rules are underspecified (exact regex, allowed phase values, mismatch behavior), which risks inconsistent resume behavior.
- Label transition operations are described as atomic, but `gh` operations are not truly atomic; retry/conflict semantics are missing.
- Startup reconciliation logic assumes empty in-memory children after restart but does not explicitly define policy for all `ralph:in-progress` issues.
- Branch migration from old naming/config is not fully specified, including handling existing branches and deprecated `branch_format`.
- Failure behavior for network/auth/push errors is incomplete, which weakens feasibility of resumability guarantees.
- Several acceptance criteria are not mapped to deterministic validate tests with clear pass/fail mechanics.
- Safety constraints for destructive sync behavior (discard local commits) are not explicitly bounded to daemon-managed worktrees.

## Refined Prompt

# Engineering Specification: Remove Durable Local Workflow State

## Objective

Replace durable local daemon/orchestrator state with two external sources of truth:
- Git remote commit history on project branches.
- GitHub issue lifecycle labels.

After this change, restart from a fresh clone must be deterministic without reading `state.json`, `tasks.json`, or durable local logs.

## Problem Statement

Current failure modes to eliminate:
- Corrupt or drifted `state.json` causing unrecoverable flow state.
- `tasks.json` desync on daemon crash between label changes and file writes.
- Inability to resume from fresh clone because local state is not in Git.

## Scope

In scope:
- Remove durable local state files used for workflow position and daemon task tracking.
- Derive workflow position from structured commit messages and trailers.
- Derive task lifecycle from GitHub labels.
- Enforce single-daemon-per-repo lock.
- Make startup sync discard unpushed local daemon commits and align to remote state.
- Update CLI read paths (`status`, `history`) to use Git + labels.

Out of scope:
- Multi-host/distributed locking.
- Artifact schema redesign for loop files.
- Cross-restart session persistence.
- Migration tooling for old `state.json` projects.
- Prompt review workflow redesign beyond required compile/runtime fixes.

## Durable State Contract

Durable truth sources:
- Git: `origin/ralph/issue-<number>` branch history.
- GitHub: lifecycle labels on issues.

Allowed non-durable local state:
- In-memory runtime maps.
- `.ralph/tmp/**` only, recreated at daemon startup.

Prohibited durable local state:
- `.ralph/**/state.json`
- `.ralph/**/tasks.json`
- Append-only durable daemon/task logs outside `.ralph/tmp`.

## Definitions

- `project_id`: `issue-<number>`
- `project_branch`: `ralph/issue-<number>`
- Lifecycle labels: `ralph:ready`, `ralph:in-progress`, `ralph:completed`, `ralph:failed`
- Ralph checkpoint commit subject format: `ralph(issue-<number>): loop <loop_number> <from_phase> -> <to_phase>`
- Required trailers:
  - `Ralph-Project: issue-<number>`
  - `Ralph-Loop: <u32>`
  - `Ralph-Phase: <phase>`

## Required Behavior

### 1) Branch Sync (remote-first)

Implement `sync_project_branch(repo_root, issue_number)`:
1. `git fetch origin`
2. If `origin/ralph/issue-<n>` exists: `git checkout -B ralph/issue-<n> origin/ralph/issue-<n>`
3. Else: `git checkout -b ralph/issue-<n> origin/HEAD`
4. Never create project branches from local refs.

This behavior is only allowed in daemon-managed worktrees.

### 2) Position Derivation

Implement `parse_last_ralph_commit(repo_root, branch)` and `derive_position(...)`:
- Parse newest commit on `origin/<branch>` that matches required subject and trailers.
- Validate subject and trailer consistency.
- On no matching commit: default to `loop=1`, `phase=planning`.
- On malformed newest ralph checkpoint commit: return actionable error and stop processing that issue.

### 3) Phase Transition Checkpointing

Replace loop checkpoint logic with `commit_and_push_phase_transition(...)`:
1. Stage artifacts (`git add -A`)
2. Commit with structured subject + trailers
3. Push to `origin <project_branch>`
4. Advance in-memory phase only after successful push
5. Remove tag-based checkpointing for loop transitions

Crash semantics:
- Crash before commit: no remote state change.
- Crash after commit before push: startup sync discards local-only commit via remote-first checkout.

### 4) Daemon Runtime State

Replace durable task store with:
- `children: HashMap<u32, ChildHandle>`

`ChildHandle` includes:
- `pid`, `pgid`, `tokio::process::Child`, `branch`, `log_file`

No daemon task metadata is durably persisted. Issue metadata is fetched from GitHub on demand.

### 5) Lifecycle Label Rules

- Claim: `ready -> in-progress`
- Success: `in-progress -> completed`
- Failure: `in-progress -> failed`
- If issue has more than one lifecycle label, normalize to only `ralph:failed`, skip processing this poll cycle.
- Startup reconciliation: every issue currently labeled `ralph:in-progress` is reset to `ralph:ready` (children map is empty on fresh daemon start).

Implement label transitions with retry-on-conflict and retry-on-transient-failure policy.

### 6) Single Daemon Lock

Acquire and hold:
- `/tmp/ralph-daemon-<sha256(canonical_repo_root)>.lock`
- Use non-blocking `flock`
- If lock is unavailable, exit immediately with clear non-zero error.

### 7) Temporary Files

At daemon startup:
- Remove `.ralph/tmp` recursively if present.
- Recreate `.ralph/tmp/logs`.
- Route agent output logs to `.ralph/tmp/logs`.

## Required Code Changes

- `src/project/state.rs`: remove durable save/load paths; keep in-memory model only.
- `src/project/lifecycle.rs`: remove state-file load/save/recovery; add commit/artifact-derived reconstruction hooks.
- `src/workflow/orchestrator.rs`: remove `persist_state()` calls; use commit+push checkpointing.
- `src/git/ralph_commit.rs`: new commit parser/builder with strict validation.
- `src/git/commit.rs`: add commit+push phase transition API; remove loop tag checkpointing.
- `src/git/branch.rs`: add remote-first branch sync utility.
- `src/daemon/mod.rs`: remove `TaskStore` and `tasks.json` persistence.
- `src/daemon/runtime.rs`: use in-memory child map and label-driven reconciliation.
- `src/daemon/worktree.rs`: enforce `origin/HEAD` / `origin/<branch>` base refs only.
- `src/daemon/github.rs`: add lifecycle normalization and robust label swap APIs.
- `src/output_log.rs`: move logs to `.ralph/tmp/logs`.
- `src/cli/status.rs`: derive state from commit+labels.
- `src/cli/history.rs`: derive loop/phase history from commit trailers.
- `src/util/lock.rs`: implement daemon lock file behavior above.

## Acceptance Criteria

1. Fresh clone restart resumes from last pushed checkpoint commit and current labels.
2. Workflow position is derived only from commit subject+trailers on remote branch.
3. Task lifecycle state is derived only from GitHub labels.
4. Crash before commit does not advance remote state.
5. Crash after local commit but before push does not advance recovered state.
6. No `state.json` or `tasks.json` read/write paths remain.
7. Second daemon instance for same repo exits immediately due to lock.
8. Each successful phase boundary creates exactly one structured checkpoint commit and pushes it.
9. Project branch creation/sync uses only remote refs.
10. No prior checkpoint commit starts at loop 1, phase planning.
11. Multi-lifecycle-label issues normalize to `ralph:failed`.
12. Startup resets orphaned `ralph:in-progress` issues to `ralph:ready`.

## Test Plan

### Unit Tests

- Commit message build/parse round-trip.
- Parser rejects malformed subject, missing trailers, and subject/trailer disagreement.
- Label normalization for 0/1/multi lifecycle labels.

### Integration Tests (local git)

- Checkpoint commit subject/trailers pushed to remote.
- Existing remote branch sync and position derivation.
- New branch creation from `origin/HEAD`.
- Crash-after-local-commit-before-push recovery behavior.
- Lock acquisition conflict behavior.
- `.ralph/tmp` cleanup behavior.

### Validate Conformance Tests

Add new validate module(s) and register in `src/validate/mod.rs`. Must include daemon restart/reconcile flows with mocked `gh` behavior. Must include status/history conformance checks against commit+label truth model. All new/changed CLI-visible behavior must be covered by validate tests.

## Rollout Constraints

- No backward compatibility for old in-flight `state.json` projects is required.
- Preserve existing loop artifact file formats.
- Keep changes scoped so daemon-managed worktrees are the only place where local commits may be discarded during startup sync.
