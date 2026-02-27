---
artifact: prompt-review
project: test-daemon-on-empty-dirs-and-new-projec
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-14T17:20:58Z
---

# Prompt Review

## Issues Found
- The prompt is internally inconsistent about **bare repositories**: it cites them as a target scenario, then marks them out of scope. This can lead to incorrect implementation and test expectations.
- Workspace initialization timing is ambiguous: it says bootstrap should run in `dispatch_task()`, but daemon startup may already depend on `.ralph` paths. This affects feasibility in truly empty directories.
- The identity-check instruction conflicts with its intent: checking only `git config --local` does not “respect existing global/system identity,” so bootstrap may overwrite or mis-detect valid config.
- `has_diff()` guidance is too broad: “diff command fails => no diff” can hide real git errors (permissions, repo corruption). This risks false negatives.
- Test plan has naming drift (`new_empty` vs `new_bare_dir`) and mixes unit/conformance concerns, which makes implementation loops slower and error-prone.
- Some acceptance criteria are not directly testable as written (for example “completes without error” without explicit observable outcomes/logs/artifacts).
- PR fallback behavior is underspecified for exact command and failure handling, so downstream loops may diverge in behavior and logging.

## Refined Prompt
Implement daemon bootstrap support for greenfield repositories so task dispatch works when `repo_root` is a non-git directory or a git repo with unborn `HEAD`, while preserving current behavior for normal repos.

### 1) Goal
Enable `ralph daemon` to safely prepare repository state before worktree creation, then run normal orchestration and PR flow with best-effort remote handling.

### 2) Scope
In scope:
1. Non-git directory bootstrap (`git init` + initial empty commit + `.ralph` workspace init).
2. Zero-commit git repo bootstrap (initial empty commit if `HEAD` is unborn).
3. Idempotent repeated bootstrap calls.
4. PR flow guards for missing `origin` and missing remote default branch.
5. Fix diff detection false positives in single-commit/invalid-base scenarios.
6. Validate conformance coverage for all new behavior.

Out of scope:
1. Bare repository execution (`git --bare` repos). Detect and return a clear non-fatal task error message.
2. Cloning remote URLs.
3. Template/seeded starter files beyond current `ralph init` behavior.
4. Multi-remote PR support beyond `origin`.

### 3) Functional Requirements
1. Add `ensure_repo_ready(repo_root: &Path) -> Result<()>` in `src/daemon/bootstrap.rs`.
2. Call `ensure_repo_ready()` at the start of `dispatch_task()` before `create_worktree()`.
3. Bootstrap behavior:
   - If `repo_root` is not a git repo: run `git init`.
   - If repo is bare: return explicit unsupported error.
   - If `HEAD` is unborn: create one empty bootstrap commit.
   - Bootstrap commit must disable signing and hooks: `-c commit.gpgsign=false` and `--no-verify`.
   - If user identity is not resolvable, set repo-local fallback identity (`ralph-daemon`, `ralph@localhost`) without overwriting existing resolvable identity.
   - If `.ralph/` is missing, initialize workspace using internal init logic (preferred) or equivalent idempotent path.
4. Existing repos with valid `HEAD` must remain behaviorally unchanged.
5. PR flow behavior after task completion:
   - If `origin` missing: log warning and skip push/PR, task remains `completed`.
   - If `origin` exists and base branch is known: current push + PR flow.
   - If `origin` exists but no default base can be resolved: push branch, attempt `gh pr create --head <branch>` without `--base`; if PR fails, log warning and keep task `completed`.
6. `has_diff()` behavior:
   - If `git diff <base>...HEAD` fails only due to invalid/missing revision, treat as `false` (no diff) and log debug/warn.
   - For other git execution failures, do not silently coerce to no-diff.

### 4) Required File Changes
1. `src/daemon/bootstrap.rs` (new): bootstrap logic.
2. `src/daemon/mod.rs`: export module.
3. `src/daemon/runtime.rs`: invoke bootstrap pre-worktree; add `origin` guard in PR path.
4. `src/daemon/github.rs`: `has_origin_remote()` helper and safe `has_diff()` invalid-revision handling.
5. `src/validate/harness.rs`: add constructors for bare directory and zero-commit repo.
6. `src/validate/tests_daemon.rs` (or `tests_daemon_bootstrap.rs`): new conformance tests.
7. `src/validate/mod.rs`: register new tests module if split.

### 5) Acceptance Criteria
1. Non-git directory:
   - Daemon initializes git repo and creates bootstrap commit.
   - `.ralph` workspace exists after bootstrap.
   - Worktree creation and task dispatch succeed.
2. Zero-commit repo:
   - Exactly one bootstrap commit is created before worktree add.
   - Dispatch succeeds.
3. Existing initialized repo:
   - `HEAD` SHA and commit count unchanged by bootstrap.
4. Idempotency:
   - Calling bootstrap twice does not add extra commits or fail.
5. No `origin`:
   - Push/PR skipped with warning; task ends `completed`.
6. Missing remote default branch:
   - PR attempt uses `--head` fallback; failure is warning-only; task ends `completed`.
7. Single-commit/invalid-base diff path:
   - No false “has changes” when no divergence exists.

### 6) Validate Test Requirements
Add/extend conformance tests to cover:
1. `daemon_bootstrap_non_git_dir`
2. `daemon_bootstrap_zero_commit_repo`
3. `daemon_bootstrap_idempotent`
4. `daemon_bootstrap_existing_repo_noop`
5. `daemon_pr_no_origin_skips_push_and_pr`
6. `daemon_pr_no_default_branch_fallback_head_only`
7. `daemon_has_diff_invalid_base_returns_false` (unit or conformance, but must be automated)

Test assertions must verify concrete artifacts/state:
1. Commit count and `HEAD` presence.
2. `.ralph` existence.
3. Task final state (`completed`).
4. Warning log substrings for skip/fallback paths.

### 7) Implementation Constraints
1. Keep bootstrap operations idempotent and side-effect minimal.
2. Do not regress existing daemon conformance tests.
3. Prefer typed errors and explicit log messages over generic stderr passthrough.
4. Keep behavior deterministic under absent global git config.
