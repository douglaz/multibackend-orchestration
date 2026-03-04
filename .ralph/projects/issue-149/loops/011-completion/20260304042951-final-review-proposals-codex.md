---
artifact: final-review-proposals
loop: 11
project: issue-149
backend: codex
role: final_reviewer
created_at: 2026-03-04T04:29:51Z
---

Reviewing the implemented daemon resilience changes now. I’ll first identify the base branch and inspect `git diff <base>...HEAD -- . ':(exclude).ralph'`, then trace the changed code paths and tests for correctness/safety issues.
I identified `origin/master` as the base. Next I’m running the required three-dot diff excluding `.ralph`, then I’ll inspect each changed source/test file in detail.
I have the complete diff and will now inspect the surrounding code in the three edited files to verify behavior on error paths, retry logic, and watcher teardown under failure conditions.
I’m moving to verification now: compiling and running tests in the Nix dev environment to confirm these changes are actually passing and to catch any hidden regressions beyond static inspection.
`cargo test` is still running and producing large output from integration/conformance-style tests. I’m polling to completion and then I’ll finish the code-level audit with any concrete amendments needed.
Test execution is still in progress across the larger daemon/integration suites; I’m continuing to poll until the final result and then I’ll consolidate findings with exact file/line references.
`cargo check` and `cargo test` both passed. I’m now validating subtle edge cases in the new retry/error-classification paths and watcher timeout helper to ensure no latent correctness or safety regressions remain.
I found one potentially real edge-case defect in push-retry classification and I’m confirming it against the exact logic paths before finalizing amendments. I’m also checking watcher behavior for any remaining unbounded or leaked work after timeout-abort.
# Final Review: AMENDMENTS

## Amendment: DAEMON-PUSH-RETRY-CLASSIFIER-001

### Problem
`is_retryable_push_stderr` does raw substring matching for HTTP codes across the full stderr text (for example `"403"` and `"500"`), then decides retryability from that result ([src/daemon/github.rs:916](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs:916), [src/daemon/github.rs:935](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-149/src/daemon/github.rs:935)).

This is brittle and can misclassify:
- Transient DNS/network errors can be marked non-retryable if the repo URL/path contains `403`.
- Unknown/permanent errors can be marked retryable if unrelated text contains `500/503`.
- `unable to access` is treated as retryable, but HTTP 401-style auth failures may not be excluded.

This can violate the intended behavior for transient vs permanent push handling.

### Proposed Change
Make status-code matching context-aware instead of raw numeric substring checks:
1. Match HTTP codes only in explicit HTTP/error phrases (`http 503`, `returned error: 503`, etc.), not anywhere in stderr.
2. Explicitly classify `401`/`403` auth/permission patterns as non-retryable.
3. Keep existing permanent keyword checks (`permission denied`, `non-fast-forward`, `protected branch`, `GH013`, etc.).
4. Add regression tests for:
   - transient DNS error where URL contains `403` (must still retry),
   - permanent HTTP 401 auth failure (must not retry),
   - explicit HTTP 503 transient failure (must retry),
   - explicit HTTP 403 permanent failure (must not retry).

### Affected Files
- `src/daemon/github.rs` - harden classifier logic and add regression tests for code-collision/auth cases.

---
