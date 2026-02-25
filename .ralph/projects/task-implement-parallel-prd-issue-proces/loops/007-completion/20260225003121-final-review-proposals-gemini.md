---
artifact: final-review-proposals
loop: 7
project: task-implement-parallel-prd-issue-proces
backend: gemini
role: final_reviewer
created_at: 2026-02-25T00:31:21Z
---

# Final Review: AMENDMENTS

## Amendment: INTERACTIVE-PRD-TEST-REFACTOR-INCOMPLETE

### Problem
The refactor of `tests/daemon_interactive_prd.rs` to use `TestHarness::with_config_and_mock_backend` is incomplete. The new tests `advances_concurrently_with_slow_issue`, `concurrency_is_bounded`, and `panic_in_one_issue_is_isolated` correctly use the new setup.

However, the existing tests (`advances_pending_to_awaiting_answers`, `advances_awaiting_answers_to_awaiting_feedback`, `updates_bot_login_cache_from_feedback`, `dedups_issues_across_labels`) were not updated. They still use the old, more verbose setup involving manual configuration and `TempDir`. This makes the test file inconsistent and harder to maintain.

To complete the refactor, all tests should be migrated to use the `TestHarness`.

### Proposed Change
Update the remaining four tests in `tests/daemon_interactive_prd.rs` to use the `TestHarness::with_config_and_mock_backend` helper, removing the manual `TestConfig`, `GithubService`, and `TempDir` setup. This will align them with the newer tests, improving consistency and readability.

### Affected Files
- `tests/daemon_interactive_prd.rs` - Refactor the four older tests to use the new `TestHarness` setup.
