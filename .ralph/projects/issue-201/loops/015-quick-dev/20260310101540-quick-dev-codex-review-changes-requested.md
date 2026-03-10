---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T10:15:40Z
---

# Review: CHANGES REQUESTED
1. **[High] `exactly once` dedup can be violated on state persistence failure**  
In [pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:644), a comment is treated as processed (`processed_keys.insert`, `new_count += 1`) before confirming `state.save()` succeeded ([same block](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:649)).  
If save fails, the code logs and continues. After a later successful dispatch purges staged files, that same comment can be re-enqueued on the next cycle because durable dedup state never recorded it. That breaks the acceptance criterion for persisted exactly-once behavior.

   **Fix:** make stage+dedup persistence transactional for each comment.  
   - Only increment `new_count` and keep the staged file when `state.save()` succeeds.  
   - On save failure, remove the just-staged file (deterministic filename) and revert in-memory key insertion, so the comment retries cleanly next poll without duplicate enqueue risk.

2. **[Medium] New validate tests currently fail due incorrect label-log assumptions**  
`swap_lifecycle_label()` performs **two separate gh edits** (remove then add) in [github.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:1441).  
But assertions in [tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs:698) and [tests_pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs:1179) assume fixed `lines[0]/lines[1]` entries containing `--add-label ...`, which is not how logs are emitted.

   **Fix:** update those tests to assert the expected transition sequence by searching for operations in order (remove completed → add in-progress → remove in-progress → add completed), not fixed line indices for add-only commands.

**Open question / assumption**
- The spec text says quick-dev resume should set `quick_dev_phase=ReviewAndIterate`, while implementation sets `plan_and_implement` in [pr_review.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:275). I treated this as an intentional adaptation to current enums, but it should be explicitly confirmed/updated in spec text.

**Verification note**
- Running `validate --filter pr_review::` currently gives **9 passed / 2 failed** (the two failing tests above).
