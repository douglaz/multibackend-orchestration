---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 2
project: issue-161
backend: claude
role: implementer
created_at: 2026-03-04T19:12:46Z
---

# Apply-Fixes: Update Final Review Mock Planner in Orchestrator Tests

## Changes Made

Updated `tests/orchestrator.rs` to fix the mock planner behavior after amendment restarts, addressing the test suite regression caused by the new guard in `orchestrator.rs:655`.

### 1. Mock planner now uses a call counter (line 2844)

Instead of always returning `# Project Completion Request`, the planner section of `write_final_review_backend_script` now tracks call count:
- **Call 1 and call >= 3**: Returns `CompletionRequest` (original behavior)
- **Call 2**: Returns `# Feature: Address Amendments` (correct post-amendment behavior)

This matches the pattern already used in `src/validate/tests_final_review.rs`.

### 2. Added implementer and reviewer handlers

Added mock responses for:
- `"You are a software developer implementing a feature specification."` — returns implementation notes (no `git add`, matching existing orchestrator test patterns)
- `"You are a code reviewer ensuring implementations match specifications."` — returns `APPROVED`
- `"You are a QA engineer validating an implementation against its specification."` — returns `PASS`

These are needed because the Feature spec triggers an implementation loop.

### 3. Test assertions unchanged

All existing assertions remain correct because:
- Tests with `max_restarts=1` (`fail-after-proposals-once`, `config-mismatch`, `always-amend`): The force-complete check fires BEFORE final reviewers are called on the second completion attempt, so reviewer counts don't increase
- Tests with `max_restarts=3` (`accepted-restart`, `disputed-restart`): `completion_attempts.len() == 2` still holds (Feature loop doesn't create a completion attempt)

### Verification

- `cargo test --test orchestrator final_review_` — 6/6 pass
- `cargo test --test orchestrator` — 24/24 pass  
- `nix build -L` — 355/355 tests pass, build succeeds
