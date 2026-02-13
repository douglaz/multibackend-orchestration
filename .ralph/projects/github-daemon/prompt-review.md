---
artifact: prompt-review
project: github-daemon
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-13T18:50:49Z
---

# Prompt Review

## Issues Found
- The “original prompt” is a resolution changelog, not an implementation brief. It states what was fixed but not what must now be built end-to-end.
- Expected user-facing behavior is incomplete: command surface, flags, output format, and failure modes are not fully specified.
- The task lifecycle is only partially implied; explicit state transitions and invariants are missing, which risks inconsistent behavior.
- Persistent data shape (`tasks.json`) is not defined, making compatibility and migration unclear.
- Polling behavior is underspecified (cadence, overflow handling policy, claim rules), which affects correctness under load.
- Recovery semantics across restart/crash are not fully test-defined, which weakens feasibility and reliability.
- Conformance testing intent exists, but exact required cases and assertions are not concretely enumerated.
- Out-of-scope boundaries are incomplete, so downstream loops may overbuild or make conflicting assumptions.

## Refined Prompt
# Implement `ralph daemon` GitHub Task Orchestration

## Objective
Add a daemon workflow that continuously polls GitHub issues, claims eligible work items, runs each item in an isolated git worktree using a real child process, and supports status/abort operations with durable state and conformance coverage.

## Scope
- Implement `ralph daemon start`, `ralph daemon status`, and `ralph daemon abort`.
- Persist daemon task state in `.ralph/daemon/tasks.json`.
- Run each task in its own git worktree at `.ralph/daemon/worktrees/<task-id>/`.
- Use real subprocess execution (`std::process::Command`) to run `ralph auto`.
- Integrate GitHub issue/PR/comment behavior via `gh`.
- Add conformance tests in `src/validate/tests_daemon.rs` and register them in `src/validate/mod.rs`.

## Out of Scope
- Distributed multi-host coordination.
- True multi-repo development from different local checkouts.
- `ralph:paused` state/label.
- UI/TUI dashboards.

## Task Identity and State Model
- Task ID format: `<owner>-<repo>-<number>` (example: `acme-widgets-42`).
- Persisted states: `pending`, `in_progress`, `completed`, `failed`, `aborted`.
- On daemon shutdown/restart reconciliation:
  - Tasks that were `in_progress` become `pending` in `tasks.json`.
  - Keep `ralph:in-progress` label on GitHub.
  - Daemon must re-adopt these tasks on next start.
- `child_pid: Option<u32>` and `child_pgid: Option<u32>` must represent actual OS PID/PGID only.

## Process Execution Requirements
- Do not use `tokio::spawn` for task execution.
- Launch `ralph auto` as a child process with a separate session/process group (`setsid` or equivalent).
- Record child PID/PGID in task state.
- Abort kill policy:
  - Send `SIGTERM` to process group.
  - Wait up to 10 seconds.
  - Escalate to `SIGKILL` if still running.
- If PID is stale/nonexistent, skip kill and continue cleanup/state transition.

## Git Worktree Requirements
- Create a dedicated worktree per task under `.ralph/daemon/worktrees/<task-id>/`.
- `max_concurrent > 1` must be safe because each task runs in its own worktree.
- Clean up worktree on terminal task completion when safe.
- Include startup cleanup/reconciliation for orphaned worktrees/tasks.

## GitHub Polling and Claiming Rules
- Poll issues with `gh issue list --limit 100`.
- Support repeated `--label` filters with AND semantics.
- Ignore issues that already contain any `ralph:*` label.
- If poll returns exactly 100 items, emit an overflow warning (possible truncation).
- Claiming must add `ralph:in-progress`.

## Idempotency Rules
- Every daemon-authored comment must include marker:
  - `<!-- ralph:task:<id>:<phase> -->`
- Before posting a comment, scan existing comments for the exact marker and skip duplicates.
- PR behavior:
  - If no diff: do not open PR; add note/comment (idempotent).
  - Check existing PR first (`gh pr list --head <branch>`).
  - Reuse existing PR URL if present.
  - If PR creation fails, log warning and still move task to terminal completion path per policy.

## Abort Command Semantics
- Accept full task ID or bare issue number.
- Bare issue number must resolve uniquely; otherwise return ambiguity error.
- Abort must work even if daemon is not currently running by operating directly on `tasks.json`.
- Aborting a terminal task returns an error.
- Abort label updates:
  - Remove `ralph:in-progress`.
  - Add `ralph:aborted`.

## Concurrency and Storage Correctness
- `TaskStore` must use file locking (`fs2`):
  - Shared lock for reads.
  - Exclusive lock for write read-modify-write cycle.
- Keep lock scope minimal (only around critical file operations).
- `status` and `abort` must take their own locks safely and independently.

## Required Conformance Tests
Create `src/validate/tests_daemon.rs`, export `pub fn tests() -> Vec<ConformanceTest>`, and register in `register_tests()` in `src/validate/mod.rs`.

Implement at least these 8 tests:
1. `daemon::cli_parse_start_status_abort`
2. `daemon::config_merge_and_defaults`
3. `daemon::start_validates_inputs_and_workspace`
4. `daemon::status_reads_store_with_locking`
5. `daemon::abort_by_full_task_id`
6. `daemon::abort_by_bare_number_ambiguous_error`
7. `daemon::abort_when_daemon_not_running`
8. `daemon::abort_stale_pid_and_terminal_state_handling`

## Acceptance Criteria
- `cargo check` passes.
- `cargo test` passes.
- `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon` passes.
- No regressions in existing validate suites.
- Behavior matches all requirements above with deterministic state transitions and idempotent side effects.
