---
artifact: completer-verdict
loop: 4
project: issue-93
backend: claude(opus)
role: completer
created_at: 2026-02-28T21:34:33Z
---

# Verdict: COMPLETE

The project satisfies all requirements:

- **Early Prompt Push** (`commit_and_push_initial_prompt` in `src/git/commit.rs:140-193`): Implemented with branch guard (resolves current branch, compares to expected, returns `BranchMismatch` typed error), stages only the three prompt-input files (`prompt.md`, `project.toml`, `config.toml`), returns `Ok(())` on empty staged diff, and commits+pushes on non-empty diff. Called from `orchestrator.rs:263` immediately after branch checkout and before the implementation loop.

- **Draft PR Watcher** (`draft_pr_watcher` async fn in `src/daemon/runtime.rs:184-299`): Uses `tokio::select!` with `CancellationToken` for clean shutdown. Polls `has_commits_ahead_of_base` on a 15-second interval. Creation rule enforced: checks ahead, checks `!pr_created`, pushes unconditionally before `create_pr(..., true)`. Single-flight guard via `pr_created` bool. Persists PR URL via `save_task_metadata`.

- **GitHub API Extensions** (`src/daemon/github.rs`): All four new functions implemented — `has_commits_ahead_of_base` (line 585, uses `git rev-list --count`), `mark_pr_ready` (line 616, uses `gh pr ready`), `is_pr_draft` (line 639, uses `gh pr view --json isDraft`), `close_pr` (line 671, uses `gh pr close`). Both `create_pr` (line 554) and `create_pr_with_body_file` (line 722) accept `draft: bool` parameter and conditionally pass `--draft`. All failures use typed `RalphError` propagation.

- **PR Lifecycle Management**: `handle_pr_flow` in `runtime.rs:2146-2428` implements: (1) draft→ready transition via `should_mark_draft_pr_ready` (line 2378, checks `has_changes && pr_is_draft && terminal_label == "ralph:completed"`), (2) no-diff draft closure via `should_close_no_diff_draft_pr` (line 2203, checks `!has_changes && pr_is_draft`) which calls `close_pr` and clears stored PR URL. `complete_task` (line 1548) implements retry: max 3 attempts (`COMPLETE_TASK_MAX_ATTEMPTS`), 30s delay (`COMPLETE_TASK_RETRY_DELAY_SECS`), retries only transient failures via `should_retry_complete_task` which delegates to `RalphError::is_transient()`. Terminal errors (Validation, BranchMismatch, etc.) are explicitly non-transient in `error.rs:162-196`.

- **Child Process Plumbing**: `--pr-url` optional arg added to `AutoArgs` (`cli/auto.rs:64-65`) and `RunArgs` (`cli/run.rs:27`). Plumbed through `spawn_ralph_auto` and `spawn_ralph_run` in `process.rs` (lines 33, 77, 135-137, 164-166). PR URL resolved by head-branch match via `find_existing_pr` in dispatch (runtime.rs:1237-1258). `ChildHandle` struct (`daemon/mod.rs:25`) includes `draft_pr_cancel: CancellationToken` (line 32) and `draft_pr_handle: Option<JoinHandle<()>>` (line 34) and `pr_url: Option<String>` (line 42). Watcher handle joined/cancelled in all exits: normal completion (line 1409), abort check (line 1469), and drain_all_children (line 1520).

- **Git Pollution Prevention**: `.gitignore` contains generated-artifact patterns including `.ralph/daemon/`, `.ralph/**/*.log`, `.ralph/quick-prd/`, `.ralph/tmp/`, `.ralph/sessions/`, `.ralph/index.json`, `.ralph/workspace.lock`, `.ralph/repo.lock`, and `/SPEC.md`. Both `commit_and_push_phase_transition` (commit.rs:218-224) and `commit_feature_loop` (commit.rs:122-127) perform `git add -A` followed by `git rm --cached -r --ignore-unmatch .ralph` to explicitly unstage tracked generated artifacts. The `--ignore-unmatch` flag safely handles "pathspec not found" and `let _ =` ensures it never deletes working-tree files.

- **12 Conformance Tests**: All 12 tests registered in `validate/mod.rs:125-126` via `tests_pr_runtime::tests()` (6 tests) and `tests_pr_lifecycle::tests()` (6 tests):
  1. `early_prompt_push_stages_only_prompt_files` — validates only prompt files staged
  2. `early_prompt_push_fails_on_branch_mismatch` — validates typed BranchMismatch error
  3. `draft_watcher_creates_draft_when_branch_ahead` — validates `has_commits_ahead_of_base`
  4. `draft_watcher_pushes_before_create` — validates push then create ordering
  5. `draft_watcher_exits_cleanly_on_cancellation` — validates CancellationToken + select!
  6. `draft_pr_marked_ready_transition` — validates `should_mark_draft_pr_ready` logic
  7. `no_diff_draft_pr_closed_transition` — validates `should_close_no_diff_draft_pr` logic
  8. `complete_task_retries_transient_up_to_three` — validates retry cap at 3
  9. `complete_task_no_retry_terminal` — validates terminal errors skip retry
  10. `pr_url_plumbed_through_child_args` — validates CLI parsing for both `run` and `auto`
  11. `e2e_draft_create_via_binary` — validates real binary accepts `--pr-url`
  12. `pr_url_persisted_across_restarts` — validates save/load round-trip for PR URL metadata

- **`cargo check` passes**: Clean build with no errors.
- **`cargo test` passes**: All tests pass (0 failures across all test suites).

---
