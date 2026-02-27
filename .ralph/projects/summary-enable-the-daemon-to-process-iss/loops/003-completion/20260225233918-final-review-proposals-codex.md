---
artifact: final-review-proposals
loop: 3
project: summary-enable-the-daemon-to-process-iss
backend: codex
role: final_reviewer
created_at: 2026-02-25T23:39:18Z
---

Reviewing the full change set now with a correctness/safety focus. I’ll identify the base branch, run the requested triple-dot diff excluding `.ralph`, then inspect changed code paths and tests for real failure modes.
I’ve identified `master` as the likely base and will now inspect the exact `master...HEAD` diff with `.ralph` excluded, then drill into each changed file and associated tests.
I’ve got the diff and next I’m drilling into exact changed files with line-level context to check for hidden correctness issues, especially around parser behavior, fallback paths, and whether tests verify actual dispatch inputs.
I’m now validating the new conformance tests themselves, since weak assertions can hide regressions. I’ll inspect the helper harness logic and whether tests prove actual `raw_idea` selection vs only log side effects.
I’ve finished static inspection of the new paths and now I’m running targeted tests for the modified modules to validate behavior and check for hidden runtime failures.
I’ve validated the modified code builds and unit tests pass locally. I’m doing one final pass for unintended scope creep and test adequacy, including whether any committed files look accidental.
# Final Review: AMENDMENTS

## Amendment: AMND-PRD-001

### Problem
`clean_draft_body()` does not actually evaluate the “first content line” before heading stripping. It checks the literal first remaining line after marker removal ([`src/daemon/interactive_prd.rs:2171`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs#L2171), [`src/daemon/interactive_prd.rs:2178`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs#L2178)).  
If a draft comment starts with blank lines before the heading, heading cleanup is skipped and polluted text can be dispatched.

### Proposed Change
Strip/skip leading blank lines before heading detection, then apply `DRAFT_HEADING_PREFIX` matching to the first non-empty content line. Add a regression unit test covering `marker + blank line + heading + body + footer`.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs) - fix cleanup logic and add regression coverage.

## Amendment: AMND-PRD-002

### Problem
The new `prd_done_*` conformance tests claim dispatch behavior, but they do not assert the dispatched `--idea` payload.  
Current assertions only check stderr substrings and parser helper output ([`src/validate/tests_interactive_prd.rs:4962`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs#L4962), [`src/validate/tests_interactive_prd.rs:4983`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs#L4983)).  
Also, the daemon mock `ralph` script ignores args and exits immediately ([`src/validate/mock_scripts.rs:967`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/mock_scripts.rs#L967)), so runtime `raw_idea` regressions could pass undetected.

### Proposed Change
Capture the actual `--idea` argument in the daemon mock `ralph` script (write to a temp file), and assert:
1. PRD-done success paths dispatch approved cleaned spec.
2. Fallback paths dispatch exact `compose_raw_idea(title, body)` output.
3. Highest approved revision path dispatches the expected revision body.

### Affected Files
- [`src/validate/mock_scripts.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/mock_scripts.rs) - add argument-capturing daemon mock.
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs) - assert captured dispatched idea content.

## Amendment: AMND-PRD-003

### Problem
A timestamped implementation artifact was committed at repo root ([`1740527543-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md)). This appears unrelated to product/runtime behavior and is likely accidental scope creep.

### Proposed Change
Remove the file from the branch.

### Affected Files
- [`1740527543-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md) - delete.

---
