---
artifact: quick-dev-final-review
loop: 15
project: issue-181
backend: codex
role: reviewer
created_at: 2026-03-07T20:57:57Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Enforce daemon branch format to vary per issue

### Problem
`dispatch_task` now derives `branch_name` from `git.branch_format` ([runtime.rs:1424](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs:1424)), but `sync_project_branch` still operates on `ralph/issue-{issue_number}` ([branch.rs:115](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/git/branch.rs:115)).  
The daemon-side validation only checks one rendered value (`issue-1`) ([runtime.rs:1400](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs:1400)), so a constant format like `"ralph/issue-1"` incorrectly passes. For issue `N != 1`, that causes PR lookup/watcher/`ChildHandle.branch` to use `ralph/issue-1` while sync/commits use `ralph/issue-N`, reintroducing branch mismatch behavior.

### Proposed Change
Strengthen `validate_daemon_branch_format` to enforce issue-dependent rendering:
1. Require `"{project_id}"` to be present.
2. Validate at least two project IDs (e.g., `issue-1 -> ralph/issue-1` and `issue-2 -> ralph/issue-2`), rejecting constant outputs.
3. Add tests to reject `"ralph/issue-1"` in both unit validation and daemon conformance startup checks.

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs) - harden `validate_daemon_branch_format` and add unit coverage.
- [src/validate/tests_daemon.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/validate/tests_daemon.rs) - extend startup validation conformance test for constant branch format rejection.
