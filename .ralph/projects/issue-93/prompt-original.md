The engineering specification is complete in `PLAN.md`. Here's a brief summary of what it covers:

**6 major work areas across 12 files:**

1. **Early Prompt Push** — New `commit_and_push_initial_prompt()` in `commit.rs`, called from `orchestrator.rs` after branch checkout, staging only the 3 prompt files with a hard branch guard
2. **Draft PR Watcher** — New `draft_pr_watcher()` async task in `runtime.rs` using `tokio::select!` cancellation pattern, polling via `git rev-list --count`, performing unconditional push before `gh pr create --draft`
3. **GitHub API Extensions** — 4 new functions (`has_commits_ahead_of_base`, `mark_pr_ready`, `is_pr_draft`, `close_pr`) + `draft: bool` parameter on existing `create_pr` functions
4. **PR Lifecycle Management** — `handle_pr_flow` gains draft→ready marking and no-diff draft closure; `complete_task` gains 3-attempt retry loop with 30s delays
5. **Child Process Plumbing** — `--pr-url` arg on `AutoArgs`/`RunArgs`, dispatch-time multi-branch PR lookup, `ChildHandle.draft_pr_handle` joined in all exit paths
6. **Git Pollution Prevention** — `.gitignore` entry + explicit `git rm --cached` unstaging in `commit_and_push_phase_transition`

**12 conformance tests** covering the full lifecycle from draft creation through readiness marking, retry logic, no-diff closure, branch mismatch safety, env var timing, and a real-binary e2e test.