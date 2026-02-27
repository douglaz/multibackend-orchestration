I now have a comprehensive understanding. Let me produce the revised specification.

---

## Summary

The daemon currently assumes the repository it operates in has a populated working tree with at least one commit on `HEAD`. When given a bare repository, a freshly-`git init`'d repo with zero commits, or an empty directory, the worktree creation step fails with `fatal: Not a valid object name` (or `fatal: Not a git repository`), and `ralph auto` inside an empty worktree has no project files to build upon. This feature adds support for the daemon to bootstrap repositories from scratch — initializing git state, seeding an initial commit, and allowing the `ralph auto` child to operate in a worktree that starts with zero application files.

## Acceptance Criteria

1. **Zero-commit repository**: When the daemon's `repo_root` points to a git repository with no commits, the daemon automatically creates an initial empty commit before attempting `git worktree add`, and worktree creation succeeds.
2. **Empty working tree**: `ralph auto` running inside a worktree that contains only `.ralph/` (workspace scaffolding) and `.git` completes without error. The orchestration pipeline treats this as a greenfield project where the implementer creates all files from scratch.
3. **Non-git directory**: When `repo_root` is not a git repository, the daemon initializes it with `git init` and an empty commit before proceeding with worktree creation. The daemon also initializes the `.ralph` workspace (via `ralph init`) if not already present.
4. **Existing repos unaffected**: Repositories that already have commits follow the current code path with no behavioral change.
5. **Idempotency**: The bootstrap logic is safe to run multiple times (e.g., across daemon restarts); re-initializing an already-initialized repo or re-creating an existing empty commit is a no-op.
6. **PR flow degrades gracefully in bootstrapped repos**: After the child completes in a bootstrapped repo:
   - **If `origin` remote exists and the remote has a default branch**: push-and-PR succeeds normally.
   - **If `origin` remote exists but has no default branch** (e.g., brand-new empty GitHub repo): the daemon pushes the task branch and creates the PR using `--head <branch>` without a `--base` flag, letting GitHub infer the base from the repo's default branch setting. If PR creation fails (e.g., GitHub cannot determine a base), the task still reaches `completed` state and a warning is logged.
   - **If no `origin` remote is configured**: push and PR are skipped entirely, the task reaches `completed` state, and a warning is logged.

## Technical Approach

### 1. Repo bootstrap in new `src/daemon/bootstrap.rs`

Add a `ensure_repo_ready(repo_root: &Path) -> Result<()>` function called at the top of `dispatch_task()` before `create_worktree()`:

```
ensure_repo_ready(repo_root):
  1. If repo_root is not a git repo (git rev-parse --git-dir fails):
     - Run git init repo_root
  2. If HEAD is unborn (git rev-parse HEAD fails):
     - Set local git identity if not already configured:
       - Check: git config --local user.name
       - If missing: git config --local user.name "ralph-daemon"
       - Check: git config --local user.email
       - If missing: git config --local user.email "ralph@localhost"
     - Run git -c commit.gpgsign=false commit --allow-empty --no-verify \
           -m "initial commit (ralph-daemon bootstrap)"
  3. If .ralph/ dir does not exist under repo_root:
     - Run ralph init in repo_root (or call init::create_workspace directly)
```

**Robustness details** (addressing Review Issue 3):
- The bootstrap commit uses `-c commit.gpgsign=false` to override any global/system `commit.gpgsign=true` config.
- The bootstrap commit uses `--no-verify` to skip pre-commit hooks that may reject an empty commit or require tools not yet available in the repo.
- Git identity is set via `--local` config, scoped to the repo only. It checks whether a user name/email is already resolvable (from global or system config) before writing, so existing user config is respected.
- All operations are idempotent: `git init` on an existing repo is a no-op, the HEAD check prevents duplicate bootstrap commits, and `ralph init` on an existing workspace is a no-op.

### 2. Worktree creation — no changes needed

After `ensure_repo_ready()`, `HEAD` is guaranteed to resolve to a valid commit. The existing `create_worktree()` in `src/daemon/worktree.rs` uses `HEAD` as the start-point and will succeed. The error message on failure already includes git's stderr which is diagnostic enough.

### 3. `ralph auto` in empty worktree — no changes needed

`ensure_workspace()` in `auto.rs` already auto-creates the `.ralph/` workspace if missing. The quick-PRD + project-creation + orchestration pipeline does not require pre-existing application files; the implementer role generates them from the spec. The only requirement is that the worktree is a git directory (which `git worktree add` guarantees). No changes to `auto.rs` are needed.

### 4. Guard the PR flow against missing remote and bootstrapped-repo edge cases

The post-completion PR flow in `runtime.rs` → `handle_pr_flow()` needs two guards:

**4a. Skip push+PR when no `origin` remote exists** (addressing Review Issue 2):

Before calling `push_branch()`, check whether `origin` is configured:
```
git remote get-url origin
```
If this fails, log a warning ("no origin remote configured, skipping push and PR creation") and return early. The task still reaches `completed` state since `handle_pr_flow()` is best-effort.

**4b. Fix `has_diff()` / `detect_base_branch()` HEAD~1 fallback for single-commit repos** (addressing Review Issue 4):

In `detect_base_branch()`, when all candidates fail and it falls back to `HEAD~1`: on a bootstrapped repo with only the empty bootstrap commit plus the task's commits, `HEAD~1` may be the bootstrap commit itself (which is correct — the diff shows all new files). However, if the task branch has exactly one commit total (the bootstrap commit, with no new work), `HEAD~1` doesn't exist and `git diff HEAD~1...HEAD` fails, which `has_diff()` interprets as `!success` → `true` (false positive: "has changes" when there are none).

Fix: in `has_diff()`, if the `base...HEAD` diff command fails (non-zero exit due to invalid ref), treat it as "no divergence detected" and return `false` rather than `true`. This is safe because stage 1 (uncommitted changes check) already caught any real working-tree modifications. Add a log line when this fallback triggers.

### 5. Workspace auto-init in bootstrap (addressing Review Issue 1)

The original spec assumed `ralph daemon start` is always invoked in a directory that already has `.ralph/`. For a truly empty directory, `.ralph/` won't exist. Step 3 of `ensure_repo_ready()` (above) handles this by running `ralph init` if `.ralph/` is absent. This means the daemon can be pointed at a completely bare directory and will produce a fully functional workspace.

Note: `ralph daemon start` resolves `repo_root` from the current directory and `workspace_root` from `.ralph/daemon/tasks.json`'s parent path. The bootstrap must run before these paths are used. Since `dispatch_task()` already has `config.repo_root` available, and workspace root is derived from the task store path which is created by the daemon's own startup, the only missing piece is the `.ralph/` workspace itself — which `ensure_repo_ready()` now creates.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/bootstrap.rs` (new) | `ensure_repo_ready()` — git init + identity config + empty commit + workspace init. Deterministic commit (gpgsign=false, no-verify). |
| `src/daemon/mod.rs` | Re-export `bootstrap` module |
| `src/daemon/runtime.rs` | Call `ensure_repo_ready()` at the top of `dispatch_task()`, before worktree creation |
| `src/daemon/runtime.rs` (PR path) | In `handle_pr_flow()`, add early-return guard when `git remote get-url origin` fails |
| `src/daemon/github.rs` | In `has_diff()`, if `git diff {base}...HEAD` command itself fails (bad ref), return `false` instead of `true`. Add `has_origin_remote(worktree_path) -> bool` helper. |
| `src/validate/tests_daemon.rs` | New conformance tests (see Testing Strategy) |
| `src/validate/mock_scripts.rs` | Add mock scripts for empty-repo scenarios |
| `src/validate/harness.rs` | Add `new_empty()` constructor that creates a plain directory (no git init, no commit). Add `new_zero_commit()` that runs `git init` but makes no commit. |

## Testing Strategy

### Harness extensions (addressing Review Issue 5)

The existing `RalphHarness::new()` always creates a git repo with an initial commit. To test bootstrap scenarios, add two new constructors:

- **`RalphHarness::new_bare_dir(bin)`**: Creates only the temp directory and `repo_root` subdirectory. No `git init`, no `.ralph/`. Returns a harness whose `repo_root` is a plain directory.
- **`RalphHarness::new_zero_commit(bin)`**: Runs `git init` and configures user identity but does NOT create any commit. `HEAD` is unborn. No `.ralph/`.

Both constructors skip `ralph init` and the initial commit, so the daemon's bootstrap logic is the system under test.

### New conformance tests (in `tests_daemon.rs`)

1. **`daemon_bootstrap_zero_commit_repo`**: Use `new_zero_commit()` harness. Manually create `.ralph/daemon/tasks.json` with a pending task (since `ralph init` hasn't run, the test writes the minimal file structure directly). Start daemon in single-iteration mode with mock `gh` and mock `ralph`. Assert: `ensure_repo_ready()` creates initial commit, worktree is created, child is spawned, task reaches `completed` state.

2. **`daemon_bootstrap_non_git_dir`**: Use `new_bare_dir()` harness. Write tasks.json into a manually-created `.ralph/daemon/` directory. Start daemon in single-iteration mode. Assert: git repo is initialized, initial commit is created, `.ralph/` workspace is initialized, dispatch succeeds, task reaches `completed`.

3. **`daemon_bootstrap_idempotent`**: Use `new_zero_commit()` harness. Call `ensure_repo_ready()` twice on the same repo. Assert: only one commit exists after both calls (`git rev-list --count HEAD` == 1), no errors on second run.

4. **`daemon_bootstrap_existing_repo_noop`**: Use standard `RalphHarness::new()`. Call `ensure_repo_ready()`. Assert: commit count is unchanged (still 1, the original "chore: initial commit"), HEAD SHA is unchanged.

5. **`daemon_bootstrap_no_git_identity`**: Use `new_zero_commit()` harness. Unset `HOME` and `GIT_CONFIG_GLOBAL` env vars to ensure no global git config is available. Call `ensure_repo_ready()`. Assert: commit succeeds (the function sets local identity), `git config --local user.name` returns `ralph-daemon`.

6. **`daemon_no_remote_skip_push`**: Use standard harness with `write_daemon_mock_ralph_with_commit()` (which creates a bare remote). Remove the `origin` remote from the repo before starting the daemon (`git remote remove origin`). Start daemon in single-iteration mode. Assert: task reaches `completed` state, no push error propagated, stderr contains "no origin remote" warning.

7. **`daemon_has_diff_single_commit_no_changes`**: Unit test for `has_diff()` edge case. Create a repo with a single empty commit (bootstrap-style). Create a worktree from HEAD. Make no changes in the worktree. Call `has_diff()`. Assert: returns `false` (not a false positive). This validates the fix to the `HEAD~1` fallback.

### Existing test preservation

All 39+ existing daemon conformance tests continue to pass because `RalphHarness::new()` creates repos with an initial commit — `ensure_repo_ready()` is a no-op for them. The new harness constructors are additive; existing tests are unmodified.

### Manual/integration test

Run the daemon against a real empty GitHub repo (created via `gh repo create --add-readme=false --public`) with a `ralph:ready` labeled issue. Verify end-to-end: bootstrap → worktree → orchestration → PR created. This exercises the "remote exists but has no default branch" path from AC6.

## Out of Scope

- **Bare repository support** (`git clone --bare`): Bare repos have fundamentally different semantics (no working tree). Supporting them would require `git clone` instead of `git worktree add`. Not needed for the stated use case.
- **Remote repository cloning**: The daemon assumes a local repo path. Cloning from a remote URL before starting is the user's responsibility.
- **Custom initial commit content**: The bootstrap commit is always empty. Seeding a repo with template files (e.g., `.gitignore`, `README.md`) is left to the user or a future feature.
- **Multi-remote support**: Only `origin` is considered for push/PR. Supporting other remote names is out of scope.
- **Workspace template customization for empty repos**: The default templates from `ralph init` are sufficient for greenfield projects.
- **Automatic base-branch creation on empty remotes**: If a GitHub repo has no default branch at all (no commits on remote), `gh pr create` may fail because there's no base branch to target. The daemon does not push an initial commit to `origin/main` to seed the remote — this is the user's responsibility. The daemon logs a warning and still completes the task. A future enhancement could push the bootstrap commit to set up the remote's default branch.