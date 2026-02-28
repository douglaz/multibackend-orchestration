## Summary

Daemon-dispatched tasks currently use a complex multi-path discovery mechanism to determine whether to resume an existing project or start fresh. This involves scanning worktree branches for `ralph/{slug}` patterns (`prior_project_id`), searching `.ralph/projects/` directories (`discover_project_ids`), and iterating remote `ralph/*` branches (`discover_project_from_remote_branches`). This complexity exists because `ralph auto --idea` generates slug-based project IDs (e.g., `ralph/implement-dark-mode`), creating a mismatch with the daemon's `ralph/issue-{n}` branch convention.

The fix is straightforward: pass `--project-id issue-{n}` when the daemon invokes `ralph auto`, guaranteeing the project ID matches the issue branch. Resume detection then reduces to a single file-existence check, and all slug-branch discovery code in the daemon dispatch path can be removed.

**Key interaction with existing code**: `dispatch_task` calls `sync_project_branch()` before spawning `ralph auto`, which creates/checks out the `ralph/issue-{n}` branch. When `ralph auto` subsequently calls `create_project` → `maybe_create_project_branch` with `git.auto_branch=true`, it will attempt to create `ralph/issue-{n}` again, causing a "branch already exists" error. This spec addresses this by making `maybe_create_project_branch` idempotent — skipping branch creation when the current HEAD is already on the target branch.

**Branch format assumption**: The daemon hardcodes `ralph/issue-{n}` as its branch convention via `sync_project_branch`. This spec requires that `git.branch_format` (if configured) is compatible with this convention, and adds a fail-fast check at daemon startup to prevent silent mismatches.

**Migration note**: Existing `ralph/{slug}` branches from prior daemon runs will no longer be discovered or resumed. Re-triggering those issues will start fresh `issue-{n}` projects. This is an intentional behavior change, and the spec adds a one-time warning log when a legacy slug branch is detected on a daemon-managed worktree.

## Acceptance Criteria

1. `spawn_ralph_auto` in `src/daemon/process.rs` accepts an optional `project_id: Option<&str>` parameter and passes `--project-id <value>` to the command when present.
2. `dispatch_task` in `src/daemon/runtime.rs` computes `let project_id = format!("issue-{issue_number}")` and passes it through to `spawn_ralph_auto`.
3. Resume detection is a single check: does `.ralph/projects/issue-{n}/prompt.md` exist on the `ralph/issue-{n}` branch (after `sync_project_branch`)?
4. `discover_project_ids()` call removed from dispatch flow.
5. `discover_project_from_remote_branches()` call removed from dispatch flow.
6. `create_worktree` return type reverted from `Result<(PathBuf, Option<String>)>` to `Result<PathBuf>`.
7. `prior_project_id` detection logic removed from `verify_worktree_branch` in `src/daemon/worktree.rs` (it returns `Result<()>` instead of `Result<Option<String>>`).
8. Separate project branch checkout block (lines 1059-1077 in `runtime.rs`) removed — `sync_project_branch` already puts the worktree on `ralph/issue-{n}`.
9. Daemon creates only `ralph/issue-{n}` *project execution branches*, never `ralph/{slug}` branches. (Note: `ralph/daemon/{task_id}` housekeeping branches used by `create_worktree` are unaffected and out of scope.)
10. No regressions in remote branch synchronization behavior (`sync_project_branch` untouched).
11. Manual `ralph auto --idea` (without daemon) continues to derive slug-based project IDs — no behavioral change.
12. `maybe_create_project_branch` is made idempotent: when the current HEAD is already on the target project branch (i.e., `sync_project_branch` already created it), skip branch creation instead of erroring.
13. Daemon startup validates that `git.branch_format` (if configured) is compatible with the `ralph/{project_id}` convention. If incompatible, the daemon logs an error and refuses to dispatch, rather than silently creating mismatched branches.
14. When the daemon detects a legacy `ralph/{slug}` branch on a worktree it manages for an issue, it logs a warning: `"legacy branch ralph/{slug} found on worktree for issue {n}; starting fresh as issue-{n} instead of resuming"`.
15. Existing conformance test `daemon::discover_project_id_ignores_dirs_without_state_json` is removed or updated to reflect the new dispatch logic.
16. New conformance tests assert: (a) daemon passes `--project-id issue-{n}`, (b) resume is detected via `.ralph/projects/issue-{n}/prompt.md`, (c) no slug-based fallback discovery occurs.

## Technical Approach

### 1. Make `maybe_create_project_branch` idempotent (`src/git/branch.rs` or `src/cli/auto.rs`)

Before creating a new branch, check whether HEAD is already on the target branch. If so, skip branch creation:

```rust
// In maybe_create_project_branch (or the caller in create_project)
let target_branch = format_project_branch(project_id); // e.g. "ralph/issue-42"
let current_branch = get_current_branch(repo_path)?;
if current_branch == target_branch {
    // Branch already exists and we're on it (daemon pre-created via sync_project_branch).
    // Skip creation — this is the idempotent path.
    return Ok(());
}
// ... existing branch creation logic ...
```

This resolves the blocking issue where `sync_project_branch` creates `ralph/issue-{n}` before `ralph auto` attempts to create it again.

### 2. Add branch format validation at daemon startup (`src/daemon/runtime.rs`)

At daemon initialization (or at the top of `dispatch_task`), validate the configured branch format:

```rust
fn validate_branch_format(config: &Config) -> Result<()> {
    let test_id = "issue-1";
    let expected = format!("ralph/{}", test_id);
    let actual = config.git.format_branch(test_id);
    if actual != expected {
        anyhow::bail!(
            "daemon requires git.branch_format='ralph/{{project_id}}' but config produces '{}' for project_id='{}'. \
             Daemon dispatch is disabled until this is corrected.",
            actual, test_id
        );
    }
    Ok(())
}
```

This is called once at daemon startup or lazily on first dispatch. It prevents silent dual-branch creation when users have custom branch formats.

### 3. Add `project_id` parameter to `spawn_ralph_auto` (`src/daemon/process.rs`)

Add `project_id: Option<&str>` to both `spawn_ralph_auto` and `build_ralph_auto_command`. When `Some`, append `["--project-id", project_id]` to the command args:

```rust
pub async fn spawn_ralph_auto(
    ralph_bin: &Path,
    worktree_path: &Path,
    idea: &str,
    project_id: Option<&str>,  // NEW
    log_file: &Path,
) -> Result<SpawnedChild>
```

In `build_ralph_auto_command`:
```rust
let mut cmd = Command::new(ralph_bin);
cmd.args(["auto", "--idea", idea]);
if let Some(pid) = project_id {
    cmd.args(["--project-id", pid]);
}
cmd.current_dir(worktree_path)
   .stdin(std::process::Stdio::null())
   .stdout(std::process::Stdio::from(file))
   .stderr(std::process::Stdio::from(file_clone));
```

Update the existing test `spawn_command_uses_long_idea_flag` and add a new test for the `project_id` variant.

### 4. Simplify dispatch flow in `dispatch_task` (`src/daemon/runtime.rs`)

Replace the current ~60-line discovery block (lines 1015-1077) with:

```rust
let project_id = format!("issue-{issue_number}");
let is_resume = {
    let wt = wt_path.clone();
    let pid = project_id.clone();
    spawn_blocking_op(move || {
        Ok(wt.join(".ralph/projects").join(&pid).join("prompt.md").is_file())
    }).await?
};
```

Add a legacy branch warning before the resume check:

```rust
// Warn if a legacy slug branch exists on this worktree
if let Ok(branches) = list_local_branches(&wt_path) {
    for branch in &branches {
        if branch.starts_with("ralph/") && !branch.starts_with("ralph/issue-") && !branch.starts_with("ralph/daemon/") {
            eprintln!(
                "dispatch: warning: legacy branch '{}' found on worktree for issue {}; \
                 starting fresh as {} instead of resuming",
                branch, issue_number, project_id
            );
        }
    }
}
```

Replace the spawn decision (lines 1160-1179) with:

```rust
let spawned = if is_resume {
    eprintln!("dispatch: task {task_id} resuming project_id={project_id}; using ralph run --project");
    process::spawn_ralph_run(&ralph_bin, &wt, &project_id, &log_path).await?
} else {
    eprintln!("dispatch: task {task_id} fresh dispatch with project_id={project_id}; using ralph auto --idea --project-id");
    process::spawn_ralph_auto(&ralph_bin, &wt, &idea_clone, Some(&project_id), &log_path).await?
};
```

Remove the `effective_project_id` variable, the `prior_project_id` destructure, and the project branch checkout block entirely.

### 5. Simplify `create_worktree` return type (`src/daemon/worktree.rs`)

- Change `create_worktree` return type from `Result<(PathBuf, Option<String>)>` to `Result<PathBuf>`.
- Change `verify_worktree_branch` return type from `Result<Option<String>>` to `Result<()>`.
- Remove the `prior_project` extraction logic from `verify_worktree_branch` (the `strip_prefix("ralph/").filter(...)` block). Keep the branch mismatch warning and force-checkout — the worktree still needs to be corrected to the daemon branch.
- Update all call sites to destructure `PathBuf` instead of `(PathBuf, Option<String>)`.

### 6. Remove discovery functions from dispatch path

- Remove the `discover_project_ids` function entirely **if** it has no other callers. Otherwise, remove only the call from `dispatch_task`.
- Remove the `discover_project_from_remote_branches` function entirely **if** it has no other callers. Otherwise, remove only the call from `dispatch_task`.

Check for other callers before deleting the functions themselves.

## Files & Modules

| File | Changes |
|---|---|
| `src/daemon/process.rs` | Add `project_id: Option<&str>` to `spawn_ralph_auto` and `build_ralph_auto_command`; pass `--project-id` arg when present; update test |
| `src/daemon/runtime.rs` | Add `validate_branch_format` check; replace discovery block with file-existence check; add legacy branch warning; pass `project_id` to `spawn_ralph_auto`; remove project-branch checkout block; remove/dead-code `discover_project_ids` and `discover_project_from_remote_branches` if no other callers |
| `src/daemon/worktree.rs` | Revert `create_worktree` return to `Result<PathBuf>`; simplify `verify_worktree_branch` to `Result<()>`; remove `prior_project` extraction logic |
| `src/git/branch.rs` (or `src/cli/auto.rs`) | Make `maybe_create_project_branch` idempotent — skip branch creation when HEAD is already on the target branch |
| `src/cli/auto.rs` | No changes to `--project-id` flag functionality (existing flag works as-is) |

## Testing Strategy

### Unit tests in `src/daemon/process.rs`

- Update `spawn_command_uses_long_idea_flag` to pass `None` for `project_id` and assert args are unchanged.
- Add `spawn_command_passes_project_id` that passes `Some("issue-42")` and asserts args include `["--project-id", "issue-42"]`.

### Unit tests in `src/daemon/runtime.rs`

- Test that `format!("issue-{}", 42)` produces `"issue-42"` (trivial but documents the format).
- Test resume detection: create a temp dir with `.ralph/projects/issue-42/prompt.md` and verify the file-existence check returns `true`; verify it returns `false` when the file is absent.
- Test `validate_branch_format` passes with default config and fails with a custom incompatible format.

### Unit tests in `src/daemon/worktree.rs`

- Update any existing tests that destructure `(PathBuf, Option<String>)` to expect `PathBuf`.

### Unit tests in `src/git/branch.rs` (or `src/cli/auto.rs`)

- Test `maybe_create_project_branch` when HEAD is already on the target branch — assert it returns `Ok(())` without error.
- Test `maybe_create_project_branch` when HEAD is on a different branch — assert it creates the branch as before.

### Conformance tests (updates and additions)

- **Remove or update** `daemon::discover_project_id_ignores_dirs_without_state_json` — this test encodes discovery behavior that no longer exists in the dispatch path.
- **Add** `daemon::dispatch_passes_project_id_to_ralph_auto` — assert that `spawn_ralph_auto` receives `--project-id issue-{n}` in the command args during fresh dispatch.
- **Add** `daemon::dispatch_resumes_via_prompt_md_existence` — assert that when `.ralph/projects/issue-{n}/prompt.md` exists, the daemon invokes `ralph run --project issue-{n}` instead of `ralph auto`.
- **Add** `daemon::dispatch_does_not_perform_slug_discovery` — assert that neither `discover_project_ids` nor `discover_project_from_remote_branches` is called during dispatch (verified by removing the calls and ensuring no test regressions).
- **Add** `daemon::branch_format_validation_rejects_incompatible_config` — assert that a non-default `git.branch_format` causes dispatch to fail with a clear error message.

### Integration / manual testing

- Trigger a fresh GitHub issue dispatch and verify the child process receives `ralph auto --idea <idea> --project-id issue-{n}`.
- Verify the resulting project lands in `.ralph/projects/issue-{n}/`.
- Retrigger the same issue and verify the daemon detects resume via `prompt.md` existence and invokes `ralph run --project issue-{n}`.
- Verify no `ralph/{slug}` branches are created during daemon dispatches.
- Verify manual `ralph auto --idea "something"` (no `--project-id`) still creates slug-based branches.
- Verify that dispatching an issue that previously ran under the old slug-based system logs the legacy branch warning and starts fresh under `issue-{n}`.

## Out of Scope

- **Removing `sync_project_branch()`**: Still required for remote-first branch synchronization.
- **Changing `checkout_branch_in_worktree`**: The remote-first checkout fix from PR #109 is preserved as-is.
- **Changing manual `ralph auto` behavior**: Slug-based project IDs remain the default when `--project-id` is not passed.
- **Migrating existing `ralph/{slug}` branches**: Existing remote branches from prior daemon runs are left in place. They will no longer be discovered or resumed by the daemon. **This is intentional**: re-triggering those issues starts fresh under the `issue-{n}` convention, with a warning log emitted to make the behavior change visible. No automated migration or cleanup of legacy branches is performed.
- **Removing `discover_project_ids` / `discover_project_from_remote_branches` if used elsewhere**: If these functions have callers outside `dispatch_task`, they are preserved but the calls from `dispatch_task` are removed.
- **Changes to `spawn_ralph_run`**: The resume path (`ralph run --project`) is unchanged.
- **GitHub issue label/comment workflow changes**: Not part of this change.
- **`ralph/daemon/{task_id}` housekeeping branches**: These are created by `create_worktree` for daemon internal use and are unrelated to project execution branches. They are not affected by this change and remain as-is. The acceptance criterion "daemon creates only `ralph/issue-{n}` branches" refers exclusively to *project execution branches* (the branches on which `ralph auto`/`ralph run` execute).