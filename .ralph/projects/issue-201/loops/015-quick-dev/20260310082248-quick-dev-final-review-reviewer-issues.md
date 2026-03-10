---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T08:22:48Z
---

# Final Review: AMENDMENTS

## Amendment: PRR-RESUME-GUARD [P1]

### Problem
`pr_review_phase` always dispatches PR-review resumes with a placeholder prompt (`"PR review amendments for issue #..."`) at [`runtime.rs:2671`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2671).  
In `dispatch_task`, dispatch origin is currently ignored (`_origin`) at [`runtime.rs:1379`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1379), and the flow falls back to “fresh start” when `resume_existing_project == false` at [`runtime.rs:1450`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1450) and [`runtime.rs:1492`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1492).  
That can start a brand-new orchestration from the placeholder text if the resumable project is missing (branch/prompt drift), instead of safely failing and rolling back labels.

### Proposed Change
Add a hard precondition for PR-review resumes:
- In `dispatch_task`, when `origin == DispatchOrigin::PrReviewResume`, require `resume_existing_project == true`.
- If not resumable, return an error before spawning a fresh task.
- Keep staged amendments intact and let existing rollback in `pr_review_phase` restore `ralph:completed`/`ralph:ready`.
- Add a conformance test for this failure mode (missing resumable project) asserting: no fresh dispatch, labels rolled back, staged amendments preserved.

### Affected Files
- [`src/daemon/runtime.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs) - enforce PR-review resume precondition using `DispatchOrigin`.
- [`src/validate/tests_pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add regression coverage for non-resumable PR-review resume.

## Amendment: PRR-TEST-STATE-ASSERT [P3]

### Problem
The conformance test `pr_review::completed_project_resumes_with_state_reset` at [`tests_pr_review.rs:150`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs:150) does not actually assert project state reset despite its name. It only checks label/log behavior and staged-file drain, so it can pass even if reset logic regresses.

### Proposed Change
Strengthen this test to assert the resumed project state in the worktree (at minimum `status == "in_progress"` for the resumed project). Keep existing label/drain assertions.

### Affected Files
- [`src/validate/tests_pr_review.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs) - add explicit state-reset assertion in the regular resume test.
