---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-172
backend: claude
role: implementer
created_at: 2026-03-07T09:31:21Z
---

# Applied fixes for timeout handling regression

All three required changes from the review have been applied to `src/daemon/process.rs`:

### 1. Process group kill instead of single-process kill (lines 421, 460–461)
- Child is now spawned with `.process_group(0)` so it becomes its own process group leader
- On timeout/error, `kill_process_group()` sends `SIGKILL` to the entire process group via `killpg`, killing all descendants that inherited pipe FDs

### 2. No blocking reader thread joins in timeout/error paths (lines 462–465, 472–473)
- Removed `stdout_thread.join()` / `stderr_thread.join()` calls from both the timeout and error branches
- Reader threads are left detached — once the process group is killed, the pipe FDs close and threads terminate naturally
- The success path still joins threads to collect output

### 3. Regression test (lines 1022–1048)
- `run_command_with_timeout_kills_group_on_timeout` spawns `sh -c "sleep 60 & echo started; wait"` — a shell that starts a long-lived background child inheriting pipe FDs
- Asserts the function returns a timeout error within 10 seconds (well under the 60s that the old code would block)
- Test passes in exactly 2.0s (matching the 2s timeout)

### Verification
- New regression test: **passes** (2.0s)
- Existing high-output test: **passes**
- All 10 `daemon_rebase_agent` tests: **pass** (the four previously failing tests now work correctly)
