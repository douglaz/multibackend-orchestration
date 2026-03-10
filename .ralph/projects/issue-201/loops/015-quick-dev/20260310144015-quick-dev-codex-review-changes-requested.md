---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T14:40:15Z
---

# Review: CHANGES REQUESTED
1. **High: ready issues with staged PR-review amendments can be permanently skipped**
   - In claim flow, ready issues are skipped whenever staged amendments exist ([runtime.rs:1136](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1136)).
   - In PR-review flow, ready issues are only resumed if a marker exists ([runtime.rs:2739](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2739)).
   - Comments are staged without lifecycle filtering ([pr_review.rs:602](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:602), [pr_review.rs:693](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:693)), so `ready + staged + no marker` is reachable and then owned by neither path.
   - **Fix:** In `pr_review_phase`, allow `ralph:ready` resumes when staged amendments exist (not only marker), and set/maintain a resume marker for that path too. Add a validate test for `ready + staged + no marker` to verify dispatch happens.

2. **High: partial label-swap rollback failure can strand tasks forever**
   - On swap failure, marker is intentionally retained when rollback failed ([runtime.rs:2791](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2791)).
   - But resume selection later only accepts `completed` or `ready+marker` ([runtime.rs:2737](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2737)). If the issue is left with **no lifecycle label** (possible when remove succeeded, add failed, rollback failed), it is never recovered.
   - **Fix:** Add explicit recovery for `marker present + no lifecycle label` (e.g., re-add `ralph:ready`, or store original label in marker and restore it), then retry resume. Add a conformance test where forward add and rollback add both fail.

Assumption to confirm: if this feature is intentionally post-completion only, also gate staging to completed issues in `poll_pr_reviews` to avoid creating staged artifacts for non-resumable lifecycle states.
