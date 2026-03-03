---
artifact: final-review-proposals
loop: 3
project: issue-105
backend: codex
role: final_reviewer
created_at: 2026-03-03T18:39:28Z
---

Review started with a full diff audit against the base branch, then I’ll trace the modified runtime and tests for correctness/safety, especially concurrency and shutdown paths. I’m running branch/diff commands first to identify exactly what changed.
I found the branch context and there are only untracked `.ralph` artifacts, which I’ll ignore per your exclude rule. Next I’m diffing `origin/master...HEAD` and then reading each changed source/test file in detail.
I’ve identified 11 modified files with a large `runtime.rs` rewrite. I’m now walking the runtime diff and function bodies first, then I’ll verify tests actually assert the intended behavior.
I’ve confirmed the main concurrency refactor points in `runtime.rs` and now I’m tracing the actual current code paths (not just the patch) around shutdown, dispatch, collect, rebase, and git-lock usage to validate invariants and failure handling.
I’ve completed the first pass on runtime internals and am now auditing supporting files (`worktree`, `bootstrap`, config/CLI, and validate tests) to catch cross-file safety regressions and test-quality gaps.
Next I’m reviewing the validate harness and new daemon-concurrency conformance tests, focusing on whether they assert the actual behavior claims or can pass for the wrong reason.
I’m now checking runtime config plumbing end-to-end (`DaemonRuntimeConfig` creation paths, defaults, validation, and tests) and then I’ll run targeted test commands to confirm the new paths behave as expected.
I’ve finished static inspection of the changed paths and identified a few panic-path consistency concerns to verify. I’m running tests/build checks now to confirm whether these issues are surfaced and to validate the new conformance module compiles cleanly.
`cargo` isn’t installed directly in this environment, so I’m switching to the project’s expected `nix develop -c ...` workflow for verification commands.
Unit/integration tests passed under `nix develop`. I’m running `nix build -L` next, then I’ll run validate checks for the daemon concurrency suite to ensure the new conformance tests actually execute successfully.
I’ve confirmed the new validate tests are registered and now running the `daemon_concurrency` subset directly; I’ll report any failures with exact test names.
I found a likely test-quality issue in `partial_dispatch_rollback`: its failure injection appears to make the child exit after successful dispatch, not fail `dispatch_task` itself. I’m running that one validate test in verbose mode to confirm the actual path exercised.
I’ve confirmed the main failing points and now I’m finalizing amendment-level findings with exact line references: two panic-path state-invariant gaps in runtime and one conformance-test gap where the test does not exercise the code path it claims.
# Final Review: AMENDMENTS

## Amendment: RVW-DAEMON-001

### Problem
Claimed issues can be left stuck in `ralph:in-progress` if a dispatch worker panics.  
In [`runtime.rs:1122`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs:1122), issues are claimed (`ready -> in-progress`) before dispatch starts.  
In [`runtime.rs:1223`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs:1223), JoinSet panic handling only logs and does not roll back labels.  
That violates state recovery invariants: the issue is no longer in `children`, won’t be collected, and won’t be repolled because it is not `ralph:ready`.

### Proposed Change
Capture per-issue panic outcomes in dispatch workers and apply the same rollback path used for `Err` results (`in-progress -> failed`).  
Ensure panic/error outcomes always carry `issue_number` so rollback is deterministic.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs` - make dispatch panic path label-safe and per-issue recoverable.

## Amendment: RVW-DAEMON-002

### Problem
Finished-child completion panic handling can silently drop terminal-state persistence.  
In [`runtime.rs:1660`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs:1660), the child is removed from `children` before completion.  
In [`runtime.rs:1695`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs:1695), panic handling only logs (`complete_task panicked`) and continues.  
If `complete_task` panics before lifecycle swap, the issue can remain `ralph:in-progress` with no remaining worker reference to recover it in-loop.

### Proposed Change
Return structured completion outcomes from JoinSet tasks (including `issue_number`/`task_id`) and treat panic as a failure that triggers explicit fallback label transition (at minimum to `ralph:failed`).  
Do not leave panic handling as log-only for terminal-state transitions.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs` - harden completion panic path to preserve lifecycle invariants.

## Amendment: RVW-DAEMON-003

### Problem
New conformance tests do not reliably prove the behaviors they claim.  
In [`tests_daemon_concurrency.rs:107`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/validate/tests_daemon_concurrency.rs:107), `concurrent_dispatch_two_issues` asserts both issues dispatched, but does not prove concurrent dispatch (sequential code would also pass).  
In [`tests_daemon_concurrency.rs:201`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/validate/tests_daemon_concurrency.rs:201), `partial_dispatch_rollback` uses a mock `ralph` script that exits non-zero; that typically exercises post-dispatch child failure, not `dispatch_task` failure.  
In [`tests_daemon_concurrency.rs:244`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/validate/tests_daemon_concurrency.rs:244), assertion `contains("failed to dispatch issue #301") || contains("301")` is too weak and can pass for unrelated output.

### Proposed Change
Strengthen tests to assert the actual code paths:
1. Add deterministic pre-spawn dispatch failure injection for one issue (so rollback is from dispatch failure path, not child terminal path).
2. Tighten assertions to require explicit dispatch-failure markers and per-issue rollback transitions.
3. Prove concurrency via timing/barrier evidence (not just “both dispatched”).

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/validate/tests_daemon_concurrency.rs` - strengthen behavioral assertions.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/daemon/runtime.rs` - optionally add test-only deterministic dispatch-failure hook.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-105/src/validate/mock_scripts.rs` - support stronger concurrency/failure instrumentation if needed.

---

## Context Provided
- Reviewed full changes via `git diff origin/master...HEAD -- . ':(exclude).ralph'`.
- Ran `nix develop -c cargo test -q` (pass).
- Ran `nix build -L` (pass).
- Ran `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon_concurrency --jobs 1` (4/4 pass).
