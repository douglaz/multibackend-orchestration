# Final Review Amendments Applied

## Round 1

### Amendment: FR-20260304-01

### Problem
`git push` retry classification is unsafe and can be wrong because it classifies on the full formatted error string (`err.to_string()`), not just git stderr ([`src/daemon/github.rs:911`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs:911), [`src/daemon/github.rs:1007`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs:1007)).  
Because branch name is embedded in that string, numeric patterns like `"403"`/`"500"` can match branch text and misclassify retryability. Also, unknown errors default to retry (`true`) ([`src/daemon/github.rs:954`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs:954)), which can delay permanent failures instead of failing immediately. Existing tests do not cover this collision/unknown-permanent case ([`src/daemon/github.rs:2212`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs:2212)).

### Proposed Change
Classify retryability from raw push stderr only (not the fully formatted error message), and default unknown errors to non-retryable. Keep explicit transient pattern matching, expand explicit permanent patterns, and add tests for:
1. branch-name numeric collision (`403`/`500`) not affecting classification,
2. permanent unknown-ish errors (e.g. repository-not-found/auth variants) failing without retry.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs) - retry classifier/input source and unit coverage.

### Reviewer
codex

### Amendment: FR-20260304-02

### Problem
The force-kill path in `drain_all_children()` still awaits watcher joins without timeout ([`src/daemon/runtime.rs:2015`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:2015), [`src/daemon/runtime.rs:2021`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:2021)).  
If a watcher is stuck in blocking work, shutdown can still hang and delay/skip `complete_task(..., "ralph:failed", ...)` for that task path ([`src/daemon/runtime.rs:2026`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:2026)).

### Proposed Change
Use `await_watcher_with_timeout(...)` in `drain_all_children()` force-kill teardown as well (after cancellation token signaling), matching the safer behavior already used in `collect_children()` and `kill_aborted_children()`.

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs) - bounded watcher teardown in the shutdown force-kill path.

---

### Reviewer
codex


## Round 2

### Amendment: FR-149-001

### Problem
In `open_log_file_append`, separator preflight failures (`metadata`, `seek`, `read_exact`) are treated as hard errors and can fail command construction ([`src/daemon/process.rs:184`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/process.rs:184), [`src/daemon/process.rs:193`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/process.rs:193), [`src/daemon/process.rs:198`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/process.rs:198)).  
This introduces new failure modes where retrigger can fail even when append logging itself would still work (e.g., transient read/seek issues). For resilience, separator handling should be best-effort.

### Proposed Change
Keep file-open failure fatal, but downgrade separator inspection failures to warnings and continue. If trailing-newline detection fails, fall back to appending a conservative separator (`\n\n--- retrigger at ... ---\n\n`) instead of returning `Err`.

### Affected Files
- `src/daemon/process.rs` - make separator inspection/write path fully best-effort so retrigger still runs.

### Reviewer
codex

### Amendment: FR-149-002

### Problem
`await_watcher_with_timeout_impl_aborts_stuck_task` does not actually prove abort behavior ([`src/daemon/runtime.rs:3693`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:3693)).  
The test spawns a task that sleeps 5 seconds and only asserts after 50ms that it has not completed ([`src/daemon/runtime.rs:3709`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:3709)). That assertion passes even if timeout returns without aborting, so the test can pass for the wrong reason.

### Proposed Change
Rewrite the test so it can distinguish timeout-return from actual abort. Example: spawn a loop that increments an atomic counter every few milliseconds; after helper returns, assert the counter stops changing over an additional observation window.

### Affected Files
- `src/daemon/runtime.rs` - strengthen watcher-timeout test to assert abort side effects, not just early return.

---

### Reviewer
codex

