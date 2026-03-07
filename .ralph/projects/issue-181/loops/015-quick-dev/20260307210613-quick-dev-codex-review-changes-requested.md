---
artifact: quick-dev-codex-review
loop: 15
project: issue-181
backend: codex
role: reviewer
created_at: 2026-03-07T21:06:13Z
---

# Review: CHANGES REQUESTED
1. **High: branch-format validation now allows a config that breaks the new branch unification path.**  
[`src/daemon/runtime.rs:1400`](\/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs:1400) only checks that `resolve_branch_name(format, "issue-1") == "ralph/issue-1"`.  
That means a constant format like `ralph/issue-1` passes validation, but dispatch now uses that rendered value for watcher/PR lookup/child state at [`src/daemon/runtime.rs:1424`](\/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs:1424), while `sync_project_branch` still uses `issue_number` (`ralph/issue-{N}`) at [`src/git/branch.rs:115`](\/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/git/branch.rs:115).  
For issue `N != 1`, this recreates a mismatch (wrong head in `find_existing_pr`, watcher push/create on wrong branch, wrong `ChildHandle.branch` for rebase paths).

**Actionable fix**
- Harden daemon validation in [`src/daemon/runtime.rs`](\/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs):
  - Validate at least two IDs, e.g. `issue-1 -> ralph/issue-1` and `issue-2 -> ralph/issue-2`.
  - Or additionally require `branch_format.contains("{project_id}")`.
- Add tests:
  - Unit test in [`src/daemon/runtime.rs`](\/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/runtime.rs) rejecting `git.branch_format = "ralph/issue-1"`.
  - Conformance test in [`src/validate/tests_daemon.rs`](\/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/validate/tests_daemon.rs) asserting daemon startup blocks that config before dispatch.
