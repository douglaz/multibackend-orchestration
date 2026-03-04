## Summary

Three independent resilience gaps exist in the daemon's child-process lifecycle. Each must be fixed independently without altering unrelated code.

1. **Log files are truncated on retrigger.** `build_ralph_auto_command()` and `build_ralph_run_command()` in `src/daemon/process.rs` use `File::create()`, which truncates existing log files. When an issue is retriggered, the previous run's output is lost.

2. **Push failures in the PR flow are not retried, and errors are swallowed.** `github::push_branch()` (`src/daemon/github.rs:905-921`) has no retry logic. In `handle_pr_flow()` (`src/daemon/runtime.rs:3020-3026`), push errors are converted to a warning and `Ok(())`, so even after adding retry the failure is never communicated to the caller. In `draft_pr_watcher_with_sleep()` (`src/daemon/runtime.rs:290`), push errors gate PR creation correctly but are not retried. Both call sites must use retry for transient failures, and `handle_pr_flow()` must propagate the final error rather than swallowing it.

3. **Watcher task teardown has no bounded shutdown guarantee.** `collect_children()` (`src/daemon/runtime.rs:1770-1781`) and `kill_aborted_children()` (`src/daemon/runtime.rs:1940-1951`) cancel watcher `CancellationToken`s then `await` their `JoinHandle`s with no timeout. Watcher loops contain `spawn_blocking_op` calls (GitHub API via `gh`, git operations) that run on the tokio blocking threadpool. The cancellation token is only checked in the `tokio::select!` at the end of each loop iteration; if a watcher is mid-execution of a blocking operation, cancellation is not observed until that operation completes — which may take arbitrarily long if the network is hung or GitHub is unresponsive. This can delay `complete_task()` and the `ralph:in-progress` → `ralph:failed` label transition indefinitely.

## Acceptance Criteria

- [ ] Log file for a retriggered issue contains output from all runs, with a timestamped separator between runs
- [ ] Separator write failures produce a warning log (not silent `let _ =` discard)
- [ ] A simulated transient push error (5xx / network timeout) triggers retry logic with exponential backoff and does not immediately fail the run
- [ ] A permanent push failure (e.g. auth error, non-fast-forward rejection) does not retry and fails immediately
- [ ] After retry exhaustion in `handle_pr_flow()`, the push error propagates to `complete_task_attempt()` (not swallowed as `Ok(())`); `complete_task_attempt` logs it as best-effort and proceeds to label swap
- [ ] When a child exits non-zero, watcher join awaits are bounded by a timeout; `collect_children()` proceeds to `complete_task()` even if a watcher is stuck in a blocking operation
- [ ] The label transition from `ralph:in-progress` to `ralph:failed` happens automatically without manual intervention, even when watcher teardown times out
- [ ] `cargo check` passes with no new warnings
- [ ] Automated unit tests cover: `is_retryable_push_error` for transient vs. permanent classification, `push_branch_with_retry_impl` retry/no-retry paths via mock git binary, and the append-mode log separator

## Technical Approach

### 1. Append logs on retrigger

**File:** `src/daemon/process.rs`

In `build_ralph_auto_command()` (line 121) and `build_ralph_run_command()` (line 155), replace:

```rust
let file = std::fs::File::create(log_file).map_err(|err| {
    RalphError::Orchestration(format!(
        "failed to create log file {}: {err}",
        log_file.display()
    ))
})?;
```

with:

```rust
use std::fs::OpenOptions;
use std::io::Write;

let file = OpenOptions::new()
    .create(true)
    .append(true)
    .open(log_file)
    .map_err(|err| {
        RalphError::Orchestration(format!(
            "failed to open log file {}: {err}",
            log_file.display()
        ))
    })?;

// Write separator for retriggered runs (file already has content from previous run)
if file.metadata().map(|m| m.len() > 0).unwrap_or(false) {
    let mut separator_file = file.try_clone().map_err(|err| {
        RalphError::Orchestration(format!("failed to clone log file handle: {err}"))
    })?;
    if let Err(err) = writeln!(
        separator_file,
        "\n--- retrigger at {} ---\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    ) {
        eprintln!(
            "warning: failed to write retrigger separator to {}: {err}",
            log_file.display()
        );
    }
}
```

**Rationale:** `OpenOptions::append(true)` preserves existing content. The `metadata().len()` check avoids writing a separator on the first run. Separator write errors are logged as warnings rather than silently ignored (`let _ =`), satisfying the observability requirement without failing command setup. The project already depends on `chrono` (`Cargo.toml` line 11).

### 2. Retry transient push failures

**File:** `src/daemon/github.rs`

Add a transient-error classifier for git push errors, alongside `is_retryable_gh_error`:

```rust
/// Determine if a git-push error is transient and worth retrying.
/// Checks the stringified error, which embeds git's stderr output.
fn is_retryable_push_error(err: &RalphError) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    // Transient: HTTP 5xx, network issues, timeouts
    message.contains("500")
        || message.contains("502")
        || message.contains("503")
        || message.contains("504")
        || message.contains("timeout")
        || message.contains("timed out")
        || message.contains("connection")
        || message.contains("network")
        || message.contains("could not resolve")
        || message.contains("unable to access")
        || message.contains("the remote end hung up")
    // NOT retryable (absent from list, so fall through to false):
    // "non-fast-forward", "denied", "authentication", "Permission denied",
    // "protected branch hook declined"
}
```

**Why not reuse `is_retryable_gh_error` or `RalphError::is_transient()`:** `is_retryable_gh_error` is tuned for `gh` CLI stderr and includes patterns irrelevant to git push ("409", "conflict", "api rate"). `RalphError::is_transient()` uses a broad `_ => true` fallback for non-`Orchestration` variants and includes patterns like "failed to execute" / "subprocess" that should not trigger a push retry. A dedicated classifier avoids false positives.

Add `push_branch_with_retry()` alongside the existing `push_branch()`:

```rust
const PUSH_RETRY_MAX: u32 = 3;
const PUSH_RETRY_DELAYS_SECS: [u64; 3] = [10, 30, 60];

pub fn push_branch_with_retry(worktree_path: &std::path::Path, branch: &str) -> Result<()> {
    push_branch_with_retry_impl("git", worktree_path, branch, &PUSH_RETRY_DELAYS_SECS)
}

/// Internal implementation accepting injectable git binary and delay schedule
/// for deterministic testing.
fn push_branch_with_retry_impl(
    git_bin: &str,
    worktree_path: &std::path::Path,
    branch: &str,
    delays_secs: &[u64],
) -> Result<()> {
    let max_attempts = (delays_secs.len() as u32) + 1;
    for attempt in 0..max_attempts {
        let output = Command::new(git_bin)
            .args(["push", "-u", "origin", branch])
            .current_dir(worktree_path)
            .output()
            .map_err(|err| {
                RalphError::Orchestration(format!("failed to run git push: {err}"))
            })?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let err = RalphError::Orchestration(format!(
            "git push failed for branch {branch}: {stderr}"
        ));

        if attempt < delays_secs.len() as u32 && is_retryable_push_error(&err) {
            let delay = Duration::from_secs(delays_secs[attempt as usize]);
            eprintln!(
                "push-retry: push failed for branch {branch} (attempt {}/{}), \
                 retrying in {}s: {stderr}",
                attempt + 1,
                max_attempts,
                delay.as_secs()
            );
            thread::sleep(delay);
            continue;
        }

        return Err(err);
    }
    unreachable!()
}
```

**Design notes:**

- **`_impl` variant for testability:** `push_branch_with_retry_impl` accepts `git_bin` and `delays_secs`, enabling tests with a mock git binary and zero-length delays. This follows the same pattern as `add_label_with_retry_with_gh_bin` (github.rs:1224).
- **Inlined command execution:** The `_impl` function inlines the `git push` command rather than calling `push_branch()`, consistent with how `add_label_with_retry_with_gh_bin` inlines its `gh` command rather than calling a base function. This avoids indirect error-message wrapping.
- **Existing `push_branch()` unchanged:** Retained for any callers that do not need retry.

**Callers to update:**

1. **`handle_pr_flow()`** in `src/daemon/runtime.rs` (~line 3016-3027) — change to `push_branch_with_retry` **and propagate errors instead of swallowing**:

   Before:
   ```rust
   // Step 2: Push branch
   {
       let wt = wt_path.to_path_buf();
       let br = branch.clone();
       match spawn_blocking_op(move || github::push_branch(&wt, &br)).await {
           Ok(()) => {}
           Err(err) => {
               eprintln!("warning: failed to push branch {branch} for {task_id}: {err}");
               return Ok(());
           }
       }
   }
   ```

   After:
   ```rust
   // Step 2: Push branch (retries transient failures; propagates final error)
   {
       let wt = wt_path.to_path_buf();
       let br = branch.clone();
       spawn_blocking_op(move || github::push_branch_with_retry(&wt, &br)).await?;
   }
   ```

   **Behavioral impact:** After retry exhaustion, the push error propagates from `handle_pr_flow()` via `?`. The caller in `complete_task_attempt()` (runtime.rs:2125-2127) already catches `handle_pr_flow` errors as best-effort:
   ```rust
   if let Err(err) = handle_pr_flow(config, task_id, issue_number, &wt_path).await {
       eprintln!("warning: PR flow failed for {task_id} (best-effort, continuing to label swap): {err}");
   }
   ```
   The error is logged, and execution proceeds to `swap_lifecycle_label()`. The label transition is unaffected. The change makes the failure visible in logs rather than silently returning `Ok(())` with only the push-level warning.

2. **`draft_pr_watcher_with_sleep()`** in `src/daemon/runtime.rs` (~line 290) — change `github::push_branch` to `github::push_branch_with_retry`. The existing error handling (gate PR creation on `push_ok`, continue loop on failure) is already correct; retry adds transient-failure resilience within each attempt.

### 3. Bounded watcher teardown on child death

**Problem:** `collect_children()` (runtime.rs:1770-1781) and `kill_aborted_children()` (runtime.rs:1940-1951) cancel watcher tokens then `await` join handles with no timeout. Watcher loops perform `spawn_blocking_op` calls (GitHub API via `gh`, git operations) that run on tokio's blocking threadpool. `CancellationToken::cancel()` is only observed at the `tokio::select!` between loop iterations. If a watcher is mid-`spawn_blocking_op`, cancellation is deferred until that blocking operation completes — potentially indefinitely if the network is hung. The unbounded `await` on the join handle then blocks `collect_children()`, delaying `complete_task()` and the label transition.

**File:** `src/daemon/runtime.rs`

Add a teardown timeout constant alongside the existing constants (near line 99):

```rust
/// Maximum time to wait for a watcher task to stop after its cancellation
/// token is signalled. If exceeded, the task is aborted so collect_children
/// can proceed to complete_task without unbounded delay.
const WATCHER_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(30);
```

**Extract a helper** to avoid duplicating the timeout pattern across `collect_children()` and `kill_aborted_children()`:

```rust
/// Await a watcher join handle with a bounded timeout.  If the watcher does
/// not stop within `WATCHER_TEARDOWN_TIMEOUT`, abort it and move on.
async fn await_watcher_with_timeout(
    join_handle: tokio::task::JoinHandle<()>,
    watcher_name: &str,
    task_id: &str,
) {
    let abort_handle = join_handle.abort_handle();
    match tokio::time::timeout(WATCHER_TEARDOWN_TIMEOUT, join_handle).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            eprintln!("warning: {watcher_name} join failed for {task_id}: {err}");
        }
        Err(_elapsed) => {
            abort_handle.abort();
            eprintln!(
                "warning: {watcher_name} for {task_id} did not stop within {}s, aborted",
                WATCHER_TEARDOWN_TIMEOUT.as_secs()
            );
        }
    }
}
```

**Key design:** `JoinHandle::abort_handle()` is called _before_ passing the handle into `timeout`, capturing an `AbortHandle` that remains valid after `timeout` consumes the `JoinHandle`. On timeout, `abort_handle.abort()` cancels the tokio task. Any in-flight `spawn_blocking` op on the blocking threadpool will run to completion independently, but the watcher's async task is terminated and its resources freed. (`JoinHandle::abort_handle` is stable since tokio 1.21; the project uses `tokio = "1"` with `features = ["full"]`.)

**Update `collect_children()`** (runtime.rs:1770-1781):

Before:
```rust
handle.watcher_cancel.cancel();
if let Some(join_handle) = handle.watcher_handle.take() {
    if let Err(err) = join_handle.await {
        eprintln!("warning: artifact watcher join failed for {task_id}: {err}");
    }
}
handle.draft_pr_cancel.cancel();
if let Some(join_handle) = handle.draft_pr_handle.take() {
    if let Err(err) = join_handle.await {
        eprintln!("warning: draft PR watcher join failed for {task_id}: {err}");
    }
}
```

After:
```rust
handle.watcher_cancel.cancel();
if let Some(join_handle) = handle.watcher_handle.take() {
    await_watcher_with_timeout(join_handle, "artifact watcher", &task_id).await;
}
handle.draft_pr_cancel.cancel();
if let Some(join_handle) = handle.draft_pr_handle.take() {
    await_watcher_with_timeout(join_handle, "draft PR watcher", &task_id).await;
}
```

**Update `kill_aborted_children()`** (runtime.rs:1940-1951) — same transformation.

**Guarantee:** With this change, `collect_children()` proceeds to `complete_task()` within at most 60 seconds of watcher teardown (30s per watcher × 2 watchers), regardless of blocking-operation hangs. `complete_task()` then transitions the label via `swap_lifecycle_label()` as it does today.

## Files & Modules

| File | Change | Lines affected |
|------|--------|----------------|
| `src/daemon/process.rs` | Replace `File::create()` with `OpenOptions::new().append(true)` + separator with error logging in `build_ralph_auto_command()` and `build_ralph_run_command()` | ~121, ~155 |
| `src/daemon/github.rs` | Add `push_branch_with_retry()`, `push_branch_with_retry_impl()`, `is_retryable_push_error()`, and unit tests | After line 921, tests in `#[cfg(test)]` |
| `src/daemon/runtime.rs` | Add `WATCHER_TEARDOWN_TIMEOUT` constant and `await_watcher_with_timeout()` helper; update `collect_children()` and `kill_aborted_children()` to use bounded await; update `handle_pr_flow()` and `draft_pr_watcher_with_sleep()` push call sites | ~99, ~290, ~1770-1781, ~1940-1951, ~3016-3027 |

## Testing Strategy

### Unit tests (in-file `#[cfg(test)]`)

**1. `github.rs` — `is_retryable_push_error()` classification (deterministic):**

- Assert `true` for `RalphError::Orchestration` containing: `"502"`, `"503"`, `"504"`, `"timeout"`, `"connection refused"`, `"the remote end hung up unexpectedly"`, `"unable to access"`, `"could not resolve host"`
- Assert `false` for messages containing: `"non-fast-forward"`, `"Permission denied (publickey)"`, `"authentication failed"`, `"protected branch hook declined"`
- Follows the existing `is_retryable_gh_error` test pattern (github.rs:2183-2207)

**2. `github.rs` — `push_branch_with_retry_impl()` transient retry (deterministic, fast):**

Create a temporary directory with a mock `git` shell script that:
- On invocations 1-2: writes "error: 503 Service Unavailable" to stderr and exits 1
- On invocation 3: exits 0

Invoke `push_branch_with_retry_impl(mock_git_path, &tmpdir, "test-branch", &[0, 0, 0])` (zero-second delays for fast tests). Assert the call succeeds. Verify by checking the mock script's invocation counter (e.g. a counter file incremented by the script) that exactly 3 invocations occurred.

**3. `github.rs` — `push_branch_with_retry_impl()` permanent failure (deterministic, fast):**

Mock `git` script always writes "Permission denied (publickey)" to stderr and exits 1. Invoke with `delays_secs = &[0, 0, 0]`. Assert the call returns `Err` after exactly 1 invocation (no retries for permanent errors).

**4. `github.rs` — `push_branch_with_retry_impl()` transient exhaustion (deterministic, fast):**

Mock `git` script always writes "error: 503 Service Unavailable" to stderr and exits 1. Invoke with `delays_secs = &[0, 0]` (2 retries = 3 total attempts). Assert the call returns `Err` after exactly 3 invocations. Assert error message contains "503".

**5. `process.rs` — append-mode log separator:**

Create a temp file with pre-existing content ("first run output"). Call the log-file-opening logic (extracted into a testable helper or tested via `build_ralph_auto_command` with a temp path). Verify the file now contains the original content followed by a `--- retrigger at <timestamp> ---` separator. Call again and verify a second separator is appended. Verify the timestamp matches `\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z` format.

### Integration / smoke testing

- Trigger an issue, wait for the child to start logging, then retrigger the same issue. Inspect the log file in `.ralph/tmp/logs/<task_id>.log` — it should contain a `--- retrigger at ... ---` separator between runs.
- Verify `cargo check` passes with zero new warnings.

### What is NOT tested (and why)

- **Watcher teardown timeout:** The `await_watcher_with_timeout` helper is simple glue over `tokio::time::timeout` and `AbortHandle::abort`, both well-tested by tokio. A unit test would require constructing a mock `JoinHandle` (which tokio does not expose a constructor for). The timeout constant (30s) is verified by code review. If future needs warrant it, an integration test can spawn a deliberately-stuck watcher and assert the timeout fires.
- **End-to-end push retry with real network faults:** Covered by the deterministic mock-git tests above. Real network fault injection remains a manual validation option but is not required for CI.

## Out of Scope

- Refactoring the `complete_task` retry mechanism or its delay constants
- Adding configuration knobs for push retry counts, delays, or watcher teardown timeout
- Modifying the `is_transient()` method on `RalphError`
- Adding per-operation timeouts to `spawn_blocking_op` itself (addresses the symptom via teardown timeout instead)
- Durable persistence of daemon state
- Config schema changes