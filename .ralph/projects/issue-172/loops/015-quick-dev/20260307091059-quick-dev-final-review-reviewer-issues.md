---
artifact: quick-dev-final-review
loop: 15
project: issue-172
backend: codex
role: reviewer
created_at: 2026-03-07T09:10:59Z
---

# Final Review: AMENDMENTS

## Amendment: A1

### Problem
`cargo clippy` / `nix build` output can deadlock the check runner and produce false timeouts.

`run_pre_commit_checks` invokes verbose commands through `run_command_with_timeout` ([pre_commit_checks.rs:114](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/workflow/pre_commit_checks.rs:114)).  
`run_command_with_timeout` pipes stdout/stderr, then polls `try_wait`, and only reads output after exit ([process.rs:417](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/daemon/process.rs:417), [process.rs:429](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/daemon/process.rs:429)). If child output fills pipe buffers before exit, child blocks, never reaches exit, and is killed on timeout.  
That can route passing code into repeated “fix” loops and is a robustness/safety issue for this new gate.

### Proposed Change
Drain stdout/stderr concurrently while the process is running (reader threads or equivalent), then enforce timeout on the running child. Return collected output (including partial output on timeout) so feedback remains actionable.

Add a regression test that runs a command producing large output and verifies no false timeout when command exits normally.

### Affected Files
- `[src/daemon/process.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/daemon/process.rs)` - make timeout runner non-blocking with concurrent output draining.
- `[src/workflow/pre_commit_checks.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/workflow/pre_commit_checks.rs)` - consume improved output/timeout behavior.
- `tests` for process/check runner - add high-output timeout regression.

## Amendment: A2

### Problem
`pre_commit_checks::disabled_skips_checks` does not actually prove what it claims.

The test disables fmt/clippy, but its setup project has no `Cargo.toml` ([tests_pre_commit_checks.rs:391](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs:391)), so cargo checks are skipped regardless of config.  
Assertions in [tests_pre_commit_checks.rs:94](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs:94) can pass even if `workflow.pre_commit_fmt` / `workflow.pre_commit_clippy` toggles are broken.

### Proposed Change
Make this test use a Rust fixture (`Cargo.toml` + intentionally fmt/clippy-failing source).  
Then verify:
1. with checks disabled, loop still commits and no pre-commit failure artifact is generated, and
2. with checks enabled (control case), the same fixture triggers pre-commit failure/reloop.

### Affected Files
- `[src/validate/tests_pre_commit_checks.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs)` - strengthen assertions so the test cannot pass for the wrong reason.

---
