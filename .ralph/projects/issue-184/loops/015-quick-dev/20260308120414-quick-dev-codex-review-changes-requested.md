---
artifact: quick-dev-codex-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T12:04:14Z
---

# Review: CHANGES REQUESTED
1. **[P1] Cancellation can fail to kill backend descendants in a real code path**  
   In [`src/backend/mod.rs#L790`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs#L790), `kill_guard.disarm()` is called **before** `collect_stderr(...).await` at [`#L795`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs#L795).  
   If cancellation happens while awaiting stderr drain, the future is dropped with the guard disarmed, so descendant processes can survive.  
   **Fix:** keep `kill_guard` armed until all awaited cleanup in the success path is done; disarm only immediately before return with no further `.await`. Also add a bounded stderr-drain timeout and fallback `kill_and_reap_child()`.

2. **[P2] `RALPH_MAX_BACKEND_RETRIES` is still read from env in CLI paths (spec #10 mismatch)**  
   Env parsing remains in:
   - [`src/cli/auto.rs#L13`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/auto.rs#L13)
   - [`src/cli/run.rs#L12`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/run.rs#L12)
   - [`src/cli/quick_dev_auto.rs#L13`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_auto.rs#L13)
   - [`src/cli/quick_dev_run.rs#L13`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/cli/quick_dev_run.rs#L13)  
   This preserves old env-based behavior instead of fully moving retry control to options/config defaults.  
   **Fix:** remove these env readers; pass `max_backend_retries` explicitly via config/args (or `None` for default 3). Update env-based conformance tests such as [`src/validate/tests_e2e_conformance.rs#L95`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs#L95) to validate option/config-driven behavior instead.
