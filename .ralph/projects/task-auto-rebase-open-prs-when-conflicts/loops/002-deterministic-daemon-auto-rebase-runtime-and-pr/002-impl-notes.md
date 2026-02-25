# Implementation Notes

## Decisions Made
- **Auto-rebase phase placement**: Integrated immediately after `collect_children()` in the main daemon loop, before poll/claim, as specified. This ensures rebase runs every cycle without blocking new task adoption.
- **Worktree creation failures count toward cap**: A failed worktree creation (e.g., branch not found on remote) increments `rebase_count` to prevent infinite loops when all tasks have invalid worktrees. This ensures bounded runtime even in failure scenarios.
- **Environment variable-based mock configuration**: The rebase mock `gh` script uses `MOCK_PR_VIEW_JSON`, `MOCK_PR_VIEW_EXIT`, and `MOCK_PR_COMMENT_LOG` environment variables (inherited by the daemon child process) rather than file-based configuration. This avoids PATH shadowing issues with wrapper scripts.
- **Synchronous rebase execution**: The `execute_rebase` function uses `process::run_command_with_timeout` (a new synchronous bounded command utility) for fetch/rebase steps, and `github::push_force_with_lease` for the push. Each step respects the per-attempt timeout.
- **Lease mismatch detection**: Uses string matching on push error stderr for common indicators (`stale info`, `[rejected]`, `failed to push`, `fetch first`). This covers GitHub's actual rejection messages without over-matching.

## Spec Deviations
- None. All acceptance criteria are implemented as specified.

## Testing
- **14 new conformance tests** added to `src/validate/tests_daemon.rs`:
  1. `rebase_disabled_skip` — config disabled causes skip with log message
  2. `rebase_conflict_skip` — CONFLICTING merge status skips
  3. `rebase_closed_merged_skip` — CLOSED PR state skips
  4. `rebase_unknown_mergeability_skip` — UNKNOWN merge status skips
  5. `rebase_branch_switched_task` — switched branch name appears in rebase log
  6. `rebase_base_branch_from_pr` — `origin/<baseRefName>` used as target (not hardcoded)
  7. `rebase_pr_comment_not_issue` — failure comment posted via `gh pr comment`
  8. `rebase_dedup_by_head_sha` — same head SHA prevents duplicate failure comments
  9. `rebase_force_with_lease_rejection` — push failure handled gracefully
  10. `rebase_gh_pr_view_failure_break` — `gh pr view` failure stops cycle processing
  11. `rebase_per_cycle_cap` — `MAX_REBASES_PER_CYCLE` respected
  12. `rebase_interval_skip` — recently-rebased tasks skipped by interval
  13. `rebase_status_last_rebase_column` — RFC3339 timestamp in status output
  14. `rebase_backward_compat_state` — legacy state without rebase fields deserializes

- **Verification commands**:
  - `nix develop -c cargo check` — clean, no warnings
  - `nix develop -c cargo test` — all unit tests pass
  - `nix develop -c cargo run -- validate --bin target/debug/ralph --filter "daemon::"` — all 44 daemon tests pass (30 existing + 14 new)
