---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T13:31:12Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] PR-Review Task Discovery Silently Drops Corrupt Metadata

### Problem
`discover_tasks_with_prs` silently defaults corrupt task metadata to empty (`unwrap_or_default`) and then skips the task when `pr_url` is missing, which can orphan staged amendments/markers with no operator signal ([src/daemon/pr_review.rs#L499](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L499)).  
This is especially risky because metadata persistence is non-atomic (`std::fs::write` directly), so crash-interrupted writes can create malformed JSON ([src/daemon/runtime.rs#L729](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs#L729)).

### Proposed Change
Make `save_task_metadata` atomic (tmp file + rename), and in `discover_tasks_with_prs` replace `unwrap_or_default` with explicit parse error handling that logs a warning (and ideally quarantines bad metadata) so tasks are not silently lost.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs` - atomic metadata persistence.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs` - strict metadata parse path with surfaced errors.

## Amendment: [P2] `stage_amendment` Idempotency Check Accepts Invalid Payloads

### Problem
Existing staged-file validation treats any syntactically valid JSON as idempotent success ([src/daemon/pr_review.rs#L138](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L138)).  
If the file is valid JSON but not a valid `AmendmentRequest`, polling still marks the comment as processed ([src/daemon/pr_review.rs#L682](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs#L682)), which can permanently drop that amendment.

### Proposed Change
Validate existing staged files as `AmendmentRequest` (and verify expected `id`/`source`) before treating as idempotent success; otherwise rewrite atomically. Add a unit test for the valid-but-invalid JSON case (e.g., `{}`).

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs` - tighten idempotency validation and add regression test.

## Amendment: [P3] Whitelist Conformance Test Can Pass for Wrong Reason

### Problem
`whitelist_filters_comments` only asserts counts (`3` staged, `3` dedup keys) ([src/validate/tests_pr_review.rs#L149](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs#L149)).  
It does not assert exact included/excluded keys, so a wrong inclusion/exclusion mix with the same count can still pass.

### Proposed Change
Assert exact key set membership (`pull_comment:1`, `issue_comment:10`, `review:20`) and explicit absence of non-whitelisted/self keys (`pull_comment:2`, `issue_comment:11`).

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs` - strengthen assertions to match test intent.

---

## Context Provided
- Reviewed diff via `git diff origin/master...HEAD -- . ':(exclude).ralph'`.
- Audited all changed production files, especially:
  - `src/daemon/pr_review.rs`
  - `src/daemon/runtime.rs`
  - `src/daemon/github.rs`
  - config wiring in `src/config/*` and `src/cli/daemon.rs`
- Ran focused checks:
  - `nix develop -c cargo test pr_review -- --nocapture`
  - `nix develop -c cargo run -- validate --bin target/debug/ralph --filter pr_review` (12/12 passing)
