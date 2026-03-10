---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 3
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T04:16:13Z
---

# Apply Reviewer Fixes: PR Review Feature

## Fix 1: `daemon_pr_review_whitelist` not configurable via `ralph config set` (Critical)

Added the missing `set_global_config_value` arm in `src/config/global.rs:1419-1421`:
```rust
"workspace.daemon_pr_review_whitelist" => {
    config.workspace.daemon_pr_review_whitelist = cfg_parse_string_list(raw_value)?;
}
```
This follows the exact same pattern as `daemon_labels` on line 1391. Added a unit test `set_global_config_value_daemon_pr_review_whitelist_roundtrip` that verifies set, TOML serialization roundtrip, and clearing to empty list.

## Fix 2: Staged PR-review amendments stranded on dispatch failure (High)

Changed `drain_staged_amendments()` in `src/daemon/pr_review.rs` to **copy-only** (no longer deletes staged files). Added a new `purge_staged_amendments()` function that removes the staged originals. In `dispatch_task()` (`src/daemon/runtime.rs:1757`), `purge_staged_amendments` is called only after the task spawn succeeds. If dispatch fails at any point, staged files remain intact and `has_staged_amendments()` returns true on the next poll cycle.

Updated the existing `staging_and_drain_roundtrip` test to match the new copy-only behavior. Added:
- **Unit test**: `drain_preserves_staged_files_until_purge` — verifies drain copies without deleting, and purge cleans up
- **Validate test**: `dispatch_failure_preserves_staged_amendments` — integration test where mock `gh repo clone` fails, verifying staged amendments survive and label is reverted
