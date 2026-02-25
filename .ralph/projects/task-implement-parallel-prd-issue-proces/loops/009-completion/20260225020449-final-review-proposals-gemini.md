---
artifact: final-review-proposals
loop: 9
project: task-implement-parallel-prd-issue-proces
backend: gemini
role: final_reviewer
created_at: 2026-02-25T02:04:49Z
---

# Final Review: AMENDMENTS

## Amendment: PRD_CONCURRENCY_TEST_LOG_INCONSISTENCY

### Problem
The test `tests::daemon_interactive_prd::handles_slow_and_fast_issue_concurrently` in `tests/daemon_interactive_prd.rs` verifies that a fast-moving PRD issue can complete while a slow one is still processing. It does this by checking the daemon's log output.

However, the log message assertions are incorrect. The test checks for `Finished processing PRD-1`, but the actual log output is `Finished processing PRD issue 1`. This mismatch will cause the test to fail.

The same issue exists for `PRD-2`.

### Proposed Change
Update the log messages being searched for in the test to match the actual format produced by the daemon.

Specifically, change:
- `"Finished processing PRD-1"` to `"Finished processing PRD issue 1"`
- `"Finished processing PRD-2"` to `"Finished processing PRD issue 2"`

### Affected Files
- `tests/daemon_interactive_prd.rs` - Update string literals in log assertions.

## Amendment: PRD_CWD_HANDLING_IN_TESTS

### Problem
The `CliBackend` was refactored to accept an optional `cwd` to ensure concurrent PRD operations run in the correct directory (the cloned repo). This was a key requirement.

However, several test suites that construct a `CliBackend` for PRD-related testing were not updated. They are still creating the backend without providing the necessary `cwd`.

The affected tests are:
1.  `tests::daemon_interactive_prd::handles_prd_lifecycle_happy_path`
2.  `tests::daemon_interactive_prd::handles_prd_lifecycle_with_feedback`
3.  `tests::daemon_interactive_prd::handles_panics_during_issue_processing`

These tests now construct `CliBackend` like this: `Arc::new(CliBackend::new(model_spec.clone()))`. This should be `Arc::new(CliBackend::new(model_spec.clone()).with_cwd(repo_path.clone()))` to match the correct, safe implementation used in the daemon runtime. Without this, the backend processes will execute in the wrong directory, leading to incorrect behavior or test failures.

### Proposed Change
In the three affected tests within `tests/daemon_interactive_prd.rs`, update the instantiation of `CliBackend` to correctly set the working directory using `.with_cwd(repo_path.clone())`.

### Affected Files
- `tests/daemon_interactive_prd.rs` - Correct the `CliBackend` instantiation in three test functions.
