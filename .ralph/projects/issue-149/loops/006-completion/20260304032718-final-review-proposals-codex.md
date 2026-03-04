---
artifact: final-review-proposals
loop: 6
project: issue-149
backend: codex
role: final_reviewer
created_at: 2026-03-04T03:27:18Z
---

Reviewing this as a final audit focused on correctness/safety. I’ll start by identifying the base branch, run the requested `git diff <base>...HEAD -- . ':(exclude).ralph'`, then read the touched files and tests for behavioral gaps.
I found the branch topology (`ralph/issue-149` off `master`), so I’ll use `master` as base and inspect the exact triple-dot diff next.
I’ve captured the full diff and now I’m tracing the touched code paths in `process.rs`, `github.rs`, and `runtime.rs` (including tests) to validate behavior and look for hidden failure modes.
Next I’m running compile/test commands to confirm these changes actually build and the new tests execute as expected, then I’ll look for logic holes not covered by tests.
I’m now cross-checking call sites and test intent to catch hidden regressions (for example, whether retries are actually used everywhere needed, and whether any tests pass for the wrong reason).
Full `cargo test` is running and emitting lots of integration output; I’m waiting for completion and then I’ll finish the defect audit with concrete amendment decisions.
The test suite is still in progress; once it exits I’ll consolidate the concrete findings with file/line references and whether amendments are needed.
# Final Review: AMENDMENTS

## Amendment: FR-149-001

### Problem
In `open_log_file_append`, separator preflight failures (`metadata`, `seek`, `read_exact`) are treated as hard errors and can fail command construction ([`src/daemon/process.rs:184`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/process.rs:184), [`src/daemon/process.rs:193`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/process.rs:193), [`src/daemon/process.rs:198`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/process.rs:198)).  
This introduces new failure modes where retrigger can fail even when append logging itself would still work (e.g., transient read/seek issues). For resilience, separator handling should be best-effort.

### Proposed Change
Keep file-open failure fatal, but downgrade separator inspection failures to warnings and continue. If trailing-newline detection fails, fall back to appending a conservative separator (`\n\n--- retrigger at ... ---\n\n`) instead of returning `Err`.

### Affected Files
- `src/daemon/process.rs` - make separator inspection/write path fully best-effort so retrigger still runs.

## Amendment: FR-149-002

### Problem
`await_watcher_with_timeout_impl_aborts_stuck_task` does not actually prove abort behavior ([`src/daemon/runtime.rs:3693`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:3693)).  
The test spawns a task that sleeps 5 seconds and only asserts after 50ms that it has not completed ([`src/daemon/runtime.rs:3709`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/runtime.rs:3709)). That assertion passes even if timeout returns without aborting, so the test can pass for the wrong reason.

### Proposed Change
Rewrite the test so it can distinguish timeout-return from actual abort. Example: spawn a loop that increments an atomic counter every few milliseconds; after helper returns, assert the counter stops changing over an additional observation window.

### Affected Files
- `src/daemon/runtime.rs` - strengthen watcher-timeout test to assert abort side effects, not just early return.

---
