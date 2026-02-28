---
artifact: completer-verdict
loop: 6
project: issue-93
backend: claude(opus)
role: completer
created_at: 2026-02-28T21:59:15Z
---

# Verdict: COMPLETE

The project satisfies all requirements:

- **Early Prompt Push** (`commit_and_push_initial_prompt` in `src/git/commit.rs:138`): Stages only the 3 prompt-input files (`prompt.md`, `project.toml`, `config.toml`), enforces branch guard via `BranchMismatch` error, returns early on empty diff, commits and pushes on diff. Called from `src/workflow/orchestrator.rs:263` after branch checkout, before implementation loop.

- **Draft PR Watcher** (`draft_pr_watcher` in `src/daemon/runtime.rs:191`): Async task using `tokio::select!` with `CancellationToken` for clean shutdown. Polls `has_commits_ahead_of_base` (via `git rev-list --count`) on 15s interval. Pushes unconditionally before `gh pr create --draft`. Uses `pr_created` flag to prevent duplicate creation. Persists PR URL via `save_task_metadata`. Falls back to `find_existing_pr` on creation failure (concurrent PR detection).

- **GitHub API Extensions** (all in `src/daemon/github.rs`): `has_commits_ahead_of_base` (line 585), `mark_pr_ready` (line 616), `is_pr_draft` (line 639), `close_pr` (line 671). `create_pr` (line 554) accepts `draft: bool` parameter and passes `--draft` to `gh pr create`. All errors propagated through `RalphError::Orchestration`.

- **PR Lifecycle Management** (`handle_pr_flow` in `src/daemon/runtime.rs:2179`): Uses `decide_draft_pr_transition` to determine action — `MarkReady` when PR is draft and has changes on completion; `CloseNoDiff` when no diff and PR is draft (clears persisted PR URL). `complete_task` (line 1581) retries with max 3 attempts (`COMPLETE_TASK_MAX_ATTEMPTS`), 30s delay (`COMPLETE_TASK_RETRY_DELAY_SECS`), and only retries when `err.is_transient()` returns true. `BranchMismatch` is explicitly NOT transient (verified in `src/error.rs:172`).

- **Child Process Plumbing**: `--pr-url` exists in `AutoArgs` (`src/cli/auto.rs:64`) and `RunArgs` (`src/cli/mod.rs`). At dispatch (`runtime.rs:1241-1261`), PR URL resolved first from durable metadata, then by exact head-branch match via `find_existing_pr`. Passed to child via `process::spawn_ralph_auto`/`spawn_ralph_run` (process.rs:136,165). `ChildHandle` (mod.rs:25) includes `draft_pr_cancel: CancellationToken` and `draft_pr_handle: Option<JoinHandle<()>>`. All three exit paths cancel+join both watcher handles: `collect_children` (line 1415-1416), `kill_aborted_children` (line 1475-1476), `drain_all_children` (line 1526-1527).

- **Git Pollution Prevention**: `.gitignore` includes `/SPEC.md` and `.ralph/` patterns. `GENERATED_ARTIFACT_PATHS` constant (`src/git/commit.rs:15`) lists `SPEC.md`. `unstage_non_commit_artifacts` (line 268) runs `git rm --cached -r --ignore-unmatch .ralph` and `git rm --cached --ignore-unmatch -- SPEC.md` — never deletes working-tree files. Called in `commit_feature_loop` (line 125), `commit_and_push_phase_transition` (line 218), and `stage_implementation_changes` (line 264).

- **Conformance Tests**: All 12 required tests exist — 6 in `tests_pr_lifecycle.rs` (early prompt staging, branch mismatch, draft-ready transition, no-diff close, complete_task retry, complete_task terminal error) and 6 in `tests_pr_runtime.rs` (draft watcher creation, push-before-create, cancellation, create_pr draft, --pr-url plumbing, e2e binary test). Both registered in `validate/mod.rs`.

- **Acceptance Criteria**: `nix develop -c cargo check` passes. `nix develop -c cargo test` passes (all tests ok, 0 failures).

---
