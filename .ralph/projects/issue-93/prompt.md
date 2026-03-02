Implement draft-PR lifecycle support and early prompt synchronization in `ralph` with explicit, testable behavior.

### Objective
Add end-to-end support for:
1. Committing/pushing the initial prompt inputs immediately after branch checkout.
2. Creating a draft PR automatically when work begins.
3. Promoting draft PRs to ready when appropriate.
4. Closing no-diff draft PRs.
5. Passing PR URL context through child process execution paths safely.
6. Preventing generated artifacts from polluting git history.

### Required Changes

#### 1) Early Prompt Push
- Add `commit_and_push_initial_prompt()` in `commit.rs`.
- Call it from `orchestrator.rs` immediately after successful branch checkout and before implementation loop execution.
- Stage only the same three prompt-input files used by project setup (no state files, no loop artifacts).
- Enforce a hard branch guard:
1. Resolve current HEAD branch.
2. Compare to expected project branch.
3. If mismatch, return a typed error and do not commit/push.
- If staged diff is empty, return success without commit.
- If staged diff exists, commit and push to the current project branch.

#### 2) Draft PR Watcher
- Add `draft_pr_watcher()` async task in `runtime.rs`.
- Use `tokio::select!` with cancellation so shutdown is immediate and clean.
- Poll branch divergence using `git rev-list --count` (or equivalent helper) on a fixed interval.
- Creation rule:
1. If branch has commits ahead of base.
2. No PR URL is currently recorded.
3. Then do `git push` unconditionally, followed by `gh pr create --draft`.
- Ensure only one draft creation attempt is active at a time.
- Persist/emit the created PR URL for downstream flow.

#### 3) GitHub API Extensions
- Extend GitHub integration with:
1. `has_commits_ahead_of_base`
2. `mark_pr_ready`
3. `is_pr_draft`
4. `close_pr`
- Add `draft: bool` parameter to existing `create_pr` API paths and wire through all implementations/callers.
- Keep failures typed and propagated through existing `RalphError` conventions.

#### 4) PR Lifecycle Management
- Update `handle_pr_flow`:
1. If PR is draft and completion conditions are met, mark it ready.
2. If PR has no diff against base and remains draft, close it and clear stored PR URL/state.
- Update `complete_task` with retry behavior:
1. Max 3 attempts total.
2. 30s delay between attempts.
3. Retry only transient failures (network/transport/rate limit/subprocess transient).
4. Do not retry validation/branch/config logic errors.

#### 5) Child Process Plumbing
- Add optional `--pr-url` to `AutoArgs` and `RunArgs`.
- At dispatch, resolve PR URL by exact head-branch match when multiple PRs exist.
- Add `ChildHandle.draft_pr_handle`.
- Ensure watcher handle is joined/cancelled in all exits:
1. Normal completion
2. Error return
3. Cancellation/interrupt paths

#### 6) Git Pollution Prevention
- Add generated-artifact patterns to `.gitignore`.
- In `commit_and_push_phase_transition`, explicitly unstage tracked generated artifacts via `git rm --cached` before commit.
- Ignore “pathspec not found” safely and never delete working-tree files.

### Conformance Tests (Required)
Add/extend validate conformance tests in `src/validate/` and register in `src/validate/mod.rs`. Provide 12 explicit tests:

1. Early prompt push stages only prompt files.
2. Early prompt push fails safely on branch mismatch.
3. Draft watcher creates draft PR when branch first moves ahead.
4. Draft watcher pushes before draft PR creation (order assertion).
5. Draft watcher exits cleanly on cancellation.
6. `create_pr` honors `draft: true`.
7. Draft PR is marked ready in ready transition path.
8. No-diff draft PR is closed.
9. `complete_task` retries transient failures exactly up to 3 attempts.
10. `complete_task` does not retry terminal errors.
11. `--pr-url` is plumbed through child args and timing is correct.
12. Real-binary e2e covering draft create → ready mark (or close on no diff).

### Acceptance Criteria
- `nix develop -c cargo check` passes.
- `nix develop -c cargo test` passes.
- Validate suite includes the new PR lifecycle tests and they pass against built binary.
- No orphan watcher tasks, no unintended staged/generated files, and branch safety guard is enforced.