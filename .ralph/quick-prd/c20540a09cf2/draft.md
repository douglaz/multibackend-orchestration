Now I have a comprehensive understanding of the entire flow. Let me produce the engineering specification.

---

## Summary

The daemon currently assumes the repository it operates in has a populated working tree with at least one commit on `HEAD`. When given a bare repository, a freshly-`git init`'d repo with zero commits, or an empty directory, the worktree creation step fails with `fatal: Not a valid object name` (or `fatal: Not a git repository`), and `ralph auto` inside an empty worktree has no project files to build upon. This feature adds support for the daemon to bootstrap repositories from scratch — initializing git state, seeding an initial commit, and allowing the `ralph auto` child to operate in a worktree that starts with zero application files.

## Acceptance Criteria

1. **Zero-commit repository**: When the daemon's `repo_root` points to a git repository with no commits, the daemon automatically creates an initial empty commit before attempting `git worktree add`, and worktree creation succeeds.
2. **Empty working tree**: `ralph auto` running inside a worktree that contains only `.ralph/` (workspace scaffolding) and `.git` completes without error. The orchestration pipeline treats this as a greenfield project where the implementer creates all files from scratch.
3. **Non-git directory**: When `repo_root` is not a git repository, the daemon initializes it with `git init` and an empty commit before proceeding with worktree creation.
4. **Existing repos unaffected**: Repositories that already have commits follow the current code path with no behavioral change.
5. **Idempotency**: The bootstrap logic is safe to run multiple times (e.g., across daemon restarts); re-initializing an already-initialized repo or re-creating an existing empty commit is a no-op.
6. **PR flow works**: After the child completes in a bootstrapped repo, the push-and-PR flow succeeds (the branch has a real commit history relative to `HEAD`).

## Technical Approach

### 1. Repo bootstrap in `worktree.rs` (or new `src/daemon/bootstrap.rs`)

Add a `ensure_repo_ready(repo_root: &Path) -> Result<()>` function called at the top of `dispatch_task()` before `create_worktree()`:

```
ensure_repo_ready(repo_root):
  1. If repo_root is not a git repo (git rev-parse --git-dir fails):
     - Run git init repo_root
  2. If HEAD is unborn (git rev-parse HEAD fails):
     - Configure git user if not set (user.name="ralph-daemon", user.email="ralph@localhost")
     - Run git commit --allow-empty -m "initial commit (ralph-daemon bootstrap)"
```

This is called once per dispatch, but all operations are idempotent. The git-user configuration is scoped to the repo (`--local`) so it doesn't pollute the global config. If a user-level config already exists, the config step is skipped.

### 2. Worktree creation — handle unborn HEAD gracefully

The existing `create_worktree()` in `src/daemon/worktree.rs` uses `HEAD` as the start-point. After `ensure_repo_ready()`, `HEAD` is guaranteed to resolve. No change needed to `create_worktree()` itself, but the error message should be improved to suggest running bootstrap if it somehow fails.

### 3. `ralph auto` in empty worktree — no changes needed

`ensure_workspace()` in `auto.rs` already auto-creates the `.ralph/` workspace if missing. The quick-PRD + project-creation + orchestration pipeline does not require pre-existing application files; the implementer role generates them from the spec. The only requirement is that the worktree is a git directory (which `git worktree add` guarantees). No changes to `auto.rs` are needed.

### 4. Remote push — ensure remote exists

The post-completion PR flow in `runtime.rs` calls `git push -u origin <branch>`. For a locally-bootstrapped repo with no remote, this will fail. Add a guard in the PR-creation path: skip push+PR if no `origin` remote is configured, and log a warning instead. The daemon already handles PR-creation failures gracefully (task still moves to `completed`), so this is a minor hardening.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/bootstrap.rs` (new) | `ensure_repo_ready()` — git init + empty commit if needed |
| `src/daemon/mod.rs` | Re-export `bootstrap` module |
| `src/daemon/runtime.rs` | Call `ensure_repo_ready()` at the top of `dispatch_task()`, before worktree creation |
| `src/daemon/worktree.rs` | Improve error message on `create_worktree()` failure to mention unborn HEAD |
| `src/daemon/runtime.rs` (PR path) | Guard `git push` against missing `origin` remote |
| `src/validate/tests_daemon.rs` | New conformance tests (see Testing Strategy) |
| `src/validate/mock_scripts.rs` | Add mock scripts for empty-repo scenarios |

## Testing Strategy

### New conformance tests (in `tests_daemon.rs`)

1. **`daemon_bootstrap_empty_repo`**: Create a `git init` repo with zero commits. Start daemon in single-iteration mode with a mock issue. Assert: initial commit is created, worktree is created, child is spawned and completes, task reaches `completed` state.

2. **`daemon_bootstrap_non_git_dir`**: Create a plain directory (no `.git`). Start daemon in single-iteration mode. Assert: git repo is initialized, initial commit is created, dispatch succeeds.

3. **`daemon_bootstrap_idempotent`**: Run `ensure_repo_ready()` twice on the same repo. Assert: only one initial commit exists, no errors on second run.

4. **`daemon_bootstrap_existing_repo_noop`**: Run `ensure_repo_ready()` on a repo that already has commits. Assert: no new commits are added, HEAD is unchanged.

5. **`daemon_no_remote_skip_push`**: Complete a task in a repo with no `origin` remote. Assert: task still reaches `completed` state, warning is logged, no push error propagated.

### Existing test preservation

All 39+ existing daemon conformance tests continue to pass because the `RalphHarness` already creates repos with initial commits — the bootstrap logic is a no-op for them.

### Manual/integration test

Run the daemon against a real empty GitHub repo (created via `gh repo create --add-readme=false`) with a `ralph:ready` labeled issue. Verify end-to-end: bootstrap → worktree → orchestration → PR created.

## Out of Scope

- **Bare repository support** (`git clone --bare`): Bare repos have fundamentally different semantics (no working tree). Supporting them would require `git clone` instead of `git worktree add`. Not needed for the stated use case.
- **Remote repository cloning**: The daemon assumes a local repo path. Cloning from a remote URL before starting is the user's responsibility.
- **Custom initial commit content**: The bootstrap commit is always empty. Seeding a repo with template files (e.g., `.gitignore`, `README.md`) is left to the user or a future feature.
- **Multi-remote support**: Only `origin` is considered for push/PR. Supporting other remote names is out of scope.
- **Workspace template customization for empty repos**: The default templates from `ralph init` are sufficient for greenfield projects.