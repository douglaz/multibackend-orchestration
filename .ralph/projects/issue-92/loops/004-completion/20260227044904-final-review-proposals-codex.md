---
artifact: final-review-proposals
loop: 4
project: issue-92
backend: codex
role: final_reviewer
created_at: 2026-02-27T04:49:04Z
---

Reviewing the completed changes as a final audit now. I’ll start by identifying the correct base branch, run the required `git diff ... -- . ':(exclude).ralph'`, and then inspect changed source/tests for correctness, safety, and robustness issues.
I found this branch is `ralph/issue-92` and it tracks `origin/master`. Next I’ll run the required triple-dot diff against `master` and inspect all touched files plus related tests.
I’ve narrowed changes to three files. I’m now reading the updated workflow functions in `interactive_prd.rs` in full context to validate ordering, error-path behavior, and whether waiting-label reconciliation is applied on every required path.
I finished first-pass review of the core PRD state machine and terminal/error ordering. Next I’m auditing the new conformance/integration tests line-by-line to verify they actually prove the waiting-label behavior and don’t pass for incidental reasons.
I’m moving from static review to execution checks now. I’ll run the Rust test suite to catch any behavioral regressions and then spot-check repository status for stray files outside `.ralph`.
The full `cargo test` run is mostly green but has one pre-existing environment-sensitive failure (`dedup_invariant_issue_processed_at_most_once`) unrelated to this label change. I’m now running targeted tests around the new waiting-label behavior to confirm these specific changes pass.
# Final Review: AMENDMENTS

## Amendment: FR-PRD-WAITING-001

### Problem
`ralph:waiting-feedback` reconciliation in waiting states is currently gated behind bot-login resolution.  
In both waiting-state transitions, `get_or_fetch_bot_login(...)` runs before the helper is reached (`[interactive_prd.rs#L1151](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1151)`, `[interactive_prd.rs#L1163](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1163)`, `[interactive_prd.rs#L1179](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1179)`, `[interactive_prd.rs#L1293](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1293)`, `[interactive_prd.rs#L1305](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1305)`, `[interactive_prd.rs#L1320](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1320)`).  
If `gh api user` fails, waiting-state ticks take the retry/error path without any waiting-label reconciliation.

Test coverage misses this gap: the bot-login-failure waiting-state tests log label edits but only assert `ralph:prd-failed`, not waiting-label add (`[daemon_interactive_prd.rs#L1621](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs#L1621)`, `[daemon_interactive_prd.rs#L1766](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs#L1766)`, `[tests_interactive_prd.rs#L2622](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs#L2622)`, `[tests_interactive_prd.rs#L2736](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs#L2736)`).

### Proposed Change
Move waiting-label reconciliation to the start of waiting-state transition wrappers, before bot-login lookup, so it always runs on no-op, processing, and retry/error ticks when labels are present.  
Then remove duplicate helper calls inside `do_awaiting_answers_to_awaiting_feedback` / `do_awaiting_feedback`.  
Add assertions in both integration and conformance bot-login-failure waiting-state tests that `--add-label ralph:waiting-feedback` is attempted.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs) - run reconciliation before bot-login in waiting-state handlers.
- [`tests/daemon_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs) - add waiting-label assertions in bot-login-failure waiting-state tests.
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs) - add equivalent conformance assertions.
