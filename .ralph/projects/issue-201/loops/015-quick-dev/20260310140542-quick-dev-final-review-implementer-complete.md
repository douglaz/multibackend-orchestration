---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T14:05:42Z
---

The numstat output seems garbled. Let me get the diff properly.
The diff output seems to be showing the full patch, not just names. Let me try differently.
Now let me read the full pr_review.rs and the config changes.
Let me read the full pr_review.rs since it's the main new file.
Good, I've read the full pr_review.rs. Now let me look at the test files and the `extract_pr_number` function.
Now let me check for some specific concerns. Let me look at some key areas more closely.
Now let me look at some specific areas of concern. Let me check the `swap_lifecycle_label` change and all callers to make sure the return type change is properly handled.
Now let me check a critical concern - the callers of `swap_lifecycle_label` that use `?` need to handle the `SwapLabelError` return type since it changed from `Result<()>` to `std::result::Result<(), SwapLabelError>`.
Good. The `SwapLabelError` implements `From<SwapLabelError> for RalphError`, so `?` on callers returning `Result<()>` (where `Result` is `crate::Result` = `Result<_, RalphError>`) will auto-convert. Let me verify the `retrigger_failed_issue` function's return type.
Good. `retrigger_failed_task` returns `Result<()>` which is `crate::Result<()>` = `Result<(), RalphError>`, and `SwapLabelError` implements `From<SwapLabelError> for RalphError`, so the `?` works. Let me now check the complete_task_attempt for the marker clearing call, and the `abort_task_by_labels` in mod.rs.
Good - this correctly uses `.map_err()` to convert `SwapLabelError` to `RalphError`.

Now let me check for more nuanced issues. Let me look at how `pr_review_phase` interacts with the dispatch rollback when `from_label` is `"ralph:ready"`.
Now let me check if the stray deleted md files were supposed to be removed:
These are leftover implementation notes from previous iterations being cleaned up. That's fine.

Now let me verify the code compiles and tests pass:
Good, it compiles. Let me now run the unit tests for the affected modules:
The `push_branch_with_retry_impl_retries_transient_then_succeeds` failure appears to be a pre-existing flaky test (OS "Text file busy" error, not related to our changes). Let me verify:
Good, it's a flaky test. Let me now run the quick_dev_orchestrator tests (since they had field additions):
Now let me do a deeper audit of the specific critical areas. Let me check the `AmendmentSource::PrReview` serialization is correct (serde rename_all):
Good. `AmendmentSource` uses `#[serde(rename_all = "kebab-case")]`, so `PrReview` serializes to `"pr-review"` which matches the `as_str()` return value. The serialization roundtrip test in pr_review.rs confirms this.

Now let me check a few more potential issues:

1. Does the `purge_staged_amendments` clean up `.tmp` files too?
2. Are there race conditions in the pr_review_phase with the pr_open_cache?
Good - `purge_staged_amendments` removes ALL files in the directory (not just `.json`), so `.tmp` files would also be cleaned up.

Now let me check one important area - in `dispatch_task`, the drain is only done for `PrReviewResume` origin. Let me verify that the `Claim` origin does NOT accidentally drain.
Good. The drain/purge is properly gated behind `origin == DispatchOrigin::PrReviewResume`.

Now let me check for one more critical issue: when the drain happens inside `spawn_blocking_op` but the `project_dir` check is done after worktree creation. Is the project_dir inside the worktree, and does it exist at that point?
The flow is correct: worktree is created → branch synced → resume check → fail fast if PrReviewResume and no existing project → drain amendments into worktree project dir.

Now let me look at the `complete_task_attempt` to verify the marker is correctly cleared:
Good, the marker is cleared at terminal completion.

Let me now check the `fetch_issue_labels_with_gh_bin` function used in `pr_review_phase`:
Good. Let me now look at one more potential issue: the `sanitize_id` function to make sure it doesn't collide across different amendment IDs:
Good. The amendment IDs like `PR-42-pull_comment-100` are already safe for filesystem use, and the `sanitize_id` function handles edge cases.

Let me check one more area - the `write_mock_script` / `WriteMockScript` usage to ensure proper test setup for the validation tests. Let me also look at whether the `quick_dev_orchestrator.rs` change is just the field addition:
Good - just a field addition to keep the struct literal complete. Now let me run the full test suite to make sure nothing is broken:
All 1085 tests pass. Let me now complete my analysis of a few remaining potential issues.
Now let me check the `write_task_metadata` / `save_task_metadata` function to ensure pr_url is being correctly stored:
Good. The `save_task_metadata` is used correctly and now uses atomic writes.

Let me check one more area - can `discover_tasks_with_prs` have an issue with the task_id format when owner or repo name contains a hyphen?
The `format_task_id` creates `"{owner}-{repo}-{issue_number}"` and `extract_issue_number_from_task_id` uses `strip_prefix("{owner}-{repo}-")`. This is consistent. If the owner/repo contain hyphens (e.g., `my-org`, `my-repo`), the task_id would be `my-org-my-repo-42` and the prefix would be `my-org-my-repo-`, which strips correctly leaving `42`. This is fine because the prefix is reconstructed from the known owner and repo values.

Let me also verify the `cfg_parse_string_list` function is correct:
Good, `cfg_parse_string_list` supports both JSON array and comma-separated formats.

Now let me do a final check for any potential issues I haven't explored yet - specifically the `Write task metadata` function's atomic write:
Good. The atomic write pattern (write-to-tmp + rename) is correctly applied. Note: if the rename fails, the tmp file is left behind, but this is best-effort logging and the temp file has a `.json.tmp` extension so it won't be confused with real data.

Let me finalize by checking whether there are any unused imports or dead code warnings:
No warnings at all (just the git dirty tree notice from nix). Let me also quickly check the `tracing::info` import in pr_review.rs is used:
Good, `info!` is used at line 734 inside `poll_pr_reviews`. The `tracing::info` import is used.

Let me now also verify the `chrono::Utc` import is used:
All imports are used. Now let me write the final review.
# Final Review: NO AMENDMENTS

## Summary

The PR review polling feature is correctly and safely implemented across 11 changed source files (plus 5 deleted stale implementation notes). All 1085 library tests pass, including 30 new PR-review-specific unit tests and 12 conformance/validation tests. No compiler warnings.

**Key areas verified:**

- **`src/daemon/pr_review.rs` (new, 1519 lines)**: Dedup state uses atomic temp-file + rename writes (`PrReviewState::save`, `stage_amendment`, `reset_project_state_for_resume`). Crash-safety is thoroughly handled: dedup state is persisted incrementally after each staged amendment (line 707), and on save failure the staged file is rolled back (lines 717-731) to prevent dedup-state/staging-dir divergence. `drain_staged_amendments` copies (not moves) files, and `purge_staged_amendments` only runs after successful spawn (line 1792 in runtime.rs). The resume-pending marker system correctly bridges the restart-drift gap.

- **`src/daemon/runtime.rs`**: `dispatch_task` gains a `DispatchOrigin` enum to gate PR-review-specific logic. The fail-fast check at line 1470 ensures `PrReviewResume` dispatches abort before draining if the project state is missing, preserving staged amendments. The drain/purge sequence (lines 1482-1513, 1792-1796) is correctly ordered: drain inside `spawn_blocking_op`, purge only after successful spawn. The `complete_task_attempt` clears the resume-pending marker at line 2427. The `pr_review_phase` function (lines 2521-2807) has correct label-swap rollback with proper marker lifecycle management.

- **`src/daemon/github.rs`**: `swap_lifecycle_label` now returns `SwapLabelError` with `from_label_restored` context for callers needing rollback intelligence. The `From<SwapLabelError> for RalphError` impl ensures all existing callers using `?` continue to compile. All 12 call sites were verified for correct error handling. `fetch_endpoint_json` uses `--paginate` with `merge_paginated_json_arrays` that correctly handles bracket-containing JSON bodies via `serde_json::Deserializer` streaming. Comment parsers (`parse_pull_comments`, `parse_issue_comments`, `parse_review_summaries`) gracefully handle null/missing users and malformed JSON.

- **`src/config/global.rs`, `src/config/mod.rs`, `src/cli/daemon.rs`**: Config plumbing for `daemon_pr_review_whitelist` is correctly threaded from `WorkspaceConfig` → `EffectiveDaemonConfig` → `DaemonRuntimeConfig`. Set/get via `cfg_parse_string_list` supports both JSON array and comma-separated formats.

- **`src/project/amendments.rs`**: `AmendmentSource::PrReview` added with `#[serde(rename_all = "kebab-case")]` → serializes as `"pr-review"`, verified by roundtrip test.

- **Concurrency safety**: `pr_review_phase` runs sequentially within the daemon's main loop (no parallel mutations to `children` map). The `pr_open_cache` is scoped per-cycle, avoiding stale state. Worktree creation holds `repo_root_lock`. All filesystem writes use atomic patterns.

- **No stray files or dead code**: Only the 5 deleted implementation-notes markdown files are outside the `src/` scope, and those were intentional cleanup.

---
