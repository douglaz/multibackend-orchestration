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

