---
artifact: final-review-proposals
loop: 4
project: summary-enable-the-daemon-to-process-iss
backend: codex
role: final_reviewer
created_at: 2026-02-26T00:05:32Z
---

Reviewing the implemented changes directly from git history and source now. I’ll first identify the correct base branch, inspect the diff excluding `.ralph`, then read the touched runtime/interactive PRD/validate files and test coverage for correctness and safety.
I confirmed the branch is ahead of `origin/master`, so I’ll use `origin/master...HEAD` as the review base and inspect all changed files excluding `.ralph`, then drill into touched code and tests line-by-line.
I’ve captured the overall patch and now I’m inspecting each modified file in full (especially `runtime.rs`) because the aggregated diff truncated part of that hunk. Then I’ll validate behavior with targeted test runs.
I’m now auditing test quality and whether assertions truly verify dispatch behavior, plus checking for orphaned files and registration consistency in validate modules.
I’ve mapped all changed code paths and found one obvious stray root file plus potential test-strength gaps. Next I’m executing focused tests and tracing daemon dispatch call flow to confirm whether the new conformance tests actually prove `raw_idea` selection, not just log output.

Plan:
1. Run targeted `cargo test` for new parser/label tests and a couple of new validate cases.
2. Trace daemon mock scripts and `RALPH_DAEMON_BIN` interactions to see whether tests verify dispatched payload content.
3. Produce amendment list with exact file/line references, including any orphaned file cleanup.
The first test invocation failed due `cargo test` accepting only one test filter. I’m rerunning with broad filters (`parse_approved_spec_`, `clean_draft_body_`, `has_in_progress_prd_label_`, `prd_done_`) to execute the new cases.
I confirmed the new unit tests pass; next I’m running the actual validate conformance cases with a filter to inspect whether they can pass without proving dispatch payload correctness.
I’ve identified a likely high-signal test gap: the new conformance tests assert log substrings but don’t verify the actual `--idea` payload passed to the dispatched `ralph auto` process. I’m confirming existing daemon test patterns for capturing child args so the amendment can be concrete.
# Final Review: AMENDMENTS

## Amendment: FR-PRD-001

### Problem
Approved-draft selection is too permissive. In [`src/daemon/interactive_prd.rs:2157`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs:2157), the parser uses `c.body.contains(&draft_marker)`.  
That can match non-draft bot comments that merely mention/quote the marker, which can cause wrong spec extraction and wrong `raw_idea` dispatch content.

### Proposed Change
Match draft comments by exact marker line (trimmed line equality), not substring containment. Keep reverse scan to preserve “latest in API order.” Add a unit test where a later bot comment quotes `draft-vN` but is not a draft.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs) - tighten marker matching and add regression test.

## Amendment: FR-PRD-002

### Problem
Heading cleanup does not implement “first content line” semantics robustly. In [`src/daemon/interactive_prd.rs:2176`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs:2176), heading stripping checks `lines.first()` directly.  
If leading blank lines exist after marker removal, the draft heading is not stripped and leaks into extracted spec.

### Proposed Change
Skip leading empty lines before heading detection, then apply `DRAFT_HEADING_PREFIX` check to the first non-empty line. Add a unit test covering marker + blank line(s) + heading.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs) - adjust cleanup logic and add test.

## Amendment: FR-PRD-003

### Problem
New validate tests don’t prove the dispatched idea payload.  
`run_prd_done_daemon` always injects a mock ralph that ignores args (see [`src/validate/tests_interactive_prd.rs:4916`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs:4916) and [`src/validate/mock_scripts.rs:967`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/mock_scripts.rs:967)).  
Tests mainly assert stderr substrings (example: [`src/validate/tests_interactive_prd.rs:4962`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs:4962)) plus direct parser calls, so a daemon bug that logs success but dispatches wrong `--idea` could still pass.

### Proposed Change
Capture and assert the actual `ralph auto --idea` argument in these conformance tests. Use a custom mock ralph script that writes `$3` to a file and assert exact expected payload for:
- approved-spec path
- fallback path
- highest-revision selection
- spoof-resistance case

### Affected Files
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs) - strengthen end-to-end assertions on dispatched idea content.

## Amendment: FR-PRD-004

### Problem
A stray implementation-notes artifact was committed at repo root: [`1740527543-impl-notes.md:1`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md:1). This is unintended scope creep.

### Proposed Change
Remove the artifact from the branch.

### Affected Files
- [`1740527543-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md) - delete file.

---
