---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-161
backend: claude
role: implementer
created_at: 2026-03-04T18:58:51Z
---

Build succeeds. Here's a summary of all three changes:

# Apply Reviewer-Requested Fixes

## Changes Made

### 1. Guard logic fix — `orchestrator.rs:656-675`
Added `has_feature_after` check so the guard only fires for the immediate bad transition (restart → completion without intervening feature loop). After a restart→feature→completion flow, the restart artifact still exists on the old completion loop, but a feature loop with a higher `loop_number` now exists, so the guard correctly allows the completion request through.

### 2. Implementation-QA handler — `tests_final_review.rs:189`
Broadened the QA prompt match from the specific `"You are a QA engineer validating overall project acceptance."` to the general `"You are a QA engineer"*` pattern. This handles both acceptance-QA and implementation-QA prompts, preventing the script from falling through to `"unrecognized prompt"` when the orchestrator runs implementation QA after a feature loop.

### 3. Restart artifact count assertion — `tests_final_review.rs:376-393`
Added assertion in `planner_completion_after_amendments_fails` that exactly one `*-final-review-exit-restart.md` artifact exists in the single completion loop's directory, verifying the restart budget was not consumed beyond the single restart that triggered the guard.
