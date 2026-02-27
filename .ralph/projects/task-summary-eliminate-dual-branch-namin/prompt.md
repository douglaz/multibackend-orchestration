Implement daemon dispatch project-ID normalization so daemon-managed issue tasks always use `project_id = issue-{n}` and project branch `ralph/issue-{n}`.

**Goal**
1. Remove slug-based project discovery from daemon dispatch.
2. Ensure fresh dispatch uses `ralph auto --idea ... --project-id issue-{n}`.
3. Ensure resume dispatch uses `ralph run --project issue-{n}` based only on a file existence check.
4. Preserve existing non-daemon/manual `ralph auto --idea` slug behavior.

**Required behavior**
1. For each dispatch, derive `project_id` exactly as `format!("issue-{issue_number}")`.
2. `sync_project_branch` remains the source of truth for checking out/syncing `ralph/issue-{n}` and must remain behaviorally unchanged.
3. Resume decision is only: `worktree/.ralph/projects/issue-{n}/prompt.md` exists (`is_file()`), evaluated after worktree is on `ralph/issue-{n}`.
4. If resume is true, run `ralph run --project issue-{n}`.
5. If resume is false, run `ralph auto --idea <idea> --project-id issue-{n}`.
6. No slug-based fallback (`discover_project_ids`, remote slug branch probing, prior slug ID inference) may influence dispatch decisions.

**Required code changes**
1. In `src/daemon/process.rs`, update `spawn_ralph_auto` and command builder to accept `project_id: Option<&str>`.
2. In `src/daemon/process.rs`, append `--project-id <id>` only when `project_id` is `Some`.
3. In `src/daemon/runtime.rs`, compute `project_id` once per dispatch and pass it through to `spawn_ralph_auto`.
4. In `src/daemon/runtime.rs`, remove dispatch-path calls to `discover_project_ids` and `discover_project_from_remote_branches`.
5. In `src/daemon/runtime.rs`, remove the extra project-branch checkout block after `sync_project_branch`; do not duplicate branch checkout logic.
6. In `src/daemon/worktree.rs`, change `create_worktree` return type to `Result<PathBuf>` and `verify_worktree_branch` to `Result<()>`.
7. In `src/daemon/worktree.rs`, remove `prior_project_id` extraction logic from `verify_worktree_branch`.
8. In `src/git/branch.rs` (or equivalent branch-creation path), make `maybe_create_project_branch` idempotent: if current HEAD branch already equals target project branch, return `Ok(())` without branch creation.
9. Keep existing behavior unchanged for manual `ralph auto --idea` without daemon-provided `--project-id`.

**Branch format validation**
1. Add daemon validation that configured branch format is compatible with `ralph/{project_id}`.
2. Validation rule is exact: formatting `project_id = "issue-1"` must produce `ralph/issue-1`.
3. Validation runs before any dispatch side effects (prefer daemon startup; first-dispatch precheck is acceptable only if no worktree/git mutation occurs first).
4. On failure, daemon logs a clear error and refuses to dispatch that task.

**Legacy slug-branch warning**
1. During dispatch, scan local branches in the managed worktree.
2. If any branch matches `ralph/*` and is not `ralph/issue-*` and not `ralph/daemon/*`, emit one warning per dispatch attempt.
3. Warning content must include: detected legacy branch name, issue number, and that daemon is starting fresh as `issue-{n}` instead of resuming.

**Acceptance criteria**
1. Fresh dispatch for issue `42` invokes `ralph auto --idea ... --project-id issue-42`.
2. Resume dispatch for issue `42` invokes `ralph run --project issue-42` when `.ralph/projects/issue-42/prompt.md` exists.
3. Dispatch never resumes from slug-based IDs or slug branch discovery.
4. Daemon-managed project execution branches are `ralph/issue-{n}` only; `ralph/daemon/{task_id}` housekeeping branches remain unchanged and out of scope.
5. No regression in `sync_project_branch` remote synchronization behavior.
6. `maybe_create_project_branch` no longer errors when daemon already checked out the target project branch.
7. Incompatible `git.branch_format` prevents dispatch with explicit error.
8. Legacy slug branch presence triggers warning and fresh `issue-{n}` behavior.
9. Existing slug-based behavior for non-daemon/manual `ralph auto --idea` remains unchanged.

**Testing requirements**
1. Update unit tests in `src/daemon/process.rs`:
- Existing long-idea test passes `None` project ID and asserts unchanged args.
- Add test asserting `Some("issue-42")` adds `--project-id issue-42`.
2. Add/update unit tests in `src/daemon/runtime.rs`:
- Resume file check returns true only when `.ralph/projects/issue-{n}/prompt.md` exists.
- Branch-format validation passes default format and fails incompatible custom format.
3. Update unit tests in `src/daemon/worktree.rs` for new `PathBuf`/`Result<()>` signatures.
4. Add/update tests for idempotent project-branch creation in `src/git/branch.rs` (or equivalent).
5. Update conformance tests under `src/validate/`:
- Remove or rewrite obsolete discovery-path test tied to slug discovery internals.
- Add test: daemon passes `--project-id issue-{n}` on fresh dispatch.
- Add test: resume is driven by `prompt.md` existence and uses `ralph run --project`.
- Add test: incompatible branch format blocks dispatch.
- Add behavioral test proving no slug fallback is used (assert outcomes and commands, not private function call counts).

**Out of scope**
1. No changes to `sync_project_branch` logic.
2. No changes to `spawn_ralph_run` behavior.
3. No migration/cleanup of existing legacy `ralph/{slug}` branches.
4. No GitHub issue label/comment workflow changes.