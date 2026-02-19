## Summary

Replace Ralph's local durable state (`state.json`, `tasks.json`, event logs) with Git commits and GitHub issue labels as the sole sources of truth. Every workflow phase transition becomes an atomic commit-then-push to a per-issue project branch (`ralph/issue-<N>`). The daemon derives all task state from GitHub labels on every poll cycle. An interrupted run may lose unpushed work, but can never corrupt durable state. A fresh clone resumes from the last pushed ralph commit without ambiguity.

The implementation is split into three phases that can each be merged independently:
1. **Safety Baseline** — eliminate persisted state files, switch to in-memory tracking, derive tasks from labels
2. **Commit+Push Checkpointing** — structured commit messages with trailers, push-gated phase advancement
3. **Strict Startup Sync** — fetch-first branch resolution, local-discard semantics, position derivation from commit history

---

## Acceptance Criteria

1. User can delete entire local clone, re-clone, and `ralph run` resumes from the last pushed ralph commit on the project branch without manual intervention.
2. Workflow position (loop N, phase P) is fully derivable from the last ralph commit's subject line and trailers on `origin/<project-branch>`.
3. Task status (ready/in-progress/completed/failed/aborted) is fully derivable from GitHub issue labels alone; the daemon reads no local task file on startup.
4. Interrupted runs may lose unpushed local work by design, but remote state is never corrupted (a crash before `git push` leaves the remote unchanged).
5. Daemon starts fresh with zero local state files and reconstructs everything from GitHub + Git within the first poll cycle.
6. Zero persisted state files (`state.json`, `tasks.json`, event logs) exist in the implementation — Git history and GitHub labels are the only durable state.
7. A second daemon instance for the same repo fails at startup with a clear error due to an `flock`-based process lock under `/tmp`, keyed by canonical remote URL (not local clone path).
8. A successful step produces exactly one commit with the required subject format and trailers, followed by a successful push. Phase advancement is recorded only after push succeeds.
9. A crash after commit but before push discards the local commit on next startup via `git reset --hard origin/<branch>`; resume continues from the last pushed step.
10. Startup with no prior ralph commits on the branch begins at loop 1, phase 1 (Planning).
11. Commit history on any project branch contains a complete ralph transition trail; every transition commit has the required trailers.
12. Issues with multiple lifecycle labels (`ralph:ready`, `ralph:in-progress`, etc.) are normalized to `ralph:failed` and skipped.
13. A push+label split-brain (commit pushed, label update failed) is self-healing: on next startup, reconciliation derives completion from pushed commits before resetting labels, preventing duplicate re-execution.
14. Malformed ralph commits (subject/trailer mismatch) produce a deterministic outcome: the issue is labeled `ralph:failed`, and the branch is not re-entered until manually fixed.
15. All daemon execution occurs in isolated worktrees (`.ralph/daemon/worktrees/<task_id>/`); startup aborts with a diagnostic if the worktree has dirty state that cannot be attributed to ralph.
16. Phase transitions that produce no file changes (e.g., a reviewer approving with zero suggestions) still produce a commit via `--allow-empty`, ensuring the trailer trail is unbroken.
17. Poll and reconciliation paginate through all matching issues, not just the first 100.
18. A manual `ralph:aborted` label applied during execution takes precedence: the child is killed, and child-exit relabeling never overwrites an abort.
19. Branch bootstrap resolves the remote default branch explicitly via `git ls-remote --symref origin HEAD`, falling back to `origin/main` then `origin/master`, with an acceptance test covering empty/misconfigured remotes.

---

## Technical Approach

### Phase 1 — Safety Baseline

**1.1 Remove `state.json` persistence from non-orchestrator code**

The orchestrator (`src/workflow/orchestrator.rs`) currently maintains a `ProjectState` struct that it saves to disk via `save_project_state()` after every phase mutation. In Phase 1, we keep `ProjectState` as an in-memory struct during a single `ralph run` invocation — it is still the orchestrator's working memory — but remove all disk persistence of `state.json` outside the orchestrator's own process. Concretely:

- Remove `state.json` reads from `src/cli/status.rs`, `src/cli/history.rs`, `src/cli/tail.rs`, `src/cli/project.rs`, `src/cli/rollback.rs`. These commands must instead derive status from either (a) the git log on the project branch, or (b) become unavailable until Phase 2 lands the commit parser.
  - **Decision:** For Phase 1, `ralph status` / `ralph history` will parse the working-tree `state.json` if it exists (for backwards compat during transition), but will also accept a `--from-git` flag that derives state from the commit log. Phase 2 makes `--from-git` the default and removes the file path.
- Remove `state.json` creation in `src/project/lifecycle.rs::create_project()`. Project creation becomes: create the branch, write `prompt.md` to the working tree, commit with the initial ralph commit message (`ralph(issue-<N>): loop 0 planning -> planning`), push.
- Remove `save_project_state()` / `load_project_state()` from `src/project/lifecycle.rs`. Replace with `commit_project_state()` that performs the commit+push atomic step.
- Remove `recover_state_from_git()` — there is no state file to recover.

**1.2 Replace `TaskStore` with in-memory child map**

The `TaskStore` (`src/daemon/mod.rs`) currently persists `tasks.json` using file locks. Replace it with:

```rust
struct ChildMap {
    children: HashMap<u32, RunningChild>,  // issue_number -> child info
}

struct RunningChild {
    pid: u32,
    pgid: u32,
    child: tokio::process::Child,
    branch: String,
    task_id: String,
    log_file: PathBuf,
}
```

This is purely in-memory. On daemon restart, the map starts empty. The daemon discovers what work is in progress by querying GitHub labels (`ralph:in-progress`) and comparing against running children.

**Key changes in `src/daemon/runtime.rs`:**
- `reconcile_tasks()` → replaced by `reconcile_from_labels()`: queries `gh issue list --label ralph:in-progress`, and for any issue that has no local child running, checks Git first (see §1.6 Push/Label Reconciliation) before resetting the label.
- `poll_and_claim()` → unchanged in structure, but reads from GitHub instead of `TaskStore::load()`.
- `dispatch_task()` → no longer writes to `tasks.json`; inserts into `ChildMap`.
- `collect_children()` → on child exit, updates GitHub label directly (no intermediate file write).
- `complete_task()` → updates label on GitHub, removes from `ChildMap`.

**1.3 Single project branch policy**

Enforce the naming convention `ralph/issue-<issue_number>` for all daemon-managed branches. The project ID becomes `issue-<issue_number>` (deterministic from the issue number).

Changes:
- `src/daemon/worktree.rs::create_worktree()` — use `ralph/issue-<N>` instead of `ralph/daemon/{task_id}`.
- `src/git/branch.rs::resolve_branch_name()` — default format becomes `ralph/issue-{issue_number}`.
- Remove the `git.branch_format` config option (or deprecate it for daemon mode).

**1.4 Clean transient files on start**

On daemon startup and on each child worker startup:
- `rm -rf .ralph/tmp/**` in the worktree.
- Remove any leftover lock files.
- `git clean -fd --exclude .ralph` to remove untracked build artifacts.

Add a `clean_transient_files(worktree: &Path)` utility in `src/daemon/worktree.rs`.

**1.5 `flock`-based single-instance lock (keyed by remote URL)**

Add to daemon startup (`src/daemon/runtime.rs::run()`):

```rust
/// Derive lock identity from the canonical repository remote URL,
/// not the local clone path. This prevents two clones of the same
/// remote repo from running concurrent daemons on one host.
fn daemon_lock_path(repo_root: &Path) -> Result<PathBuf> {
    // Get the canonical remote URL
    let remote_url = run_git(repo_root, &["remote", "get-url", "origin"])?;
    // Normalize: strip trailing .git, lowercase, trim whitespace
    let normalized = remote_url.trim().trim_end_matches(".git").to_lowercase();
    let hash = sha256_hex(normalized.as_bytes());
    Ok(PathBuf::from(format!("/tmp/ralph-daemon-{hash}.lock")))
}

let lock_path = daemon_lock_path(&config.repo_root)?;
let lock_file = File::create(&lock_path)?;
if lock_file.try_lock_exclusive().is_err() {
    return Err(RalphError::DaemonAlreadyRunning { lock_path });
}
// Hold lock_file for lifetime of daemon (stored in a struct field)
```

This ensures that two clones of `github.com/acme/widgets` on the same host share a lock, while different repos get different locks. Falls back to hashing the local repo root path if no `origin` remote exists (local-only repo edge case).

**1.6 Push/Label Reconciliation (split-brain healing)**

When the daemon restarts and finds an issue labeled `ralph:in-progress` with no running child, it must not blindly reset to `ralph:ready`. The commit may have been pushed but the label update may have failed (push succeeded, label update crashed). The reconciliation protocol is:

```
for each ralph:in-progress issue with no running child:
    1. git fetch origin ralph/issue-<N>
    2. Parse last ralph commit on origin/ralph/issue-<N>
    3. If last commit is a terminal transition (-> completed or -> failed):
         → Set label to ralph:completed or ralph:failed (derived from commit)
         → Do NOT reset to ralph:ready
    4. If last commit is a mid-workflow transition (-> implementing, -> reviewing, etc.):
         → Reset label to ralph:ready (work was interrupted, needs re-execution from last checkpoint)
    5. If no ralph commits exist on the branch:
         → Reset label to ralph:ready (work never started)
```

This prevents the duplicate-execution bug where a successfully completed task gets re-run because its label was stuck on `ralph:in-progress`.

**1.7 Task claim with post-claim verification**

The current claim flow (`gh issue list --label ralph:ready` → `gh issue edit --add-label ralph:in-progress`) is not atomic — two daemons (or two poll cycles racing) could both claim the same issue. Add post-claim verification:

```rust
fn claim_and_verify(owner: &str, repo: &str, issue_number: u32) -> Result<bool> {
    // Step 1: Add ralph:in-progress label
    github::claim_issue(owner, repo, issue_number)?;

    // Step 2: Re-read issue labels to verify claim
    let issue = github::fetch_issue_labels(owner, repo, issue_number)?;
    let lifecycle_labels: Vec<&str> = issue.labels.iter()
        .filter(|l| LIFECYCLE_LABELS.contains(&l.as_str()))
        .collect();

    // Step 3: If issue now has both ralph:ready and ralph:in-progress,
    // that's expected (we just added in-progress). Remove ralph:ready.
    // If issue has unexpected labels (another daemon also claimed it),
    // back off: remove our ralph:in-progress and return false.
    if lifecycle_labels.contains(&"ralph:in-progress") && lifecycle_labels.len() <= 2 {
        // Remove ralph:ready now that we've claimed it
        let _ = github::remove_label(owner, repo, issue_number, "ralph:ready");
        return Ok(true);
    }

    // Claim conflict detected — back off
    let _ = github::release_claim(owner, repo, issue_number);
    Ok(false)
}
```

This provides best-effort uniqueness. True single-claim guarantee requires external coordination (out of scope for v1), but the verify-after-claim pattern prevents the most common race window.

**1.8 Abort label precedence**

When a user manually applies `ralph:aborted` during execution, it must take priority over any exit-status-driven label update. The precedence rules:

1. Before updating labels on child exit, re-read the issue's current labels from GitHub.
2. If `ralph:aborted` is present, preserve it — do not overwrite with `ralph:completed` or `ralph:failed`.
3. If `ralph:aborted` is detected, kill the child process group if still running.
4. During poll, issues with `ralph:aborted` are never claimed (already handled by `filter_claimable()`).

Implementation: modify `complete_task()` to check for external abort before writing the terminal label:

```rust
async fn complete_task(..., terminal_state: TaskState) {
    // Re-read labels from GitHub before applying terminal state
    let current_labels = github::fetch_issue_labels(owner, repo, issue_number)?;
    if current_labels.contains("ralph:aborted") {
        // External abort takes precedence — do not overwrite
        eprintln!("task {task_id}: externally aborted, preserving ralph:aborted label");
        children.remove(issue_number);
        return;
    }
    // ... proceed with normal terminal label update
}
```

**1.9 Isolated worktree enforcement**

All daemon child work MUST execute inside isolated worktrees under `.ralph/daemon/worktrees/<task_id>/`. The spec explicitly requires:

1. The daemon never executes orchestrator work in the main repo working tree.
2. On child dispatch, the worktree is verified clean before use (existing `clean_worktree()` in `src/daemon/worktree.rs:300-340`).
3. If `clean_worktree()` fails (e.g., uncommitted changes in tracked files that `git checkout -- .` cannot resolve), the dispatch fails with a diagnostic error rather than silently proceeding with dirty state.
4. The startup sequence explicitly verifies worktree isolation: if `.ralph/daemon/worktrees/` does not exist, create it; if it exists, reconcile (remove orphans per §3.4).

This ensures that startup reset/checkout rules (Phase 3) only affect daemon-managed worktrees, never the user's main checkout.

---

### Phase 2 — Commit+Push Checkpointing

**2.1 Commit message format and parser**

Define the ralph commit contract. Every phase transition commit must have:

Subject: `ralph(<project-id>): loop <N> <phase> -> <next-phase>`

Trailers (in git trailer format, blank line separated from body):
```
Ralph-Project: <project-id>
Ralph-Loop: <N>
Ralph-Phase: <next-phase>
```

Add `src/git/ralph_commit.rs` with:
- `struct RalphCommitInfo { project_id, loop_number, from_phase, to_phase }`
- `fn format_ralph_commit(info: &RalphCommitInfo) -> String` — builds subject + trailers
- `fn parse_ralph_commit(message: &str) -> Option<RalphCommitInfo>` — extracts from subject + validates trailers match
- `fn find_last_ralph_commit(workdir: &Path, project_id: &str) -> Option<(String, RalphCommitInfo)>` — walks `git log --format=%H%n%B` on the current branch, returns first match
- Validation: if subject and trailers disagree on project_id, loop, or phase, the commit is malformed → return `Err(MalformedCommit)` with details (not `None`).

**2.2 Malformed commit handling policy**

When a malformed ralph commit is encountered (subject/trailer mismatch, missing trailers, unparseable loop number), the following deterministic policy applies:

1. **During startup/resume (`derive_position`)**: If the *last* ralph commit on a branch is malformed, the task is marked `ralph:failed` via label update, and the branch is not entered. A diagnostic comment is posted on the GitHub issue explaining the malformed commit hash and the nature of the mismatch. The daemon skips this issue until a human fixes the branch (e.g., by amending the commit or force-pushing a corrected history).

2. **During history walking**: Malformed commits in the *middle* of the history (i.e., not the most recent) are logged as warnings but do not block operation. `find_last_ralph_commit` returns the most recent *valid* ralph commit.

3. **During commit creation**: `format_ralph_commit()` is the only way to create ralph commits, and it guarantees internal consistency by construction. Malformed commits should only arise from manual tampering or bugs.

**2.3 No-op phase transitions and `--allow-empty`**

Some phase transitions produce no file changes (e.g., a reviewer approving without any code suggestions, or QA passing with no new test artifacts). To maintain an unbroken trailer trail:

- `checkpoint_phase_transition()` always uses `git commit --allow-empty` when the index is clean after `git add -A`.
- To ensure deterministic artifact presence, each phase writes at minimum a status marker file: `.ralph/projects/<id>/loops/<NNN-slug>/<phase>_status.json` containing `{"phase": "<phase>", "result": "<outcome>", "timestamp": "<iso8601>"}`. This file is tiny and guarantees every transition has at least one file change, but `--allow-empty` remains as a safety net.

**2.4 Phase transition becomes commit+push**

Replace the current pattern (mutate `ProjectState` → save to `state.json`) with:

1. Execute phase work (invoke backend, collect output) — all in memory / temp files under `.ralph/tmp/`
2. Write final artifacts to the working tree (spec, impl-notes, review feedback, etc.) under `.ralph/projects/<id>/loops/<NNN-slug>/`
3. Write phase status marker (see §2.3)
4. `git add -A`
5. Commit with structured ralph message (using `--allow-empty` as fallback)
6. `git push origin <project-branch>`
7. Only after push succeeds: update in-memory state and proceed to next phase

If commit succeeds but push fails, the orchestrator logs the error and exits. On next startup (Phase 3), the local commit is discarded.

**Changes to `src/workflow/orchestrator.rs`:**

Replace the existing `persist_state()` call sites with a new `checkpoint_phase_transition()` method:

```rust
fn checkpoint_phase_transition(
    &self,
    workdir: &Path,
    project_id: &str,
    loop_number: u32,
    from_phase: Phase,
    to_phase: Phase,
    sign_commits: bool,
) -> Result<String> {
    // 1. Stage all changes
    run_git(workdir, &["add", "-A"])?;

    // 2. Build commit message
    let info = RalphCommitInfo { project_id, loop_number, from_phase, to_phase };
    let message = format_ralph_commit(&info);

    // 3. Commit (allow-empty for no-op transitions)
    let hash = run_git(workdir, &["commit", "--allow-empty", "-m", &message])?;

    // 4. Push
    push_branch(workdir, &format!("ralph/issue-{}", /* issue number */))?;

    Ok(hash)
}
```

This replaces the 6+ `persist_state()` call sites identified in the orchestrator:
- After planning (feature registered) → commit spec + transition
- After implementing (impl_notes written) → commit impl_notes + transition
- After QA pass/fail → commit QA report + transition
- After review suggestions/approval → commit review artifact + transition
- After committing (current commit phase) → subsumed; the commit phase and the checkpoint are the same commit
- After completing → commit verdict + transition

**2.5 Remove `commit_feature_loop()` from Phase::Committing**

The current `Phase::Committing` exists only to run `git commit`. With every phase transition now producing a commit, the separate Committing phase is unnecessary. Remove it from the `Phase` enum. The transition from Reviewing (approved) goes directly to Planning (with the approval commit being the checkpoint).

The existing commit message style configuration (`conventional`, `descriptive`, `minimal`) is replaced by the fixed ralph commit format. The reviewer-provided commit message can be appended to the commit body (after the trailers), but the subject line must follow the ralph format.

**2.6 Trailer validation on read**

When parsing commits (for resume, status, history), validate that:
- Subject matches `ralph(<id>): loop <N> <from> -> <to>`
- All three trailers are present
- Subject and trailer values agree
- Reject malformed commits per the policy in §2.2

---

### Phase 3 — Strict Startup Sync

**3.1 Fetch-first startup with robust default branch resolution**

On every startup (both daemon and `ralph run`):

```
1. git fetch origin                        # Always fetch before any branch decision
2. if origin/<project-branch> exists:
     git checkout <project-branch>
     git reset --hard origin/<project-branch>  # Discard any local-only commits
   else:
     base_ref = resolve_remote_default_branch()  # See below
     git checkout -b <project-branch> <base_ref>
3. clean_transient_files()
4. Parse last ralph commit → derive (loop, phase)
5. If no ralph commits found → start at loop 1, Phase::Planning
```

**Critical invariant:** No branch creation or checkout may reference a local-only ref. All refs must come from `origin/*`.

**Robust default branch resolution (`resolve_remote_default_branch`):**

`origin/HEAD` may be unset or invalid in some remotes (bare repos without `git remote set-head`, forks, mirrors). The resolution chain is:

```rust
fn resolve_remote_default_branch(workdir: &Path) -> Result<String> {
    // 1. Try git ls-remote --symref origin HEAD (explicit remote query)
    //    Parse "ref: refs/heads/<branch>\tHEAD" from output
    if let Ok(branch) = ls_remote_symref_head(workdir) {
        return Ok(format!("origin/{branch}"));
    }

    // 2. Try git symbolic-ref refs/remotes/origin/HEAD (local cached ref)
    if let Ok(refname) = symbolic_ref_origin_head(workdir) {
        if !refname.is_empty() {
            return Ok(refname);
        }
    }

    // 3. Try common branch names in order
    for candidate in &["origin/main", "origin/master"] {
        if revision_exists(workdir, candidate)? {
            return Ok(candidate.to_string());
        }
    }

    // 4. Fail with actionable error
    Err(RalphError::Orchestration(
        "Cannot determine remote default branch. Tried: git ls-remote --symref, \
         origin/HEAD, origin/main, origin/master. Please run \
         'git remote set-head origin --auto' or set 'git.default_branch' in ralph.toml."
            .into(),
    ))
}
```

Unlike the current `detect_base_branch()` in `src/daemon/github.rs:740-770` which falls back to local `main`/`master` and ultimately `HEAD~1`, this function **never** falls back to a local-only ref. The error message is actionable and tells the user exactly how to fix the configuration.

**Changes to `src/git/branch.rs`:**
- `ensure_project_branch()` → renamed to `sync_project_branch()`, always fetches first, always resets to origin.
- `create_branch()` → takes the result of `resolve_remote_default_branch()` as base, never a local branch name.
- Remove `merge_base_branch()` — the daemon always resets to origin; merging local state is prohibited.

**3.2 Position derivation from commit history**

Replace `load_project_state()` entirely with:

```rust
fn derive_position(workdir: &Path, project_id: &str) -> Result<WorkflowPosition> {
    match find_last_ralph_commit(workdir, project_id)? {
        Some((hash, info)) => {
            // Validate expected artifacts exist in the tree
            validate_committed_artifacts(workdir, &info)?;
            Ok(WorkflowPosition {
                loop_number: info.loop_number,
                phase: info.to_phase,  // to_phase of last commit = current position
                commit_hash: Some(hash),
            })
        }
        None => {
            // Fresh branch, no ralph commits
            Ok(WorkflowPosition {
                loop_number: 1,
                phase: Phase::Planning,
                commit_hash: None,
            })
        }
    }
}
```

**Malformed last commit handling in `derive_position`:** If `find_last_ralph_commit` encounters a malformed commit as the most recent ralph-prefixed commit, it returns `Err(MalformedCommit { hash, details })`. The caller (orchestrator or daemon) applies the policy from §2.2: label the issue `ralph:failed`, post a diagnostic comment, and skip.

The `WorkflowPosition` struct replaces `ProjectState` for startup purposes. The orchestrator builds its in-memory working state from this position plus the artifacts present in the tree.

**3.3 Artifact validation**

After deriving position from the last commit, validate that the expected artifacts for that phase exist in the working tree:

- If position is `(loop 3, Phase::Implementing)`, there must be a spec file under `loops/003-*/`
- If position is `(loop 2, Phase::Reviewing)`, there must be impl-notes and spec
- Missing expected artifacts = malformed state → error with diagnostic message

This validation happens in `derive_position()` and prevents silent corruption.

**3.4 Daemon reconciliation from labels (with push/label healing)**

On daemon startup:
1. Query `gh issue list --label ralph:in-progress --repo owner/repo` (paginated, see §3.6)
2. For each in-progress issue: check if a local child is running (answer: no, we just started)
3. For each orphaned issue: apply the push/label reconciliation protocol from §1.6 — check the Git branch for completion before blindly resetting to `ralph:ready`
4. Start normal poll loop

This replaces `reconcile_tasks()` which currently reads `tasks.json`.

**3.5 Label normalization**

When polling issues, enforce the single-lifecycle-label invariant:

```rust
fn normalize_issue_labels(issue: &GhIssue) -> Option<NormalizedIssue> {
    let lifecycle_labels: Vec<&str> = issue.labels.iter()
        .filter(|l| LIFECYCLE_LABELS.contains(&l.as_str()))
        .map(|l| l.as_str())
        .collect();

    match lifecycle_labels.len() {
        0 => None,  // No lifecycle label, not a ralph issue
        1 => Some(NormalizedIssue { number: issue.number, state: lifecycle_labels[0] }),
        _ => {
            // Multiple lifecycle labels → normalize to failed, skip
            eprintln!("warning: issue #{} has multiple lifecycle labels: {:?}; normalizing to ralph:failed",
                issue.number, lifecycle_labels);
            // Best-effort: remove all lifecycle labels, add ralph:failed
            normalize_to_failed(issue);
            None  // Skip this cycle; next cycle will see ralph:failed
        }
    }
}
```

**3.6 Paginated GitHub issue queries**

The current `poll_issues()` in `src/daemon/github.rs:72-128` uses `--limit 100` and logs a warning when exactly 100 results are returned, but does not paginate. This is insufficient for repos with many issues.

Replace with cursor-based pagination:

```rust
fn poll_issues_all(owner: &str, repo: &str, labels: &[String]) -> Result<Vec<GhIssue>> {
    let mut all_issues = Vec::new();
    let page_size = 100;
    let max_pages = 10; // Safety cap: 1000 issues max

    for page in 1..=max_pages {
        let (issues, _) = poll_issues_page(owner, repo, labels, page_size, page)?;
        let count = issues.len();
        all_issues.extend(issues);
        if count < page_size {
            break; // Last page
        }
    }

    Ok(all_issues)
}
```

Use `gh api` with cursor-based pagination instead of `gh issue list` if the `gh` CLI does not support page parameters directly:

```
gh api graphql -f query='
  query($cursor: String) {
    repository(owner: "<owner>", name: "<repo>") {
      issues(labels: ["ralph:ready"], states: OPEN, first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes { number title labels(first: 20) { nodes { name } } body }
      }
    }
  }
' --paginate
```

Both `poll_and_claim()` and `reconcile_from_labels()` must use the paginated variant. The safety cap of 10 pages (1000 issues) prevents runaway API calls on misconfigured repos.

---

### Cross-cutting: `ralph:review` tag support

The existing `ralph:review` label support (added in commit `a3c23ee`) must be preserved. Review-only issues follow a different workflow path and are unaffected by this redesign since they don't use the multi-phase orchestration loop.

---

## Files & Modules

### New files

| File | Purpose |
|------|---------|
| `src/git/ralph_commit.rs` | Ralph commit message formatter, parser, and log walker. Contains `RalphCommitInfo`, `format_ralph_commit()`, `parse_ralph_commit()`, `find_last_ralph_commit()`. Includes malformed-commit detection with `Err(MalformedCommit)` variant. |
| `src/daemon/child_map.rs` | In-memory `ChildMap` replacing `TaskStore`. Holds `HashMap<u32, RunningChild>` keyed by issue number. No file I/O. |
| `src/daemon/instance_lock.rs` | `flock`-based single-instance enforcement. Acquires exclusive lock on `/tmp/ralph-daemon-<remote_url_hash>.lock`. Uses canonical remote URL (not local path) for lock identity. |

### Modified files (Phase 1)

| File | Changes |
|------|---------|
| `src/daemon/mod.rs` | Remove `TaskStore`, `DaemonTask` struct, `tasks.json` file ops. Remove `abort_task()` file persistence; keep `abort_task()` as label-only + child-kill operation. Export `child_map` and `instance_lock` modules. |
| `src/daemon/runtime.rs` | Replace `TaskStore` parameter with `ChildMap`. Rewrite `reconcile_tasks()` → `reconcile_from_labels()` (GitHub API + git-based completion check per §1.6). Rewrite `poll_and_claim()` with post-claim verification (§1.7) and paginated queries (§3.6). Rewrite `dispatch_task()` to insert into `ChildMap`. Rewrite `collect_children()` / `complete_task()` with abort-precedence check (§1.8). Add instance lock acquisition at `run()` entry using remote-URL-based lock (§1.5). |
| `src/daemon/worktree.rs` | Add `clean_transient_files()`. Update `create_worktree()` to use `ralph/issue-<N>` branch naming. Add dirty-state precondition check that aborts dispatch with diagnostic if worktree cleanup fails (§1.9). |
| `src/daemon/github.rs` | Add `normalize_issue_labels()` function. Add `poll_issues_all()` with pagination (§3.6). Add `fetch_issue_labels()` for post-claim verify and abort-precedence check. Add `normalize_to_failed()` for multi-label cleanup. |
| `src/project/lifecycle.rs` | Remove `save_project_state()` / `load_project_state()` file I/O (keep as thin wrappers for Phase 1 transition). Remove `recover_state_from_git()`. |
| `src/project/state.rs` | Remove `save()` and `load()` methods. `ProjectState` becomes a pure in-memory struct. Remove `tempfile` dependency from this module. |
| `src/cli/status.rs` | Add `--from-git` flag. Phase 1: support both file and git-based status. |
| `src/cli/history.rs` | Add `--from-git` flag. Phase 1: support both. |
| `src/cli/rollback.rs` | Rewrite to use `git reset` to a specific ralph commit hash instead of mutating `state.json`. |
| `src/util/lock.rs` | Keep `ProjectLock` as-is for per-project orchestration locking. The new daemon-level lock lives in `src/daemon/instance_lock.rs`. |

### Modified files (Phase 2)

| File | Changes |
|------|---------|
| `src/workflow/orchestrator.rs` | Replace all `persist_state()` calls with `checkpoint_phase_transition()`. Remove `Phase::Committing` from the phase match. Remove `generate_commit_message()` and `extract_reviewer_commit_message()` (replaced by fixed ralph format). Add `checkpoint_phase_transition()` method with `--allow-empty` support (§2.3). Write phase status markers before each checkpoint. |
| `src/project/state.rs` | Remove `Phase::Committing` variant from the `Phase` enum. |
| `src/git/commit.rs` | Remove `commit_feature_loop()` (replaced by `checkpoint_phase_transition()`). Keep `stage_implementation_changes()` and diff utilities. |
| `src/git/mod.rs` | Add `pub mod ralph_commit;`. |

### Modified files (Phase 3)

| File | Changes |
|------|---------|
| `src/git/branch.rs` | Rename `ensure_project_branch()` → `sync_project_branch()`. All branch creation uses `resolve_remote_default_branch()` (§3.1) as base — never local refs. Remove `merge_base_branch()`. Add `fetch_and_reset_to_remote()`. |
| `src/workflow/orchestrator.rs` | Replace `load_project_state()` startup with `derive_position()` + `sync_project_branch()`. Build in-memory `ProjectState` from position + tree artifacts. Handle malformed last commit per §2.2 policy. |
| `src/cli/status.rs` | Remove `--from-git` flag; git-based derivation becomes the only path. |
| `src/cli/history.rs` | Rewrite to walk ralph commits in `git log` instead of reading `state.json`. |
| `src/cli/tail.rs` | Remove `state.json` reads at lines 131, 520, 655. Derive state from git log. |
| `src/cli/project.rs` | Derive project info from branch existence + prompt.md in tree. |

### Deleted files

| File | Reason |
|------|--------|
| (none deleted, but the following paths cease to be written) | |
| `.ralph/projects/*/state.json` | No longer created or read |
| `.ralph/daemon/tasks.json` | No longer created or read |

### Files intentionally NOT changed

| File | Reason |
|------|--------|
| `src/backend/*` | Backend execution is unaffected; backends still produce text output consumed by the orchestrator. |
| `src/prompts/*` | Prompt templates are unchanged. |
| `src/config/*` | Config loading is unchanged (`.ralph/ralph.toml`, project `config.toml`). |
| `src/mcp/*` | MCP server reads project state — will need minor updates to use `derive_position()` but is not critical path. |
| `src/project/artifacts.rs` | Artifact writing is unchanged; artifacts still go to `loops/<NNN-slug>/`. |
| `src/prd/*` | PRD pipeline is independent of workflow state. |
| `src/validate/*` | Conformance tests will need updates but are not production code. |

---

## Testing Strategy

### Unit tests (new, in `src/git/ralph_commit.rs`)

1. **`format_ralph_commit` produces correct subject + trailers** — verify exact string format for various phase transitions.
2. **`parse_ralph_commit` roundtrips** — format → parse → assert fields match.
3. **`parse_ralph_commit` rejects malformed messages** — missing trailers, mismatched project IDs, garbled subjects.
4. **`parse_ralph_commit` handles non-ralph commits gracefully** — returns `None` for regular commits.
5. **Subject/trailer disagreement returns `Err(MalformedCommit)`** — e.g., subject says `loop 2` but trailer says `Ralph-Loop: 3`. Verify error includes the hash and mismatch details.
6. **Malformed commit in middle of history is skipped** — `find_last_ralph_commit` returns the most recent *valid* commit, logging a warning for the malformed one.

### Unit tests (new, in `src/daemon/child_map.rs`)

7. **Insert and lookup by issue number.**
8. **Remove on child exit.**
9. **Empty map on construction.**

### Unit tests (new, in `src/daemon/instance_lock.rs`)

10. **First lock succeeds; second lock on same path returns error.**
11. **Lock released on drop allows re-acquisition.**
12. **Lock path derived from remote URL, not local path** — two different local paths with same remote URL produce the same lock path.
13. **Lock path differs for different remote URLs.**

### Unit tests (modified, in `src/daemon/github.rs`)

14. **`normalize_issue_labels` with 0, 1, 2+ lifecycle labels** — verify skip, pass-through, and normalization behavior.
15. **`filter_claimable` continues to work with `ralph:ready` trigger label and excludes `ralph:review`-only issues correctly.**
16. **`poll_issues_all` pagination** — mock returns of exactly 100 issues on first page, <100 on second, verify both pages are combined.

### Integration tests (new, in `tests/git_checkpoint.rs`)

17. **Commit+push checkpoint creates correct commit** — init a test repo with remote, run a mock phase transition, verify commit subject and trailers on the remote.
18. **Failed push leaves remote unchanged** — simulate push failure (e.g., mismatched remote), verify remote HEAD is unchanged, verify local commit exists but is discarded on next sync.
19. **`sync_project_branch` resets to remote** — create local-only commit, call sync, verify it's gone.
20. **`sync_project_branch` creates branch from remote default when missing** — verify no local refs are used.
21. **No-op phase transition produces commit via `--allow-empty`** — verify that a phase transition with zero file changes still creates a commit with correct trailers.
22. **Push succeeds but label update fails (split-brain)** — verify that on next startup reconciliation, completion is derived from the pushed commit and the label is healed.

### Integration tests (new, in `tests/derive_position.rs`)

23. **Fresh branch (no ralph commits) returns loop 1, Planning.**
24. **Branch with one ralph commit returns correct position.**
25. **Branch with multiple ralph commits returns last one.**
26. **Malformed last ralph commit triggers `ralph:failed` label and error** — verify the §2.2 policy.
27. **Malformed commit in middle of history is skipped; previous valid commit is used.**

### Integration tests (new, in `tests/branch_bootstrap.rs`)

28. **`resolve_remote_default_branch` with `origin/HEAD` set** — verify correct resolution.
29. **`resolve_remote_default_branch` with `origin/HEAD` unset but `origin/main` exists** — verify fallback.
30. **`resolve_remote_default_branch` with no default branch configured** — verify actionable error message.
31. **`resolve_remote_default_branch` with empty remote (no commits)** — verify error, not panic.

### Integration tests (new, in `tests/claim_verify.rs`)

32. **`claim_and_verify` succeeds for uncontested issue** — verify labels are `ralph:in-progress` only (no `ralph:ready` remaining).
33. **`claim_and_verify` backs off on conflict** — simulate concurrent claim (issue has unexpected labels), verify daemon releases its claim.
34. **Stale claim race** — claim an issue that was completed between poll and claim, verify daemon detects the stale state and releases.

### Integration tests (new, in `tests/abort_precedence.rs`)

35. **Manual abort during in-progress execution preserves `ralph:aborted`** — child exits after abort label applied, verify abort is not overwritten.
36. **Child exit before abort is applied uses exit-status label** — verify normal completion path is unaffected.

### Integration tests (modified, in `tests/orchestrator.rs`)

37. **Full orchestrator run produces ralph commits on remote** — mock backends, run through Planning → Implementing → Reviewing → Planning, verify commit trail.
38. **Resume after interrupt continues from last pushed commit** — run partway, simulate crash, verify next run resumes correctly.

### Integration tests (modified, in `tests/state.rs`)

39. **Remove or rewrite tests that depend on `state.json` file existence.**

### End-to-end tests (in `src/validate/`)

40. **`tests_daemon.rs`** — update to verify label-based state instead of `tasks.json`. Add test for paginated issue discovery (mock >100 issues). Add test for multi-label normalization. Add test for push/label split-brain healing.
41. **`tests_run.rs`** — update to verify commit-based checkpointing instead of `state.json` snapshots.

### Manual acceptance tests

42. **Clone-delete-reclone test:** Run ralph through 2 loops, delete clone, re-clone, verify `ralph status --from-git` shows correct position and `ralph run` resumes.
43. **Dual-daemon rejection test:** Start daemon, start second daemon for same repo (same clone or different clone), verify immediate failure.
44. **Kill-during-push test:** Start ralph run, kill -9 during push, restart, verify remote is clean and resume works.
45. **Kill-after-push-before-label test:** Start ralph run, kill -9 after push succeeds but before label update, restart, verify reconciliation heals the label from the pushed commit.
46. **Manual abort test:** Start ralph run, apply `ralph:aborted` label via GitHub UI during execution, verify child is killed and label is preserved.

---

## Out of Scope

1. **Multi-host distributed leader election** — The single-instance lock uses `flock` which is host-local. Multi-host coordination (e.g., distributed lock via GitHub API or external service) is explicitly deferred. The remote-URL-based lock key prevents two clones on the same host from conflicting, but cannot prevent daemons on different hosts from racing (requires external coordination).
2. **Backward migration of existing `state.json` projects** — Existing projects with `state.json` will not be automatically migrated to the commit-based format. Users must re-run from the beginning or manually create an initial ralph commit matching their current state. A `ralph migrate` command may be added later.
3. **Session reuse persistence** — The `SessionStore` (currently in `state.json`) will not be persisted in commits. Session reuse becomes per-invocation only. Cross-invocation session reuse requires a separate design.
4. **MCP server updates** — The MCP server (`src/mcp/`) reads project state for its tools. Updates to use `derive_position()` will be done as a follow-up.
5. **`ralph tail` live-follow mode** — The `tail` command currently reads `state.json` for live event correlation. It will need redesign to work with git-based state, but this is non-critical and deferred.
6. **PR auto-rebase redesign** — The existing rebase logic in `src/daemon/runtime.rs` is orthogonal to state management and will continue working with the `ChildMap` approach. No changes planned.
7. **Artifact path changes** — Artifacts continue to be written under `.ralph/projects/<id>/loops/<NNN-slug>/`. The file layout is unchanged; only the persistence mechanism for tracking which artifacts exist changes (from `state.json` fields to tree inspection).
8. **Config format changes** — `.ralph/ralph.toml` and per-project `config.toml` are unchanged. The `commit_message_style` config key becomes a no-op (the ralph commit format is fixed).
9. **Prompt review workflow** — The prompt review gate (`prompt_review_completed`) is currently stored in `state.json`. It will need to be tracked via a commit trailer or a sentinel file. Detailed design deferred to implementation.
10. **`ralph rollback` to arbitrary commit** — The rollback command will be simplified to `git reset --hard <commit-hash> && git push --force-with-lease`, but the UX design (e.g., interactive commit picker) is out of scope.
11. **True atomic claim (external coordination)** — The post-claim verification pattern (§1.7) provides best-effort uniqueness but is not a true compare-and-swap. GitHub's API does not support conditional label updates. True single-claim guarantee would require an external lock service or a GitHub App webhook, both of which are out of scope for v1.
12. **Automatic malformed-commit repair** — When a malformed ralph commit is detected (§2.2), the branch is labeled `ralph:failed` and requires human intervention. Automatic repair (amending the commit, rebasing, etc.) is out of scope due to the risk of history rewriting in shared branches.
