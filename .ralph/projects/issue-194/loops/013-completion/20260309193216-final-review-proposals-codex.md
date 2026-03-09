---
artifact: final-review-proposals
loop: 13
project: issue-194
backend: codex
role: final_reviewer
created_at: 2026-03-09T19:32:16Z
---

Reviewing the implementation directly from source now. I’ll first identify the correct base commit, run the requested diff excluding `.ralph`, then trace the changed code paths and tests for correctness/safety issues.
I confirmed this branch is one commit ahead of `origin/ralph/issue-194`; next I’m computing the merge base with `origin/master` and reviewing the full project diff from that base to `HEAD` (excluding `.ralph`) as requested.
I’ve identified all changed files from the feature branch and will now trace the high-risk paths first: queue drain/enqueue logic, orchestrator integration, and completion guard behavior, then validate the tests’ assertions.
I’ve finished the queue module pass and am moving through orchestrator/final-review code paths to confirm failure semantics and whether drained amendments are safely preserved on all error branches.
I’m now auditing the conformance tests in detail (especially prompt-injection and guard tests) to verify they fail for the right reasons and truly exercise the new code paths.
I’ve completed the static trace and am now running targeted tests for the amended queue/orchestrator paths to confirm observed behavior and uncover any hidden regressions.
Targeted unit and integration tests passed. I’ll run the validate amendment test module directly next, since one earlier pattern did not match any tests.
# Final Review: AMENDMENTS

## Amendment: AMQ-UTF8-READ-001 [P1]

### Problem
Malformed non-UTF8 queue files are treated as fatal I/O errors instead of quarantined malformed input.  
`read_and_parse_inflight` uses `fs::read_to_string` ([`#L492`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L492)); invalid UTF-8 becomes `ReadFailed` and aborts drain via rollback path ([`#L246`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L246)).  
That violates the “malformed files must not fail orchestration; quarantine and continue” requirement.

### Proposed Change
Read bytes (`fs::read`) and parse with `serde_json::from_slice`; classify decode/parse/validation failures as malformed (quarantine), not fatal.  
Keep only true filesystem read failures (permission, device I/O, etc.) as fatal.  
Add a unit test with invalid UTF-8 JSON bytes to verify quarantine + continued drain.

### Affected Files
- [`src/project/amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs) - fix classification path and add regression test.

## Amendment: AMQ-STEM-RACE-002 [P1]

### Problem
Drain currently deletes any `.json` whose stem matches a previously processed `.inflight` ([`#L197`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L197)-[`#L208`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L208), [`#L254`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L254)).  
This is lossy under concurrency: a new enqueue can legally publish `<stem>.json` while `<stem>.inflight` exists, since enqueue only collision-checks destination `.json` ([`#L147`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L147)-[`#L152`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L152)).  
Result: legitimate new amendment can be silently deleted. Existing test bakes this assumption in ([`#L750`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs#L750)).

### Proposed Change
Do not drop `.json` by stem alone. Drop only when proven duplicate (for example same inode/content), otherwise process as distinct.  
Also treat matching `.inflight` as occupied when choosing enqueue filename suffix to reduce collisions.  
Replace the current same-stem test with a regression that uses same stem + different payload and verifies no loss.

### Affected Files
- [`src/project/amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs) - fix drain dedupe and enqueue collision handling; update tests.

## Amendment: AMQ-STRAY-FILE-003 [P3]

### Problem
A root-level implementation artifact was committed: [`20260309T190541Z-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/20260309T190541Z-impl-notes.md).  
This is non-source output and appears out of scope for the feature.

### Proposed Change
Remove the file from the repo history for this change set (and keep runtime artifact cleanup/ignore behavior preventing recurrence).

### Affected Files
- [`20260309T190541Z-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/20260309T190541Z-impl-notes.md) - delete.

---
