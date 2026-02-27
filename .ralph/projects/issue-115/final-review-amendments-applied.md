# Final Review Amendments Applied

## Round 1

### Amendment: AM-PRD-LOG-001

### Problem
Reviewer failure attempts can lose raw backend output. In [`quick.rs:287`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/prd/quick.rs:287), `run_review_with_retry()` uses `backend.execute(...)`, and raw output is only written on success at [`quick.rs:288`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/prd/quick.rs:288)-[`quick.rs:291`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/prd/quick.rs:291). On error/timeout, it returns after markers at [`quick.rs:295`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/prd/quick.rs:295)-[`quick.rs:304`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/prd/quick.rs:304).  
This bypasses `execute_with_log` streaming capture (stdout/stderr/partial output) available in [`mod.rs:539`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/backend/mod.rs:539)-[`mod.rs:557`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/backend/mod.rs:557) and [`mod.rs:623`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/backend/mod.rs:623)-[`mod.rs:625`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/backend/mod.rs:625).

### Proposed Change
Use `backend.execute_with_log(&current_prompt, log_writer.as_deref_mut())` in `run_review_with_retry()`, and remove the manual success-path `writer.write_str(&raw)` to avoid duplicate output.

### Affected Files
- [`src/prd/quick.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/prd/quick.rs) - switch reviewer execution to `execute_with_log` and keep markers.

### Reviewer
codex

### Amendment: AM-PRD-LOG-002

### Problem
Conformance coverage misses reviewer execution-failure logging. Existing tests cover reviewer parse-retry ([`tests_interactive_prd.rs:3248`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/validate/tests_interactive_prd.rs:3248)) and question/synthesis timeout/error ([`tests_interactive_prd.rs:3357`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/validate/tests_interactive_prd.rs:3357)), but no reviewer timeout/non-zero-exit case validates raw-output persistence and outcome markers in `issue-<n>-reviewer.log`.

### Proposed Change
Add reviewer failure conformance tests for:
1. Timeout after partial output.
2. Non-zero exit after emitting output.  
Assert reviewer log contains attempt separator, `backend_spec=...`, prompt summary, raw output snippet (when emitted), and execution/validation/parse markers.

### Affected Files
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-115/src/validate/tests_interactive_prd.rs) - add reviewer failure-path logging assertions.

---

## Context Provided
- Reviewed `git diff origin/master...HEAD -- . ':(exclude).ralph'`.
- Audited changed files in `interactive_prd`, `quick`, and `validate/tests_interactive_prd`.
- Ran:
  - `cargo check`
  - `cargo test --quiet test_review_parse_retry_success`
  - `cargo test --quiet test_review_parse_retry_exhaustion`
- Verified no tracked-file stray changes outside `.ralph` with `git status --short -- . ':(exclude).ralph'`.

## Master Prompt
The implementation is close, but reviewer execution-failure logging is still incomplete for diagnosability because raw output is not reliably preserved on failed reviewer attempts, and current conformance tests do not cover that gap.

### Reviewer
codex

