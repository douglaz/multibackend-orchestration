---
artifact: completer-verdict
loop: 20
project: issue-93
backend: claude(opus)
role: completer
created_at: 2026-03-01T16:43:41Z
---

Now let me compile the final verdict. Let me cross-reference all 12 required conformance tests:

**Required conformance tests vs. what's implemented:**

From `tests_pr_lifecycle.rs` (7 tests):
1. `early_prompt_push_stages_only_prompt_files` - **Requirement 1**
2. `early_prompt_push_fails_on_branch_mismatch` - **Requirement 2**
3. `draft_pr_marked_ready_transition` - **Requirement 7**
4. `no_diff_draft_pr_closed_transition` - **Requirement 8**
5. `complete_task_retries_transient_up_to_three` - **Requirement 9**
6. `complete_task_no_retry_terminal` - **Requirement 10**
7. `phase_transition_preserves_tracked_ralph_prompt_files` - **Extra test for git pollution prevention**

From `tests_pr_runtime.rs` (7 tests):
1. `draft_watcher_creates_draft_when_branch_ahead` - **Requirement 3**
2. `draft_watcher_pushes_before_create` - **Requirement 4**
3. `draft_watcher_exits_cleanly_on_cancellation` - **Requirement 5**
4. `pr_url_plumbed_through_child_args` - **Requirement 11**
5. `e2e_draft_create_via_binary` - **Requirement 12**
6. `create_pr_honors_draft_true` - **Requirement 6**
7. `draft_watcher_fallback_base_when_configured_missing` - **Extra resilience test**

All 12 required conformance tests are covered, plus 2 extra tests.

# Verdict: COMPLETE

The project satisfies all requirements:

- **Early Prompt Push** (Req 1): `commit_and_push_initial_prompt()` in `src/git/commit.rs:138` stages only the three prompt-input files (`prompt.md`, `project.toml`, `config.toml`), enforces a hard branch guard via `BranchMismatch` typed error, returns early on empty staged diff, and commits+pushes on the expected branch. Called from `src/workflow/orchestrator.rs:263` after branch checkout.

- **Draft PR Watcher** (Req 2): `draft_pr_watcher()` in `src/daemon/runtime.rs:206` uses `tokio::select!` with `CancellationToken` for clean shutdown. Polls `has_commits_ahead_of_base()` on a configurable interval (`RALPH_DRAFT_PR_WATCH_POLL_SECS`). Performs unconditional `push_branch()` before `create_pr(..., true)`. Single-flight guard via `pr_created` flag. Persists PR URL via `save_task_metadata()`.

- **GitHub API Extensions** (Req 3): `src/daemon/github.rs` provides `has_commits_ahead_of_base()` (line 590) with robust base resolution via `resolve_ahead_base()` + `detect_base_branch()` fallback chain, `mark_pr_ready()` (line 657), `is_pr_draft()` (line 680), `close_pr()` (line 712). `create_pr()` (line 554) and `create_pr_with_body_file()` (line 763) both accept `draft: bool` parameter. All failures propagated through `RalphError`.

- **PR Lifecycle Management** (Req 4): `handle_pr_flow()` in `src/daemon/runtime.rs:2293` checks `has_diff_with_base()`, `is_pr_draft()`, and uses `decide_draft_pr_transition()` (line 1669) to determine `MarkReady` or `CloseNoDiff`. No-diff draft PRs are closed and PR URL metadata is cleared. `complete_task()` (line 1695) retries up to `COMPLETE_TASK_MAX_ATTEMPTS=3` with `COMPLETE_TASK_RETRY_DELAY_SECS=30`, only retrying transient errors via `err.is_transient()`. `BranchMismatch` is classified as terminal in `src/error.rs:172`.

- **Child Process Plumbing** (Req 5): `--pr-url` added to `AutoArgs` (`src/cli/auto.rs:64`) and `RunArgs` (`src/cli/run.rs:27`). `ChildHandle` in `src/daemon/mod.rs:25` includes `draft_pr_cancel: CancellationToken` and `draft_pr_handle: Option<JoinHandle<()>>`. Watcher handle is joined/cancelled in all exit paths: normal completion (line 1529-1534), abort (line 1589-1594), force-kill/drain (line 1640-1645).

- **Git Pollution Prevention** (Req 6): `.gitignore` includes `SPEC.md` and `.ralph/daemon/`, `.ralph/**/*.log`, etc. `unstage_non_commit_artifacts()` (line 268) uses non-destructive `git reset HEAD -- .ralph` followed by `git rm --cached --ignore-unmatch` only for `GENERATED_ARTIFACT_PATHS` (`SPEC.md`). Called from both `commit_feature_loop()` and `commit_and_push_phase_transition()`.

- **Conformance Tests**: All 12 required tests present and registered in `src/validate/mod.rs:125-126` via `tests_pr_runtime::tests()` and `tests_pr_lifecycle::tests()`, plus 2 bonus tests. Tests are:
  1. `early_prompt_push_stages_only_prompt_files`
  2. `early_prompt_push_fails_on_branch_mismatch`
  3. `draft_watcher_creates_draft_when_branch_ahead`
  4. `draft_watcher_pushes_before_create` (order assertion)
  5. `draft_watcher_exits_cleanly_on_cancellation`
  6. `create_pr_honors_draft_true`
  7. `draft_pr_marked_ready_transition`
  8. `no_diff_draft_pr_closed_transition`
  9. `complete_task_retries_transient_up_to_three`
  10. `complete_task_no_retry_terminal`
  11. `pr_url_plumbed_through_child_args`
  12. `e2e_draft_create_via_binary`

- **Acceptance Criteria**: `cargo check` passes (1 dead_code warning only). `cargo test` passes (all tests green). Validate suite includes new PR lifecycle tests. Watcher handles are properly cancelled/joined in all exit paths. Branch safety guard is enforced.
