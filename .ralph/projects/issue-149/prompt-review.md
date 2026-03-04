---
artifact: prompt-review
project: issue-149
backend: codex
role: prompt_reviewer
created_at: 2026-03-04T02:03:46Z
---

# Prompt Review

## Issues Found
- Backoff behavior is underspecified: acceptance says “exponential backoff” but proposed delays `10,30,60` are not exponential, which can yield inconsistent implementations.
- Retry attempt count is ambiguous: `PUSH_RETRY_MAX=3` conflicts with `delays.len() + 1` logic, so total attempts are unclear.
- Error propagation expectations are split and easy to miss: `handle_pr_flow()` must return `Err` after retry exhaustion, while `complete_task_attempt()` still continues label swap.
- Log separator behavior is not fully deterministic: exact format, insertion timing, and one-separator-per-retrigger expectations should be explicit for reliable tests.
- Watcher shutdown guarantee lacks a hard numeric bound in acceptance criteria, which weakens testability.
- Test plan omits direct verification of runtime propagation and watcher-timeout behavior, leaving high-risk paths weakly covered.
- Repo conformance guidance is not reflected; prompt should explicitly state when `validate` tests are required for these behavior changes.

## Refined Prompt
Implement three independent daemon resilience fixes with minimal surface-area changes. Do not refactor unrelated code.

### Objective
Fix:
1. Log truncation on retrigger.
2. Missing retry and swallowed errors in git push during PR flow.
3. Unbounded watcher shutdown waits that can block failure completion.

### Scope
- Edit only:
  - `src/daemon/process.rs`
  - `src/daemon/github.rs`
  - `src/daemon/runtime.rs`
- Keep existing behavior unchanged outside these paths.
- No config-schema changes and no new CLI commands.

### Required Changes

#### 1. Preserve logs across retriggers
- In `build_ralph_auto_command()` and `build_ralph_run_command()` in `src/daemon/process.rs`:
  - Replace `File::create(log_file)` with append mode (`OpenOptions::new().create(true).append(true)`).
  - If the log file already has content, append a separator before new run output.
  - Separator format must be exactly: `--- retrigger at <UTC timestamp> ---` on its own line, with blank lines around it.
  - Timestamp format: `YYYY-MM-DDTHH:MM:SSZ` (UTC).
  - If separator write fails, emit a warning to stderr including file path and error.
  - Separator write failure must not fail command construction.

#### 2. Retry transient git push failures and propagate final failure
- In `src/daemon/github.rs`:
  - Add `is_retryable_push_error(err: &RalphError) -> bool`.
  - Retryable examples: HTTP 5xx, timeout, connection/network/DNS/access transient failures.
  - Non-retryable examples: auth/permission denied, non-fast-forward, protected-branch/policy rejection.
  - Add:
    - `pub fn push_branch_with_retry(worktree_path: &Path, branch: &str) -> Result<()>`
    - `fn push_branch_with_retry_impl(git_bin: &str, worktree_path: &Path, branch: &str, delays_secs: &[u64]) -> Result<()>`
  - Backoff schedule must be explicit and deterministic: `[10, 20, 40]` seconds (4 total attempts including initial attempt).
  - Keep existing `push_branch()` unless removal is proven safe.

- In `src/daemon/runtime.rs`:
  - `handle_pr_flow()`: switch to `push_branch_with_retry` and propagate failure (`?`), do not convert push failure to `Ok(())`.
  - `draft_pr_watcher_with_sleep()`: switch to `push_branch_with_retry`; keep existing gating behavior (no PR creation when push fails).

#### 3. Bound watcher teardown time
- In `src/daemon/runtime.rs`:
  - Add `const WATCHER_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(30);`
  - Add helper `await_watcher_with_timeout(join_handle, watcher_name, task_id)`:
    - call `timeout(WATCHER_TEARDOWN_TIMEOUT, join_handle).await`
    - on join error: warning log
    - on timeout: abort task and warning log, then continue
  - Use helper in both:
    - `collect_children()`
    - `kill_aborted_children()`
  - Keep cancellation-token signaling before waiting on join handles.

### Acceptance Criteria
- Retriggered issue logs preserve prior output and include one timestamped separator per new run.
- Separator write failure is visible as warning log (not silently discarded).
- Transient push failures retry with the defined backoff schedule and can recover.
- Permanent push failures do not retry and fail immediately.
- After retry exhaustion, `handle_pr_flow()` returns `Err`; caller logs best-effort warning and still performs lifecycle label swap.
- Child non-zero exit cannot block indefinitely on watcher teardown; completion path proceeds even if watcher is stuck in blocking work.
- `ralph:in-progress` to `ralph:failed` transition still occurs automatically in the timeout case.
- `cargo check` passes with no new warnings.

### Tests Required
- Unit tests in `src/daemon/github.rs`:
  - `is_retryable_push_error` transient vs permanent classification.
  - `push_branch_with_retry_impl` transient-then-success path with mock git binary and zero delays.
  - `push_branch_with_retry_impl` permanent-failure no-retry path.
  - `push_branch_with_retry_impl` transient retry exhaustion path.
- Unit test for append-mode separator behavior (in `process.rs`, via helper or direct callable path).
- Add runtime test for watcher-timeout helper if feasible with tokio test-time controls; if not feasible, document why and cover via targeted integration behavior.
- If any user-visible CLI behavior changes, add/extend `validate` conformance tests under `src/validate/` per project policy.

### Execution and Verification
- Run:
  - `nix develop -c cargo check`
  - `nix develop -c cargo test`
- Report:
  - files changed
  - key behavior changes
  - test coverage added and command results
