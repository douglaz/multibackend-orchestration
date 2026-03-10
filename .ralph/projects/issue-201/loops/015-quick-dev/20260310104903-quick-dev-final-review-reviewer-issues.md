---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T10:49:03Z
---

# Final Review: AMENDMENTS

## Amendment: PRR-001 [P1]

### Problem
`stage_amendment` treats an existing staged file as valid without verification, then writes new files non-atomically via `fs::write` ([pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L130), [pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L138)).  
A crash/power-loss during write can leave a partial JSON file. On the next poll, `path.exists()` short-circuits staging, and dedup state can still be persisted, so that comment may never be restaged correctly.

### Proposed Change
Use atomic staged-file writes (temp file + rename in same dir), and validate existing staged JSON before treating it as idempotent success. If existing content is malformed or mismatched, rewrite atomically.

### Affected Files
- [src/daemon/pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs) - make `stage_amendment` crash-safe and self-healing.
- [src/daemon/pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs) - add unit coverage for malformed pre-existing staged files.

## Amendment: PRR-002 [P2]

### Problem
Resume-pending markers are set before `completed -> in-progress` swap ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2686)), but on dispatch failure the rollback path never clears the marker ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2738)).  
Candidate selection accepts `marker OR staged amendments` ([runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L2573)), so stale markers can trigger repeated re-dispatch attempts even when there is no active in-flight resume.

### Proposed Change
Clear the resume-pending marker when a PR-review resume attempt fails *and* rollback succeeds back to `ralph:completed`. Also clear it when label swap fails immediately after marker creation (no in-flight resume actually started). Keep marker only when rollback itself fails (for restart recovery).

### Affected Files
- [src/daemon/runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - clear stale marker in failure/rollback branches.
- [src/validate/tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add regression test for marker cleanup after failed resume rollback.
