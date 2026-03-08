---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T11:39:50Z
---

# Final Review: AMENDMENTS

## Amendment: [P0] Conformance tests still depend on removed `RALPH_DAEMON_BIN` dispatch path

### Problem
The interactive-PRD conformance helper still injects a mock `ralph` binary and reads a captured `--idea` file, but daemon dispatch is now in-process so that hook is never used.  
In [tests_interactive_prd.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5525), [line 5556](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5556), and [line 5560](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5560), the test still assumes subprocess arg capture. Assertions like [line 5704](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs:5704) now fail (`captured_idea == ""`).  
I reproduced this failure with:
`nix develop -c cargo run --quiet -- validate --bin target/debug/ralph --filter interactive_prd::prd_done_missing_markers_fallback`

### Proposed Change
Migrate these PRD-done tests to in-process assertions:
- Remove `RALPH_DAEMON_BIN`/captured-script plumbing from the helper.
- Assert dispatch payload via observable in-process artifacts/logs (or a mock backend configured through workspace config), not child argv capture.
- Update all `run.captured_idea` assertions to use that new signal.

### Affected Files
- [tests_interactive_prd.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_interactive_prd.rs) - remove stale subprocess-capture mechanism and fix failing PRD-done conformance tests.

## Amendment: [P1] Backend child-kill guard is disarmed before async cleanup finishes

### Problem
In [backend/mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:776), `kill_guard.disarm()` happens before timeout/error cleanup branches run ([lines 819-839](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs:819)).  
If cancellation races in while `kill_and_reap_child().await` is in progress, the future is dropped by upstream `tokio::select!` cancellation paths ([orchestrator.rs line 6089](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs:6089), [quick_dev_orchestrator.rs line 1440](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs:1440)). At that point the guard is already disarmed, so hard-kill-on-drop fallback is lost.

### Proposed Change
Keep `KillOnDrop` armed until child termination is guaranteed:
- Disarm only on the success path after child exit is confirmed.
- In timeout/error branches, disarm only after `kill_and_reap_child` completes.
- Add a regression test that cancels during the SIGTERM grace window and asserts the process group is still force-killed.

### Affected Files
- [backend/mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/backend/mod.rs) - reorder/disciplines `KillOnDrop` disarm timing and add race test.
- [orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/orchestrator.rs) - cancellation path reference point for regression test setup.
- [quick_dev_orchestrator.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/workflow/quick_dev_orchestrator.rs) - same cancellation race surface.

## Amendment: [P2] `pr_metadata_verification` test no longer verifies PR metadata

### Problem
`pr_metadata_verification` was reduced to checking dispatch/terminal strings ([tests_e2e_conformance.rs lines 393-407](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:393)).  
It no longer asserts PR create arguments/body content, so it can pass even if PR metadata generation is broken.

### Proposed Change
Restore metadata assertions (title/head/repo/body-file and expected body content) using an execution mode that allows task completion for this test (or inject a deterministic fast backend so completion and PR creation happen in-process).  
If that is not feasible in this test, rename/scope it and add a dedicated metadata-verification conformance case.

### Affected Files
- [tests_e2e_conformance.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs) - restore high-signal PR metadata assertions.
