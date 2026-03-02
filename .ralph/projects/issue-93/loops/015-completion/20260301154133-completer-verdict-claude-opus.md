---
artifact: completer-verdict
loop: 15
project: issue-93
backend: claude(opus)
role: completer
created_at: 2026-03-01T15:41:33Z
---

Now let me compile my verification into a final mapping of the 12 required conformance tests. Let me count what's registered:

**tests_pr_runtime.rs** (7 tests):
1. `draft_watcher_creates_draft_when_branch_ahead` -> Req #3
2. `draft_watcher_pushes_before_create` -> Req #4
3. `draft_watcher_exits_cleanly_on_cancellation` -> Req #5
4. `pr_url_plumbed_through_child_args` -> Req #11
5. `e2e_draft_create_via_binary` -> Req #12
6. `create_pr_honors_draft_true` -> Req #6
7. `draft_watcher_fallback_base_when_configured_missing` -> bonus

**tests_pr_lifecycle.rs** (7 tests):
1. `early_prompt_push_stages_only_prompt_files` -> Req #1
2. `early_prompt_push_fails_on_branch_mismatch` -> Req #2
3. `draft_pr_marked_ready_transition` -> Req #7
4. `no_diff_draft_pr_closed_transition` -> Req #8
5. `complete_task_retries_transient_up_to_three` -> Req #9
6. `complete_task_no_retry_terminal` -> Req #10
7. `phase_transition_preserves_tracked_ralph_prompt_files` -> bonus

All 12 required tests are present plus 2 additional regression tests. Let me now write the final verdict.

# Verdict: COMPLETE

The project satisfies all requirements:

- **Req 1 (Early prompt push stages only prompt files)**: `commit_and_push_initial_prompt()` in `src/git/commit.rs:138` stages exactly `prompt.md`, `project.toml`, `config.toml`. Conformance test: `pr_lifecycle::early_prompt_push_stages_only_prompt_files`.

- **Req 2 (Early prompt push fails on branch mismatch)**: Branch guard via `current_branch()` comparison returning `RalphError::BranchMismatch` at `commit.rs:147-152`. Called from `orchestrator.rs:263` after checkout. Conformance test: `pr_lifecycle::early_prompt_push_fails_on_branch_mismatch`.

- **Req 3 (Draft watcher creates draft PR)**: `draft_pr_watcher()` in `runtime.rs:206-365` with `tokio::select!` cancellation, polls `has_commits_ahead_of_base()`, single-flight `pr_created` guard, consecutive failure threshold. Conformance test: `pr_runtime::draft_watcher_creates_draft_when_branch_ahead`.

- **Req 4 (Draft watcher pushes before create)**: Push at `runtime.rs:282-297` precedes PR creation at `runtime.rs:299-349`. Conformance test: `pr_runtime::draft_watcher_pushes_before_create` with ordering log assertion.

- **Req 5 (Draft watcher exits cleanly on cancellation)**: `tokio::select!` with `cancel.cancelled()` at `runtime.rs:358-363`. Conformance test: `pr_runtime::draft_watcher_exits_cleanly_on_cancellation`.

- **Req 6 (create_pr honors draft: true)**: `create_pr()` in `github.rs:554-582` accepts `draft: bool`, pushes `--draft` flag. Conformance test: `pr_runtime::create_pr_honors_draft_true`.

- **Req 7 (Draft PR marked ready)**: `handle_pr_flow()` in `runtime.rs:2293+` uses `decide_draft_pr_transition()` → `MarkReady` → calls `mark_pr_ready()`. Conformance test: `pr_lifecycle::draft_pr_marked_ready_transition`.

- **Req 8 (No-diff draft PR closed)**: `handle_pr_flow()` checks `has_diff_with_base()`, when false calls `close_pr()` and clears `TaskMetadata.pr_url`. Conformance test: `pr_lifecycle::no_diff_draft_pr_closed_transition`.

- **Req 9 (complete_task retries transient up to 3)**: `complete_task()` at `runtime.rs:1695-1720` loops `1..=COMPLETE_TASK_MAX_ATTEMPTS` (3), `COMPLETE_TASK_RETRY_DELAY_SECS` = 30. `should_retry_complete_task()` checks `err.is_transient() && attempt < 3`. Conformance test: `pr_lifecycle::complete_task_retries_transient_up_to_three`.

- **Req 10 (complete_task does not retry terminal)**: `RalphError::BranchMismatch` and `Validation` return `is_transient() = false`, so retry loop exits immediately. Conformance test: `pr_lifecycle::complete_task_no_retry_terminal`.

- **Req 11 (--pr-url plumbed through child args)**: `--pr-url` in `AutoArgs` (`cli/auto.rs:64-65`) and `RunArgs` (`cli/mod.rs:152-153`), `ChildHandle.pr_url` at `daemon/mod.rs:42`. Conformance test: `pr_runtime::pr_url_plumbed_through_child_args`.

- **Req 12 (Real-binary e2e)**: `pr_runtime::e2e_draft_create_via_binary` creates a daemon harness, runs `daemon start --single-iteration`, verifies both `create` and `ready` transitions in order.

- **GitHub API extensions**: `has_commits_ahead_of_base` (github.rs:590), `mark_pr_ready` (github.rs:657), `is_pr_draft` (github.rs:680), `close_pr` (github.rs:712) all present with typed error propagation.

- **Git pollution prevention**: `.gitignore` contains patterns for `.ralph/daemon/`, `.ralph/**/*.log`, `.ralph/quick-prd/`, `.ralph/tmp/`, `.ralph/sessions/`, etc. `unstage_non_commit_artifacts()` at `commit.rs:268-289` uses `git reset HEAD -- .ralph` (non-destructive) + `git rm --cached --ignore-unmatch` for `GENERATED_ARTIFACT_PATHS` (`SPEC.md`).

- **Watcher handle joined in all exit paths**: `draft_pr_cancel.cancel()` + `draft_pr_handle.take().await` appears in 3 locations in `runtime.rs` (normal completion at ~1529, error return at ~1589, cancellation at ~1640).

- **`nix develop -c cargo check` passes**: Only warning is unused `complete_task_retry_limits()` helper (dead code, not a build error).

- **All 12 required conformance tests registered**: `tests_pr_runtime` and `tests_pr_lifecycle` in `src/validate/mod.rs:125-126`, total 14 tests (12 required + 2 bonus regression tests).
