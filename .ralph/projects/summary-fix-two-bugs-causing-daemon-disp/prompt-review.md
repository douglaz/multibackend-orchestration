---
artifact: prompt-review
project: summary-fix-two-bugs-causing-daemon-disp
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-20T23:50:16Z
---

# Prompt Review

## Issues Found
- The runtime wiring is ambiguous: it proposes two different ways to resolve `base_branch` at dispatch time, which can lead to inconsistent implementations.
- Function naming is inconsistent (`create_project_branch()` vs `maybe_create_project_branch()`), which increases the chance of editing the wrong path.
- Test targets are partly unclear (`tests_daemon.rs` is referenced, but project guidance points to `src/validate/` modules like `tests_run`), so conformance coverage is underspecified.
- Several acceptance criteria are not fully measurable (for example, “logic remains unchanged” without a concrete assertion target).
- Line-number references are brittle and may drift, making the prompt harder to execute reliably.
- Error-handling expectations for the new `git branch -f` command are implied but not explicit.
- The prompt asks for “no hardcoded master/main” but does not explicitly require this across newly touched call sites and tests.

## Refined Prompt
Implement a fix for daemon-dispatched branching so new project branches always start from the latest remote base branch, even when the local base branch is stale.

### Goal
Eliminate merge-conflict-prone stale-branch behavior in daemon workflows by ensuring:
1. Local base branch is force-synced to remote during project branch sync.
2. Parentless project branches are created from `origin/{base_branch}`, not local `{base_branch}`.
3. Base branch name always comes from config (`workspace.config.git.base_branch`), never hardcoded.

### Scope
Apply changes only to daemon/project branching flow and related tests. Do not change unrelated orchestration behavior.

### Required Code Changes
1. In `src/git/branch.rs`, update:
   - `sync_project_branch` signature to:
     `pub fn sync_project_branch(repo_root: &Path, issue_number: u32, base_branch: &str) -> Result<()>`
   - After `git fetch origin`, run:
     `git branch -f <base_branch> origin/<base_branch>`
   - Map failures through `RalphError::Orchestration` with clear command context.
   - Keep existing issue-branch sync behavior unchanged.

2. In `src/project/lifecycle.rs`, in `maybe_create_project_branch()`:
   - Keep parent-project path unchanged:
     `resolve_branch_name(&workspace.config.git.branch_format, parent_id)`
   - Change parentless path to:
     `format!("origin/{}", workspace.config.git.base_branch)`

3. In `src/daemon/runtime.rs`:
   - Add `pub base_branch: String` to `DaemonRuntimeConfig`.
   - Populate it once from workspace config during runtime startup.
   - In `dispatch_task()`, pass `&config.base_branch` into `sync_project_branch(...)`.

### Constraints
- Do not hardcode `"master"` or `"main"` in new logic.
- Prefer function-level targeting over line-number targeting.
- Preserve existing behavior outside the stale-base fix.

### Acceptance Criteria
- [ ] `sync_project_branch()` accepts `base_branch: &str`.
- [ ] `sync_project_branch()` runs `git branch -f {base_branch} origin/{base_branch}` after fetch.
- [ ] `maybe_create_project_branch()` uses `origin/{base_branch}` for parentless branch creation.
- [ ] Parent-project branch derivation logic is unchanged.
- [ ] `dispatch_task()` passes configured base branch through runtime config.
- [ ] No new hardcoded base-branch names are introduced.
- [ ] Existing affected tests are updated and still pass.
- [ ] New tests prove both stale-local and custom-base-branch scenarios.

### Test Requirements
1. Update existing unit tests that call `sync_project_branch` to pass `base_branch`.
2. Add unit test in `src/git/branch.rs`:
   - `sync_project_branch_force_updates_local_base_branch`
   - Setup stale local base vs advanced remote base.
   - Assert local `<base_branch>` ref equals `origin/<base_branch>` after sync.
3. Add unit test in `src/git/branch.rs`:
   - `sync_project_branch_force_updates_custom_base_branch`
   - Repeat using non-default base branch (for example `main`).
4. Add/extend test for lifecycle branch creation:
   - Verify parentless project branch is created from `origin/{base_branch}`.
5. Add conformance coverage in `src/validate/` (prefer `src/validate/tests_run.rs` unless a new module is cleaner):
   - Scenario: local base stale, remote base advanced, daemon dispatch creates branch from remote base.
   - If new validate module is added, register it in `src/validate/mod.rs`.

### Validation Commands
Run and pass:
1. `nix develop -c cargo check`
2. `nix develop -c cargo test`
3. If conformance tests are added/changed:
   - `nix build -L`
   - `./result/bin/ralph validate --bin ./result/bin/ralph --filter <new-or-updated-test-name>`

### Out of Scope
- `origin/HEAD` fallback semantics outside this fix.
- Interactive non-daemon workflows.
- Auto-migration/repair of pre-existing stale branches.
- Unrelated PR/rebase flows and unrelated config schema changes.
