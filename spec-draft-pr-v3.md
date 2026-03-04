## Summary

Add draft PR lifecycle management to the daemon. Instead of creating a PR only at task completion, the daemon parent creates a **draft PR** after the child process pushes its first commit (detected by checking for commits ahead of the base ref), then **marks it ready** (`gh pr ready`) when the task completes successfully. This gives reviewers and CI early visibility into in-progress work. Failed tasks leave the PR as draft. No new persistent state is introduced — the existing PR is discovered via `find_existing_pr` at each decision point.

To ensure the draft PR is created as early as possible, the **child process commits and pushes the initial prompt immediately after project creation** (and optional prompt review), before the planner runs. This closes the gap between the daemon-side watcher being ready to create the draft PR and the child's first push — without this early push, the watcher would idle until the Planning → Implementing phase transition, delaying draft PR creation by the full duration of the planning phase.

**Changes from v2 spec to address review feedback:**

1. **(v2 items 1–7 carried forward unchanged)**: Q1 `--pr-url` CLI contract, Q2 commit-ahead detection via `rev-list`, Q3 dynamic branch resolution, Q4 watcher lifecycle on `ChildHandle`, Q5 retry on completion blocking, Q6 no-diff closure safety for non-draft PRs, Q7 expanded test coverage.
2. **Early prompt checkpoint (new)**: The child process (`ralph auto` / `ralph run`) performs a commit-and-push of the initial prompt file immediately after `create_project()` completes and optional prompt review finishes, **before** the main orchestration loop begins. This is implemented as a new function `commit_and_push_initial_prompt()` in `src/workflow/orchestrator.rs`, called between the prompt review block and the main loop entry. It is gated on the same `auto_commit` and `skip_commit` flags that gate phase transition checkpoints (respecting `workflow.auto_commit=false`). The commit message is `chore({project_id}): initial prompt` and does not use `--allow-empty` (skipped if there are no staged changes). This ensures the watcher detects committed divergence within its first poll cycle (30s after child spawn) rather than waiting for the Planning → Implementing transition.

## Acceptance Criteria

1. After `dispatch_task` spawns the child, a background task (the "draft PR watcher") polls the worktree for committed divergence from the base branch using `git rev-list --count origin/{base_branch}...HEAD`. Once the count is ≥1, the watcher reads `current_branch` from the worktree, pushes the branch, and creates a draft PR via `gh pr create --draft`. The PR title uses the issue title (or `ralph: {task_id}` as fallback) and the body contains a "Work in progress" placeholder referencing the issue number.
2. Draft PR creation is best-effort: failures are logged but do not block task execution. If the branch never has a commit ahead of base before the child exits, no draft PR is created.
3. The draft PR head branch is determined by reading `current_branch` from the worktree at creation time. This correctly handles both `ralph/issue-{N}` and `ralph/{project_id}` branches regardless of what `ChildHandle.branch` was set to at dispatch.
4. Once the draft PR is created, its URL is (a) written to `.ralph/.draft_pr_url` in the worktree and (b) passed to the child process. For (b), if the child was already spawned before the draft PR was created, the file is the only mechanism. For re-dispatches where a PR already exists at spawn time, the URL is passed as `--pr-url <url>` to `ralph auto` / `ralph run`.
5. The draft PR watcher's `JoinHandle` is stored on `ChildHandle` as `draft_pr_handle`. It shares the existing `watcher_cancel` `CancellationToken`. In `collect_children`, both `watcher_handle` (artifact watcher) and `draft_pr_handle` are cancelled and joined *before* calling `complete_task`, ensuring no race between draft PR creation and `handle_pr_flow`.
6. When the child exits with success (`ralph:completed`), `handle_pr_flow` detects the existing draft PR via `find_existing_pr`, calls `edit_pr` to update the title and body with final metadata (diff stat, `Closes #N`, project ref), then queries the PR's draft state via `is_pr_draft` and calls `gh pr ready <url>` only if still in draft. The order is: `edit_pr` → conditional `mark_pr_ready`.
7. If `edit_pr` or `mark_pr_ready` fails on a completed task, the error propagates from `handle_pr_flow`. `complete_task` retries `handle_pr_flow` up to 2 additional times with a 30-second delay between attempts. If all retries fail, the final error is logged as a warning and `complete_task` proceeds with label swap and cleanup (ensuring the task is not permanently stuck).
8. When the child exits with success but `has_diff_with_base` returns false (no net code changes), and a draft PR exists for the branch, `handle_pr_flow` queries `is_pr_draft` for that PR. If the PR is still in draft, it posts a comment ("Task completed with no net code changes") and closes the PR via `gh pr close`. If the PR is *not* draft (was manually marked ready), it is left alone and a no-diff comment is posted to the issue as today.
9. When the child exits with failure (`ralph:failed`), no PR state changes occur — the draft PR remains as-is, signaling incomplete work.
10. No new durable state files are written beyond `.ralph/.draft_pr_url` (cleaned up with the worktree). PR existence is always derived from `find_existing_pr` at the point of use.
11. Auto-rebase uses dynamic branch resolution: it reads `current_branch` from the task worktree at rebase time rather than using `ChildHandle.branch`, matching `handle_pr_flow`'s existing pattern.
12. `spawn_ralph_auto` and `spawn_ralph_run` accept an optional `pr_url: Option<&str>` parameter. When provided, `--pr-url <url>` is appended to the command args. `AutoArgs` and `RunArgs` gain a `--pr-url` field.
13. The child process commits and pushes the initial prompt immediately after project creation and optional prompt review, before entering the main orchestration loop. This is gated on `workflow.auto_commit` and `skip_commit`. The commit message is `chore({project_id}): initial prompt`. This ensures the daemon-side draft PR watcher detects divergence within its first poll cycle rather than waiting for the Planning → Implementing phase transition.
14. A new conformance test validates the draft-PR-then-mark-ready lifecycle end to end. A separate test validates the no-diff-with-draft-PR closure path (draft only). Additional tests cover: watcher cancellation on fast child exit, existing ready PR not closed on no-diff, `auto_commit=false` with no child pushes, and the `--pr-url` argument plumbing.

## Technical Approach

### 1. `github.rs` — Add `--draft` support, `mark_pr_ready`, `close_pr`, `is_pr_draft`, and `has_commits_ahead_of_base`

**Modify `create_pr_with_body_file`** (line 602): Add a `draft: bool` parameter. When `true`, append `"--draft"` to the `args` vector before executing `gh pr create`. Update all existing call sites to pass `draft: false` to preserve current behavior. Also update `create_pr` (line 554) with the same `draft: bool` parameter for consistency.

**Add `mark_pr_ready`**:
```rust
pub fn mark_pr_ready(pr_url: &str) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "ready", pr_url])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to mark PR ready: {err}")))?;
    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr ready failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}
```

**Add `is_pr_draft`** to query whether a PR is still in draft state:
```rust
pub fn is_pr_draft(pr_url: &str) -> Result<bool> {
    let output = Command::new("gh")
        .args(["pr", "view", pr_url, "--json", "isDraft", "-q", ".isDraft"])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to query PR draft state: {err}")))?;
    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr view --json isDraft failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}
```

**Add `close_pr`** for closing stale draft PRs on no-diff completion:
```rust
pub fn close_pr(pr_url: &str) -> Result<()> {
    let output = Command::new("gh")
        .args(["pr", "close", pr_url])
        .output()
        .map_err(|err| RalphError::Orchestration(format!("failed to close PR: {err}")))?;
    if !output.status.success() {
        return Err(RalphError::Orchestration(format!(
            "gh pr close failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}
```

**Add `has_commits_ahead_of_base`** — commit-only divergence check for the draft PR watcher:
```rust
/// Check if HEAD has at least one committed revision ahead of the base branch.
/// Unlike `has_diff_with_base`, this ignores uncommitted working-tree changes
/// and only returns true when there are pushable commits.
pub fn has_commits_ahead_of_base(
    worktree_path: &std::path::Path,
    base_branch: &str,
) -> Result<bool> {
    let base_ref = format!("origin/{base_branch}");
    // Verify the base ref exists; if not, fall back to detect_base_branch
    let base = {
        let check = Command::new("git")
            .args(["rev-parse", "--verify", &base_ref])
            .current_dir(worktree_path)
            .output();
        if check.map(|o| o.status.success()).unwrap_or(false) {
            base_ref
        } else {
            detect_base_branch(worktree_path)
        }
    };
    let output = Command::new("git")
        .args(["rev-list", "--count", &format!("{base}..HEAD")])
        .current_dir(worktree_path)
        .output()
        .map_err(|err| {
            RalphError::Orchestration(format!("failed to run git rev-list --count: {err}"))
        })?;
    if !output.status.success() {
        return Ok(false); // Cannot determine; treat as no commits ahead
    }
    let count: u32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    Ok(count > 0)
}
```

### 2. `runtime.rs` — `ChildHandle` gains `draft_pr_handle`

Add a new field to `ChildHandle` in `src/daemon/mod.rs`:

```rust
pub struct ChildHandle {
    pub pid: u32,
    pub pgid: u32,
    pub child: tokio::process::Child,
    pub watcher_cancel: CancellationToken,
    pub watcher_handle: Option<JoinHandle<()>>,
    pub draft_pr_handle: Option<JoinHandle<()>>,  // NEW
    pub branch: String,
    pub log_file: PathBuf,
    pub last_rebase_at: Option<Instant>,
    pub last_rebase_failure_sha: Option<String>,
}
```

`ChildHandle.branch` remains `ralph/daemon/{task_id}` — it is *not* updated to track the worktree branch. All code that needs the actual branch reads `current_branch` from the worktree dynamically.

### 3. `runtime.rs` — Draft PR creation via background task

**Add `spawn_draft_pr_watcher`** — launched at the end of `dispatch_task`, after the child is inserted into `children`. This returns a `JoinHandle<()>` stored as `draft_pr_handle`.

The watcher:

1. Waits an initial delay (30 seconds) to give the child time to make its first commit. Because the child now commits the initial prompt before the planner runs (see section 8), this delay is sufficient — the prompt commit and push typically complete within 5–10 seconds of child start.
2. Polls every 15 seconds (up to ~10 minutes total, configurable) calling `has_commits_ahead_of_base(&wt_path, &base_branch)`. This ensures only committed changes trigger PR creation.
3. Once commits ahead are detected:
   a. Reads `current_branch(&wt_path)` for the actual branch name.
   b. Checks `has_origin_remote(&wt_path)`. Skips if missing.
   c. Calls `push_branch(&wt_path, &branch)`. If push fails, logs and retries on next poll (transient failure) or gives up (non-transient, e.g., no remote).
   d. Calls `find_existing_pr(&owner, &repo, &branch)` — if a PR already exists (retrigger scenario), writes the URL to `.ralph/.draft_pr_url` and exits.
   e. Builds a minimal draft PR title (from issue title or `ralph: {task_id}`) and body (`> Work in progress for #{issue_number}`).
   f. Calls `create_pr_with_body_file(..., draft: true)`.
   g. Writes the returned URL to `{wt_path}/.ralph/.draft_pr_url`.
   h. Logs and exits.
4. Uses `watcher_cancel` (the same `CancellationToken` shared with the artifact watcher) to abort if the child exits before a draft PR is created.
5. All errors are best-effort (log and retry or give up).

**Note on timing**: The child's early prompt commit (section 8) ensures the watcher's first poll at T+30s reliably finds committed divergence. Without this early commit, the watcher would not detect divergence until the Planning → Implementing transition, which can be minutes later. The watcher does **not** perform the push itself when the child has `auto_commit=true`, since the child's `commit_and_push_initial_prompt()` already pushes. The watcher detects the pushed commit on the remote via `has_commits_ahead_of_base` (which uses the local commit history, not the remote) and then calls `push_branch` which is a no-op if the remote is already up to date.

**Implementation detail**: The watcher holds `wt_path: PathBuf`, config values (owner, repo, base_branch, issue_number, task_id), and the `CancellationToken`. It does not hold references to `children` and cannot mutate `ChildHandle`.

**Dispatch integration**: After creating the `ChildHandle` and inserting it, launch the watcher:

```rust
let draft_pr_handle = if !config.owner.is_empty() && !config.repo.is_empty() {
    let owner = config.owner.clone();
    let repo = config.repo.clone();
    let base = config.base_branch.clone();
    let tid = task_id.clone();
    let wt = wt_path.clone();
    let cancel = watcher_cancel.clone();
    Some(tokio::spawn(async move {
        spawn_draft_pr_watcher(owner, repo, base, issue_number, tid, wt, cancel).await;
    }))
} else {
    None
};
// Store in ChildHandle — requires inserting draft_pr_handle after construction
if let Some(handle) = children.get_mut(&issue_number) {
    handle.draft_pr_handle = draft_pr_handle;
}
```

### 4. `runtime.rs` — Update `collect_children` to join draft PR watcher

In `collect_children` (line 1238-1253), after cancelling and joining `watcher_handle`, also join `draft_pr_handle`:

```rust
handle.watcher_cancel.cancel();
if let Some(join_handle) = handle.watcher_handle.take() {
    if let Err(err) = join_handle.await {
        eprintln!("warning: artifact watcher join failed for {task_id}: {err}");
    }
}
// NEW: join draft PR watcher to prevent race with handle_pr_flow
if let Some(join_handle) = handle.draft_pr_handle.take() {
    if let Err(err) = join_handle.await {
        eprintln!("warning: draft PR watcher join failed for {task_id}: {err}");
    }
}
```

This must also be added to `kill_aborted_children` (line 1298) and `drain_all_children` (line 1343) for consistency.

**This guarantees**: By the time `complete_task` runs, the draft PR watcher has fully exited. There is no race between `gh pr create --draft` (watcher) and `edit_pr`/`mark_pr_ready` (`handle_pr_flow`).

### 5. `runtime.rs` — Update `handle_pr_flow` for mark-ready

**Modify the no-diff early return** (line 1951-1964): Before returning, check for an existing PR on the branch. If one exists *and is still draft*, close it. If it exists but is not draft, leave it alone:

```rust
if !has_changes {
    // Check for stale draft PR to close
    let existing_pr = {
        let owner = config.owner.clone();
        let repo = config.repo.clone();
        let br = branch.clone();
        spawn_blocking_op(move || github::find_existing_pr(&owner, &repo, &br))
            .await
            .unwrap_or(None)
    };
    if let Some(url) = existing_pr {
        let is_draft = {
            let url_clone = url.clone();
            spawn_blocking_op(move || github::is_pr_draft(&url_clone))
                .await
                .unwrap_or(false)
        };
        if is_draft {
            let comment = format!(
                "Task `{task_id}` completed with no net code changes. Closing draft PR."
            );
            let url_clone = url.clone();
            let _ = spawn_blocking_op(move || {
                Command::new("gh")
                    .args(["pr", "comment", &url_clone, "--body", &comment])
                    .output()
            }).await;
            let url_clone = url.clone();
            if let Err(err) = spawn_blocking_op(move || github::close_pr(&url_clone)).await {
                eprintln!("warning: failed to close stale draft PR {url} for {task_id}: {err}");
            }
        }
    }
    // Post the existing no-diff comment to the issue (unchanged)
    // ...
    return Ok(());
}
```

**Modify the existing-PR branch** (line 2074-2091): After `edit_pr`, query draft state and conditionally call `mark_pr_ready`:

```rust
Some(url) => {
    eprintln!("editing existing PR for {task_id}: {url}");
    // ... existing edit_pr call (unchanged, errors still propagate) ...

    // Mark ready if still in draft
    let url_for_ready = url.clone();
    let is_draft = spawn_blocking_op(move || github::is_pr_draft(&url_for_ready))
        .await
        .unwrap_or(false); // On query failure, skip mark-ready (already not draft)
    if is_draft {
        let url_for_ready = url.clone();
        spawn_blocking_op(move || github::mark_pr_ready(&url_for_ready)).await?;
        eprintln!("marked PR ready for {task_id}: {url}");
    }
}
```

Error handling: `mark_pr_ready` errors propagate via `?` from `handle_pr_flow`, same as `edit_pr` errors. The retry logic in `complete_task` handles transient failures (see section 6).

**Modify the no-existing-PR branch** (line 2093-2121): Pass `draft: false` to `create_pr_with_body_file`. This handles edge cases where the draft PR was deleted or the daemon was upgraded.

### 6. `runtime.rs` — Update `complete_task` with retry for PR flow

Change `complete_task` (line 1384-1394) from catch-and-continue to retry-then-continue:

```rust
if terminal_label == "ralph:completed" {
    let workspace_root = config.workspace_root.clone();
    let wt_path = worktree::task_worktree_path(&workspace_root, task_id);
    if wt_path.exists() {
        let max_retries = 2;
        let mut last_err = None;
        for attempt in 0..=max_retries {
            match handle_pr_flow(config, task_id, issue_number, &wt_path).await {
                Ok(()) => {
                    last_err = None;
                    break;
                }
                Err(err) => {
                    if attempt < max_retries {
                        eprintln!(
                            "warning: PR flow failed for {task_id} (attempt {}/{}): {err}; retrying in 30s",
                            attempt + 1, max_retries + 1
                        );
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                    last_err = Some(err);
                }
            }
        }
        if let Some(err) = last_err {
            eprintln!("warning: PR flow failed for {task_id} after {} attempts: {err}", max_retries + 1);
        }
    }
}
```

### 7. `runtime.rs` — Update `auto_rebase_phase` for dynamic branch resolution

In `auto_rebase_phase` (line 1474-1476), replace the static `h.branch.clone()` with a dynamic read of `current_branch` from the task worktree:

```rust
for issue_number in &issue_numbers {
    let task_id = format_task_id(&config.owner, &config.repo, *issue_number);
    let wt_path = worktree::task_worktree_path(&config.workspace_root, &task_id);

    let branch = match spawn_blocking_op({
        let wt = wt_path.clone();
        move || github::current_branch(&wt)
    }).await {
        Ok(b) => b,
        Err(err) => {
            eprintln!("auto-rebase: skip {task_id} — failed to read branch: {err}");
            continue;
        }
    };

    let (last_rebase_at, last_failure_sha) = match children.get(issue_number) {
        Some(h) => (h.last_rebase_at, h.last_rebase_failure_sha.clone()),
        None => continue,
    };
    // ... rest unchanged, uses `branch` from above ...
}
```

### 8. Child-side early prompt commit — `commit_and_push_initial_prompt()`

This is the new section that addresses the feedback. The child process commits and pushes the initial prompt immediately after project creation and optional prompt review, **before** the main orchestration loop begins. This ensures the daemon-side draft PR watcher detects committed divergence on its first poll.

**Add `commit_and_push_initial_prompt()` in `src/workflow/orchestrator.rs`:**

```rust
/// Commit and push the initial prompt file so the daemon-side draft PR watcher
/// can create a draft PR before the planner runs.  This is best-effort: failure
/// is logged but does not block orchestration.
fn commit_and_push_initial_prompt(
    workspace_root: &Path,
    project_id: &str,
    branch_format: &str,
    sign_commits: bool,
) -> Result<()> {
    let repo_root = workspace_root
        .parent()
        .ok_or_else(|| RalphError::Orchestration("workspace root has no parent path".to_owned()))?;
    if !is_git_repo(repo_root) {
        return Ok(());
    }

    // Stage only .ralph/ orchestration files (prompt, project.toml, etc.)
    // rather than `git add -A`, to avoid staging any unexpected files.
    run_git(repo_root, &["add", ORCHESTRATION_STATE_PATH_PREFIX])?;

    // Check if there's anything to commit (skip if already committed,
    // e.g. on resume).
    let status = run_git(repo_root, &["diff", "--cached", "--quiet"])
        .err()
        .is_some(); // non-zero exit = there are staged changes
    if !status {
        return Ok(()); // Nothing staged, skip commit
    }

    let message = format!("chore({project_id}): initial prompt");
    let mut commit_args = vec!["commit", "-m", &message];
    if sign_commits {
        commit_args.insert(1, "-S");
    }
    run_git(repo_root, &commit_args)?;

    let branch = crate::git::branch::resolve_branch_name(branch_format, project_id);
    run_git(repo_root, &["push", "origin", &format!("HEAD:{branch}")])?;

    Ok(())
}
```

**Call site in `run()` (orchestrator.rs, between the prompt review block at line ~474 and the main loop at line ~480):**

```rust
        // ... end of prompt review block (line 474) ...

        // Commit and push the initial prompt so the daemon-side draft PR
        // watcher can create a draft PR before the planner runs.
        if effective.workflow.auto_commit && !options.skip_commit {
            if let Err(err) = commit_and_push_initial_prompt(
                &self.workspace.root,
                &state.project_id,
                &effective.global.git.branch_format,
                effective.global.git.sign_commits,
            ) {
                warn!("failed to commit/push initial prompt (non-fatal): {err}");
            }
        }

        let feature_target = options.loops.unwrap_or(1);
        // ... main loop begins (line ~480) ...
```

**Key design decisions:**

- **Gated on `auto_commit` and `skip_commit`**: Respects the same flags as phase transition checkpoints. When `auto_commit=false` or `--skip-commit` is passed, no early push happens and the draft PR watcher falls back to detecting the first phase transition commit (existing behavior).
- **Stages only `.ralph/`**: Uses `git add .ralph/` rather than `git add -A` to avoid accidentally staging files that don't belong to the prompt. This is safer than a full `git add -A`.
- **Skips if nothing staged**: On resume (`ralph run --project`), the prompt is already committed. The `git diff --cached --quiet` check prevents empty commits.
- **Does not use `--allow-empty`**: Unlike phase transition checkpoints, this commit should only exist when there's actual prompt content to push.
- **Best-effort**: Push failures are logged as warnings and do not block orchestration. The draft PR watcher will detect the commit on its next poll and push from the daemon side.
- **Commit message**: `chore({project_id}): initial prompt` — distinct from phase transition checkpoints which use `chore({project_id}): checkpoint {from} -> {to}`.

**Interaction with `ensure_clean_start_for_new_loop`**: This function (line 4773) checks for uncommitted changes outside `.ralph/` before starting a new loop. Since `commit_and_push_initial_prompt` only stages and commits `.ralph/` contents, and pushes them, the working tree is clean when `ensure_clean_start_for_new_loop` runs. No conflict.

**Interaction with the draft PR watcher**: The child commits and pushes the prompt. On the daemon side, the watcher's first poll at T+30s calls `has_commits_ahead_of_base`, which checks the local commit log (`git rev-list --count origin/{base}..HEAD`). Since the child pushed, the local HEAD is ahead of the base. The watcher then calls `push_branch` (a no-op since the branch is already pushed) and creates the draft PR. Net result: the draft PR is created ~30s after child spawn, not minutes later.

### 9. Child process CLI changes — `--pr-url` argument

**`process.rs`**: Update `spawn_ralph_auto` and `spawn_ralph_run` to accept `pr_url: Option<&str>`:

```rust
pub async fn spawn_ralph_auto(
    ralph_bin: &str,
    worktree_path: &Path,
    idea: &str,
    log_file: &Path,
    pr_url: Option<&str>,  // NEW
) -> Result<SpawnedChild> {
    // ... existing code ...
    if let Some(url) = pr_url {
        args.push("--pr-url".to_owned());
        args.push(url.to_owned());
    }
    // ...
}
```

Same for `spawn_ralph_run`.

**`dispatch_task`**: At dispatch time, check for an existing PR on the branch and pass it:

```rust
let existing_pr_url = {
    let owner = config.owner.clone();
    let repo = config.repo.clone();
    let issue_br = format!("ralph/issue-{issue_number}");
    spawn_blocking_op(move || github::find_existing_pr(&owner, &repo, &issue_br))
        .await
        .unwrap_or(None)
};

let spawned = match effective_project_id.as_deref() {
    Some(project_id) => {
        process::spawn_ralph_run(&ralph_bin, &wt, project_id, &log_path, existing_pr_url.as_deref()).await?
    }
    None => {
        process::spawn_ralph_auto(&ralph_bin, &wt, &idea_clone, &log_path, existing_pr_url.as_deref()).await?
    }
};
```

**Argument parsing**: Add `--pr-url` to `AutoArgs` and `RunArgs` in the CLI parsing module. The child process receives but does not act on this value in this change (future follow-up integrates it into the child workflow). The flag is parsed and stored for forward-compatibility.

### 10. Child pushes during execution

The child process already pushes commits via `commit_and_push_phase_transition` when `workflow.auto_commit=true`. The new early prompt commit (section 8) adds an initial push **before** the planning phase, ensuring the draft PR is created as soon as possible.

When `workflow.auto_commit=false`, the child does not push during execution (neither the early prompt commit nor phase transitions). The draft PR watcher still detects local commits and pushes the branch itself as part of draft PR creation. Subsequent commits remain local until `handle_pr_flow` pushes at completion time. This is acceptable — the draft PR shows the initial commit state, and subsequent pushes happen at completion.

### 11. `.ralph/.draft_pr_url` convention

The draft PR watcher writes the URL to `{wt_path}/.ralph/.draft_pr_url` after creation. This is a single-line text file containing the PR URL. The `.ralph/` directory is already gitignored in worktrees (line 138 in `worktree.rs`). The file is cleaned up when the worktree is removed.

## Files & Modules

| File | Change |
|---|---|
| `src/daemon/github.rs:554` | Add `draft: bool` param to `create_pr`; append `"--draft"` when true |
| `src/daemon/github.rs:602` | Add `draft: bool` param to `create_pr_with_body_file`; append `"--draft"` when true |
| `src/daemon/github.rs` (new fn) | Add `mark_pr_ready(pr_url: &str) -> Result<()>` running `gh pr ready <url>` |
| `src/daemon/github.rs` (new fn) | Add `is_pr_draft(pr_url: &str) -> Result<bool>` running `gh pr view --json isDraft` |
| `src/daemon/github.rs` (new fn) | Add `close_pr(pr_url: &str) -> Result<()>` running `gh pr close <url>` |
| `src/daemon/github.rs` (new fn) | Add `has_commits_ahead_of_base(worktree_path, base_branch) -> Result<bool>` using `git rev-list --count` |
| `src/daemon/mod.rs:25` | Add `draft_pr_handle: Option<JoinHandle<()>>` field to `ChildHandle` |
| `src/daemon/runtime.rs` (new fn) | Add `spawn_draft_pr_watcher` async function — polls for committed divergence, then pushes and creates draft PR |
| `src/daemon/runtime.rs:1156` | Set `draft_pr_handle` on `ChildHandle` construction, launch watcher after insert |
| `src/daemon/runtime.rs:1238-1248` | In `collect_children`: cancel and join `draft_pr_handle` before calling `complete_task` |
| `src/daemon/runtime.rs:1291-1307` | In `kill_aborted_children`: cancel and join `draft_pr_handle` |
| `src/daemon/runtime.rs:1331-1348` | In `drain_all_children`: cancel and join `draft_pr_handle` |
| `src/daemon/runtime.rs:1384-1394` | In `complete_task`: add retry loop (up to 3 attempts, 30s delay) for `handle_pr_flow` |
| `src/daemon/runtime.rs:1474-1476` | In `auto_rebase_phase`: replace `h.branch.clone()` with `current_branch(&wt_path)` read |
| `src/daemon/runtime.rs:1951-1964` | In `handle_pr_flow` no-diff path: find existing PR, check `is_pr_draft`, close only if draft |
| `src/daemon/runtime.rs:2074-2091` | In `handle_pr_flow` existing-PR branch: add `is_pr_draft` check + conditional `mark_pr_ready` after `edit_pr` |
| `src/daemon/runtime.rs:2100` | Pass `draft: false` to `create_pr_with_body_file` in the no-existing-PR branch |
| `src/daemon/runtime.rs:1121-1136` | In `dispatch_task`: check for existing PR before spawn, pass `pr_url` to `spawn_ralph_auto`/`spawn_ralph_run` |
| `src/daemon/process.rs:27` | Add `pr_url: Option<&str>` param to `spawn_ralph_auto`; append `--pr-url` when provided |
| `src/daemon/process.rs:70` | Add `pr_url: Option<&str>` param to `spawn_ralph_run`; append `--pr-url` when provided |
| `src/workflow/orchestrator.rs` (new fn) | Add `commit_and_push_initial_prompt()` — commits and pushes `.ralph/` after prompt creation |
| `src/workflow/orchestrator.rs:~475` | Call `commit_and_push_initial_prompt()` between prompt review and main loop, gated on `auto_commit` and `skip_commit` |
| CLI arg parsing (AutoArgs/RunArgs) | Add `--pr-url` optional field to both arg structs |
| `src/validate/mock_scripts.rs` (new fn) | Add `daemon_mock_gh_draft_pr_script()` |
| `src/validate/tests_daemon.rs` (new tests) | 7 new conformance tests (see Testing Strategy) |

## Testing Strategy

### New mock script: `daemon_mock_gh_draft_pr_script()`

Create a new mock gh script (in `mock_scripts.rs`) that extends the existing `daemon_mock_gh_edit_pr_script` pattern with:

- **`gh pr create`**: Checks for `--draft` flag presence. Logs all args to `MOCK_GH_PR_CREATE_LOG` including a `DRAFT=yes/no` line. Returns synthetic PR URL `https://github.com/acme/widgets/pull/42`.
- **`gh pr ready`**: Logs the URL to `MOCK_GH_PR_READY_LOG`. Returns success.
- **`gh pr view --json isDraft`**: Reads `MOCK_GH_PR_IS_DRAFT` env var (default `true`). Returns the JSON value.
- **`gh pr close`**: Logs the URL to `MOCK_GH_PR_CLOSE_LOG`. Returns success.
- **`gh pr list --head`**: After `MOCK_GH_PR_CREATE_LOG` exists (post-creation), returns the synthetic PR URL. Before creation, returns empty. Configurable via `MOCK_GH_PR_LIST_RESULT` env var to force a specific result (for testing retrigger/existing-PR scenarios).
- **`gh pr edit`**: Logs args to `MOCK_GH_PR_EDIT_LOG`. Returns success.
- **`gh pr comment`**: Logs body to `MOCK_GH_PR_COMMENT_LOG`. Returns success.
- Inherits standard issue/label/comment handling from the existing `daemon_mock_gh_script` pattern.

### New conformance test 1: `draft_pr_created_then_marked_ready`

1. Set up `RalphHarness::new_daemon()` with the draft PR mock gh script and `daemon_mock_ralph_with_commit_script` (creates a commit + bare remote).
2. Inject a single mock issue with `ralph:ready` label.
3. Run daemon in single-iteration mode.
4. **Assert draft PR created**: Read `MOCK_GH_PR_CREATE_LOG`, verify `DRAFT=yes` marker.
5. **Assert PR edited**: Read `MOCK_GH_PR_EDIT_LOG`, verify `--title` and `--body-file` args.
6. **Assert PR marked ready**: Read `MOCK_GH_PR_READY_LOG`, verify the URL matches synthetic PR URL.
7. **Assert ordering**: `MOCK_GH_PR_CREATE_LOG` written before `MOCK_GH_PR_EDIT_LOG` before `MOCK_GH_PR_READY_LOG`. Use sequence counters (each mock logs an incrementing counter from a shared file).

### New conformance test 2: `draft_pr_closed_on_no_diff_completion`

1. Set up with draft PR mock gh script and standard `daemon_mock_ralph_script` (exits 0 with no changes).
2. Set `MOCK_GH_PR_LIST_RESULT` to return a PR URL (simulating a draft PR from a prior dispatch).
3. Set `MOCK_GH_PR_IS_DRAFT` to `true`.
4. Inject mock issue, run daemon.
5. **Assert `is_pr_draft` queried**: Verify via log.
6. **Assert PR closed**: `MOCK_GH_PR_CLOSE_LOG` contains the URL.
7. **Assert PR create not called**: `MOCK_GH_PR_CREATE_LOG` does not exist.
8. **Assert PR ready not called**: `MOCK_GH_PR_READY_LOG` does not exist.

### New conformance test 3: `ready_pr_not_closed_on_no_diff_completion`

1. Same setup as test 2, but set `MOCK_GH_PR_IS_DRAFT` to `false`.
2. Inject mock issue, run daemon.
3. **Assert PR NOT closed**: `MOCK_GH_PR_CLOSE_LOG` does not exist.
4. **Assert no-diff comment posted**: `MOCK_GH_PR_COMMENT_LOG` or issue comment log contains the standard no-diff message.

### New conformance test 4: `draft_pr_create_failure_is_nonfatal`

1. Set up with a mock gh that returns non-zero for `gh pr create --draft`.
2. `daemon_mock_ralph_with_commit_script` creates a commit (so divergence is detected).
3. Run daemon in single-iteration mode.
4. **Assert task completes**: Terminal label is `ralph:completed`, not `ralph:failed`.
5. **Assert non-draft PR created via fallback**: `handle_pr_flow` creates a new (non-draft) PR on the completion path. Verify `MOCK_GH_PR_CREATE_LOG` shows the fallback `create_pr_with_body_file(draft: false)` call.

### New conformance test 5: `draft_pr_watcher_cancelled_on_fast_exit`

1. Set up with `daemon_mock_ralph_script` that exits immediately (0 exit, no commit).
2. No mock PR exists (`pr list --head` returns empty).
3. Run daemon in single-iteration mode.
4. **Assert no PR operations**: Neither `MOCK_GH_PR_CREATE_LOG` nor `MOCK_GH_PR_READY_LOG` exist.
5. **Assert clean exit**: No panics or join errors in daemon stderr output.

### New conformance test 6: `pr_url_passed_to_child_on_retrigger`

1. Set `MOCK_GH_PR_LIST_RESULT` to return a PR URL (simulating existing PR from prior dispatch).
2. Use a mock ralph script that logs its full command-line args to a file.
3. Run daemon in single-iteration mode.
4. **Assert `--pr-url` arg present**: Read the ralph args log, verify `--pr-url https://github.com/acme/widgets/pull/42` appears.

### New conformance test 7: `early_prompt_commit_creates_divergence`

(New test for the early prompt checkpoint — validates that the child's initial commit is visible to the watcher)

1. Set up with draft PR mock gh script and a mock ralph script that:
   a. Writes `.ralph/projects/test/prompt.md` (simulating `create_project`).
   b. Runs `git add .ralph/ && git commit -m "chore(test): initial prompt" && git push origin HEAD:ralph/test` (simulating `commit_and_push_initial_prompt`).
   c. Sleeps 2 seconds (shorter than the watcher's 30s initial delay, to verify the watcher finds the commit on first poll).
   d. Exits 0 with `ralph:completed` label.
2. Inject mock issue, run daemon.
3. **Assert draft PR created**: `MOCK_GH_PR_CREATE_LOG` exists and shows `DRAFT=yes`.
4. **Assert draft PR created before task completion**: Use sequence counters to verify `MOCK_GH_PR_CREATE_LOG` timestamp precedes `MOCK_GH_PR_EDIT_LOG` (the edit happens in `handle_pr_flow` at completion).
5. **Assert early commit exists**: Run `git log --oneline` on the worktree and verify a commit with message containing "initial prompt" exists.

### Existing test updates

- **`runtime_no_diff_pr_path`** (line 1654): The mock's `gh pr list --head` returns empty (no existing PR), so the new no-diff closure path finds no PR to close. The draft PR watcher never fires (no commits, no divergence). **No changes needed**.
- **`pr_metadata_verification`**: The e2e mock does not handle `--draft`. The watcher's `push_branch` will fail (no real remote) and silently give up. `pr list --head` returns empty. **No changes needed**.
- **Auto-rebase tests**: The branch resolution change (section 7) reads `current_branch` from the worktree instead of `ChildHandle.branch`. If existing auto-rebase tests mock the worktree, they continue to work because the worktree's HEAD determines the branch. If tests assert on `ChildHandle.branch` directly, update them to assert on the worktree's `current_branch` value instead.

## Out of Scope

- **Child-side PR URL usage**: `AutoArgs` and `RunArgs` gain the `--pr-url` field, but the child process does not read or act on it in this change. Integrating the PR URL into child-side behavior (e.g., posting status updates to the PR) is deferred to a follow-up.
- **Push failures during draft PR creation**: If the initial branch push for draft PR creation fails, the watcher retries on the next poll. Persistent push failures cause the watcher to give up after the timeout (~10 minutes). No special retry/backoff logic beyond the existing poll loop.
- **Draft PR body refinement during loops**: The draft PR body is a static placeholder. Updating it with intermediate diff stats or loop progress is deferred.
- **Interactive PRD draft PRs**: The PRD workflow (`run_prd_phase`) has its own lifecycle. Adding draft PRs to PRD-managed tasks is out of scope.
- **`_with_gh_bin` variants**: The new functions (`mark_pr_ready`, `is_pr_draft`, `close_pr`, `has_commits_ahead_of_base`) use the default `gh`/`git` binaries. Adding configurable binary path variants follows the existing pattern but is deferred unless tests require it.
- **Worktree cleanup changes**: Draft PRs remain open after worktree cleanup on failure. No changes to `cleanup_worktree_for_terminal_state`.
- **Branch-switch retrigger reuse test**: Testing that a task retriggered on a different branch correctly reuses or creates a new draft PR is complex and deferred. The implementation handles this via `find_existing_pr` at each decision point.
- **`complete_task` retry configurability**: The retry count (2 retries = 3 total attempts) and delay (30 seconds) are hardcoded. Making them configurable via `DaemonRuntimeConfig` is deferred.
- **Prompt review push**: The early prompt commit happens after prompt review (if enabled). Pushing twice — once before review, once after — is not worth the complexity. The single post-review push captures the finalized prompt.
