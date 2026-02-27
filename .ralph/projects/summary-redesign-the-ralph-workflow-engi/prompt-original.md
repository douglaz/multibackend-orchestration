I'll start by exploring the codebase to understand the current architecture, state management, and workflow engine before writing the spec.Now let me look at a few specific files in more detail to understand the exact code patterns I need to reference in the spec.Now let me look at a few more critical files — the project state structure, git commit module, and the GitHub module.Now let me check the orchestrator commit section and any existing worktree module to get the last pieces.Now let me look at the persist_state calls in the orchestrator to understand the save frequency.Now I have comprehensive understanding of the codebase. Let me look at the quick-prd directory that was mentioned in git status to understand the full scope.I now have a thorough understanding of the entire codebase. Let me write the engineering specification.

---

## Summary

Eliminate all durable local state files (`state.json`, `tasks.json`, event/agent output logs) from the Ralph workflow engine. Replace them with two external sources of truth: **Git commits with structured messages and trailers** for workflow position, and **GitHub issue labels** for task lifecycle state. The daemon's in-process data structure shrinks to a `HashMap<u32, ChildHandle>` mapping issue numbers to live child PIDs/PGIDs. On every startup, the daemon reconstructs its world entirely from `git log` on remote project branches and `gh issue list` label queries — zero local files required.

This redesign targets three failure modes in the current architecture: (1) state.json corruption or drift causing unrecoverable orchestrator errors (`src/project/lifecycle.rs:167-226` recovery-from-git is already a workaround); (2) tasks.json desync when daemon crashes between label update and file write (`src/daemon/runtime.rs:1074-1150`); and (3) the inability to resume from a fresh clone because `.ralph/projects/*/state.json` is gitignored and lives only in worktrees.

## Acceptance Criteria

1. **Fresh-clone resumability**: Delete the entire local clone, re-clone, and `ralph daemon start` resumes all in-progress issues from their last pushed commit — no manual intervention.
2. **Commit-derived position**: Workflow position (loop N, phase P) is fully derivable by parsing the last ralph commit on `origin/ralph/issue-<N>` using subject line + trailers. No `state.json` consulted.
3. **Label-derived task state**: Active/pending/completed tasks are fully derivable from `gh issue list` with `ralph:*` label filters. No `tasks.json` consulted.
4. **Crash-before-commit safety**: A crash before `git commit` leaves the remote branch unchanged. Next startup discards unpushed local state and resumes from the last pushed commit.
5. **Crash-after-commit-before-push safety**: A crash after `git commit` but before `git push` discards the local commit on next startup (`git reset --hard origin/<branch>`). Resumes from last pushed step.
6. **Zero persisted state files**: The implementation contains no `state.json`, `tasks.json`, or append-only log writes. `.ralph/tmp/` is the only local directory used, and it is cleaned on startup.
7. **Single-daemon enforcement**: A second `ralph daemon start` for the same repo fails immediately with a clear error, enforced via `flock` on `/tmp/ralph-daemon-<repo-hash>.lock`.
8. **Atomic step commits**: Each phase transition produces exactly one commit with subject `ralph(<project-id>): loop <N> <phase> -> <next-phase>` and trailers `Ralph-Project`, `Ralph-Loop`, `Ralph-Phase`, followed by a successful `git push`.
9. **No-branch-from-local-ref invariant**: All branch creation uses `origin/HEAD` or `origin/<ref>` — never a local-only ref.
10. **Startup with no prior commits**: A new issue with no ralph commits on its project branch starts at loop 1, phase planning.
11. **Multi-label normalization**: An issue with more than one `ralph:` lifecycle label is normalized to `ralph:failed` and skipped during polling.
12. **Orphan child cleanup**: On startup, any `ralph:in-progress` issue with no running child process is reset to `ralph:ready`.

## Technical Approach

### Phase 1 — State Elimination

**Remove `state.json` persistence from orchestrator** (`src/project/lifecycle.rs`, `src/project/state.rs`, `src/workflow/orchestrator.rs`):
- Delete `save_project_state()`, `load_project_state()`, and the `recover_state_from_git()` recovery path.
- Delete `ProjectState::save()` and `ProjectState::load()` methods.
- The `ProjectState` struct remains as an **in-memory-only** working object, constructed from commit parsing at startup and maintained in RAM during a run.
- Remove all 7 `persist_state()` calls in `orchestrator.rs`. State is instead materialized into the working tree as artifacts and committed atomically at phase boundaries.
- Remove `state.json` from `discover_project_ids()` and `discover_latest_project_id()` in `src/daemon/runtime.rs:482-536`.

**Replace `TaskStore` with in-memory child map** (`src/daemon/mod.rs`, `src/daemon/runtime.rs`):
- Delete `TaskStore`, `DaemonTask`, `tasks.json` file operations, and `fs2` file locking.
- Replace `children: HashMap<String, ActiveChild>` with `children: HashMap<u32, ChildHandle>` where the key is the GitHub issue number and `ChildHandle` holds `pid: u32`, `pgid: u32`, `child: tokio::process::Child`, `log_file: PathBuf`, and `branch: String`.
- Task metadata (owner, repo, issue title/body, refined title) is fetched from GitHub on demand and held transiently — never persisted.

**Clean transient files on start**:
- Add startup step: `rm -rf .ralph/tmp/` then `mkdir -p .ralph/tmp/`.
- Agent output logs (`src/output_log.rs`) redirect to `.ralph/tmp/logs/` instead of `.ralph/projects/*/`. These are ephemeral debugging aids, not durable state.

### Phase 2 — Commit+Push Checkpointing

**Commit message parser** (new module: `src/git/ralph_commit.rs`):
```rust
pub struct RalphCommit {
    pub project_id: String,
    pub loop_number: u32,
    pub from_phase: String,
    pub to_phase: String,  // = current position
}

/// Parse the most recent ralph commit on a branch.
/// Runs: git log --format='%s%n%b' origin/<branch> --grep='ralph(' -1
/// Extracts subject line match + trailer key-value pairs.
pub fn parse_last_ralph_commit(workdir: &Path, branch: &str) -> Result<Option<RalphCommit>>;

/// Build a structured commit message with trailers.
pub fn build_ralph_commit_message(
    project_id: &str, loop_number: u32,
    from_phase: &str, to_phase: &str,
) -> String;
```

Subject format: `ralph(issue-42): loop 3 implementing -> reviewing`

Trailers appended to commit body:
```
Ralph-Project: issue-42
Ralph-Loop: 3
Ralph-Phase: reviewing
```

**Phase transition commit protocol** — replaces the current `commit_feature_loop()` in `src/git/commit.rs:99-134`:

1. All artifact writes happen during phase execution (already the case for spec/impl_notes/reviews).
2. At the boundary, call `git add -A` (current behavior).
3. Commit with `build_ralph_commit_message()`.
4. `git push origin <project-branch>`. If push fails, the phase is **not** advanced. The orchestrator retries from the beginning of the current phase on next invocation.
5. Remove the git tagging step (`git tag ralph/{project_id}/loop-{N}` at `orchestrator.rs:1542-1546`) — tags are replaced by parseable commit messages.

**Artifact materialization**: The current `FeatureLoopArtifacts` fields (spec, impl_notes, reviews, approval, qa_results) are already written as files under `.ralph/projects/<id>/loops/<NNN-slug>/`. These remain as working-tree artifacts committed atomically. The difference: they are committed with ralph trailers instead of free-form messages, and `state.json` is no longer among the committed files.

### Phase 3 — Strict Startup Sync

**Fetch-first branch resolution** (replaces `maybe_create_project_branch()` in `src/project/lifecycle.rs:85-118` and `create_worktree()` in `src/daemon/worktree.rs:24-118`):

```
fn sync_project_branch(workdir: &Path, project_id: &str) -> Result<BranchState>:
    1. git fetch origin
    2. branch_name = ralph/issue-<issue_number>
    3. if origin/<branch_name> exists:
        git checkout -B <branch_name> origin/<branch_name>
        // This discards any local-only commits
        return BranchState::Existing
    4. else:
        git checkout -b <branch_name> origin/HEAD
        return BranchState::New
```

**Position derivation on startup**:
```
fn derive_position(workdir: &Path, branch: &str) -> Result<WorkflowPosition>:
    match parse_last_ralph_commit(workdir, branch):
        Some(commit) =>
            WorkflowPosition { loop: commit.loop_number, phase: commit.to_phase }
        None =>
            WorkflowPosition { loop: 1, phase: "planning" }
```

**Reconstruct in-memory `ProjectState`**: After position derivation, rebuild the minimal `ProjectState` from:
- `loop_number` and `phase` from the last commit.
- Artifact existence checks on the working tree (e.g., if `loops/001-*/spec.md` exists, the planning phase for loop 1 is complete).
- `phase_iteration` derived from counting review/QA exchange files in the current loop directory.

**Daemon startup reconciliation** (replaces `reconcile_tasks()` in `src/daemon/runtime.rs:269-292`):
1. Query `gh issue list --label ralph:in-progress` for the repo.
2. For each issue: check if a child process is running (by checking the in-memory `children` map, which is empty on fresh start).
3. If no child running: atomically swap label `ralph:in-progress` → `ralph:ready` (issue is re-claimable).
4. No file reads — purely GitHub API driven.

**Single-daemon lock** (replaces the current `fs2`-based `tasks.json` lock):
- On startup: `flock --nonblock /tmp/ralph-daemon-<sha256(repo_root)>.lock`.
- If lock fails: exit with error "another daemon instance is running for this repository".
- Lock is held for daemon lifetime and auto-released on process exit.

### Project Branch Convention

Current code uses `ralph/{project_id}` as the branch format (`src/git/branch.rs` `resolve_branch_name`). The new convention changes to:
- Branch: `ralph/issue-<issue_number>`
- Project ID: `issue-<issue_number>`
- Deterministic from issue number alone — no slug generation, no name collision risk.

This replaces the current `ralph/daemon/<task_id>` branch in `src/daemon/worktree.rs:26` and the configurable `branch_format` in workspace config.

## Files & Modules

| File | Action | Description |
|------|--------|-------------|
| `src/project/state.rs` | **Modify** | Remove `save()`, `load()`. Keep struct as in-memory-only. Remove `SessionStore` persistence (sessions are ephemeral per run). |
| `src/project/lifecycle.rs` | **Modify** | Remove `load_project_state()`, `save_project_state()`, `recover_state_from_git()`. Add `derive_state_from_commits()` and `derive_state_from_artifacts()`. |
| `src/workflow/orchestrator.rs` | **Modify** | Remove all `persist_state()` calls (~7 sites). Replace `commit_feature_loop()` call at line 1548 with `commit_and_push_phase_transition()`. Add fetch+sync preamble before main loop. |
| `src/git/ralph_commit.rs` | **Create** | Commit message builder/parser: `parse_last_ralph_commit()`, `build_ralph_commit_message()`, trailer validation. |
| `src/git/commit.rs` | **Modify** | Remove `commit_feature_loop()` tagging logic. Add `commit_and_push_phase_transition()` that builds structured message, commits, and pushes. |
| `src/git/branch.rs` | **Modify** | Add `sync_project_branch()` with fetch-first, origin-only semantics. Remove local-ref branch creation paths. |
| `src/daemon/mod.rs` | **Modify** | Remove `TaskStore`, `DaemonTask`, `read_tasks_from_file()`, `write_tasks_to_file()`, `abort_task()` file operations. Keep `abort_task()` as label-only operation. |
| `src/daemon/runtime.rs` | **Modify** | Replace `TaskStore`-based `reconcile_tasks()` with label-based reconciliation. Replace `children: HashMap<String, ActiveChild>` with `HashMap<u32, ChildHandle>`. Remove `discover_project_ids()`, `discover_latest_project_id()`. |
| `src/daemon/worktree.rs` | **Modify** | Update `create_worktree()` to use `origin/HEAD` base ref exclusively. Remove `sync_remote_master()` (subsumed by fetch-first protocol). |
| `src/daemon/github.rs` | **Modify** | Add `normalize_lifecycle_labels()` to detect and fix multi-label issues. Add `swap_label()` for atomic label transitions. |
| `src/output_log.rs` | **Modify** | Change log directory from `.ralph/projects/*/` to `.ralph/tmp/logs/`. |
| `src/cli/status.rs` | **Modify** | Derive status display from last ralph commit + GitHub labels instead of `state.json`. |
| `src/cli/history.rs` | **Modify** | Derive history from `git log` with trailer parsing instead of `state.json` loop arrays. |
| `src/util/lock.rs` | **Modify** | Add `DaemonLock` using `flock` on `/tmp/ralph-daemon-<hash>.lock` for single-instance enforcement. |

## Testing Strategy

**Unit tests** (in-process, no git/GitHub):
- `ralph_commit.rs`: Round-trip `build_ralph_commit_message()` → `parse_last_ralph_commit()` for all phase combinations. Malformed subject/trailer detection. Missing trailer detection. Subject-trailer disagreement detection.
- `github.rs`: `normalize_lifecycle_labels()` with 0, 1, 2+ lifecycle labels. `filter_claimable()` with `ralph:review` (non-lifecycle) labels present.
- In-memory `ProjectState` reconstruction from mock commit data + mock artifact directory layouts.

**Integration tests** (local git repos, no GitHub — extend existing `src/validate/` harness):
- `test_commit_and_push_phase_transition`: Init bare repo + clone, run one phase transition, verify commit message format and trailers on remote.
- `test_startup_sync_existing_branch`: Push ralph commits to a bare remote, clone fresh, verify `sync_project_branch()` checks out the correct branch and `derive_position()` returns the right loop/phase.
- `test_startup_sync_new_branch`: Verify `sync_project_branch()` creates from `origin/HEAD` when no project branch exists.
- `test_crash_before_push_recovery`: Commit locally (no push), simulate restart, verify local commit is discarded and position matches last pushed commit.
- `test_single_daemon_lock`: Acquire lock, attempt second acquisition, verify failure.
- `test_transient_cleanup`: Create files in `.ralph/tmp/`, run startup, verify deletion.

**Daemon-level tests** (mock GitHub via `gh` replacement script — extend `src/validate/tests_daemon.rs`):
- `test_poll_claim_dispatch`: Mock `ralph:ready` issue, verify label swap to `ralph:in-progress`, child spawn.
- `test_reconcile_on_restart`: Mock `ralph:in-progress` issue with no running child, verify label reset to `ralph:ready`.
- `test_multi_label_normalization`: Mock issue with both `ralph:ready` and `ralph:completed`, verify it is skipped and normalized to `ralph:failed`.
- `test_child_exit_label_update`: Verify exit 0 → `ralph:completed`, exit 1 → `ralph:failed`.

**Manual smoke test**:
- Run full daemon cycle on a real repo: create issue with `ralph:ready`, observe claim → work → commit → push → PR → `ralph:completed`. Delete clone, re-clone, restart daemon, verify no-op (issue already completed).

## Out of Scope

- **Multi-host distributed locking / leader election**: v1 is single-host only. The `flock`-based lock is per-machine. Distributed coordination (e.g., via GitHub Deployments or Redis) is a separate effort.
- **Artifact schema migration**: Existing `.ralph/projects/*/loops/` artifact file layouts are preserved as-is. No changes to spec/impl_notes/review file formats.
- **Session reuse persistence across restarts**: `SessionStore` becomes ephemeral (in-memory per run). Cross-restart session reuse requires backend support for session discovery, which is a separate feature.
- **Backward compatibility with existing `state.json`**: Projects created before this change cannot be resumed after it lands. A one-time migration is not provided — users should complete or abandon in-flight projects first.
- **PR auto-rebase refactoring**: The `auto_rebase_phase()` in `src/daemon/runtime.rs:1339-1593` is unchanged except for removing its `TaskStore` dependency (it will use the in-memory child map and GitHub labels).
- **Prompt review workflow changes**: The prompt review gate (`prompt_review_completed` flag and associated logic in `orchestrator.rs:214-231`) is preserved as an in-memory check, not redesigned.
- **Agent output log durability**: Logs move to `.ralph/tmp/` and become ephemeral. Durable agent output logging (e.g., to GitHub issue comments or S3) is out of scope.
- **Configurable commit message style**: The three current styles (`Conventional`, `Descriptive`, `Minimal`) are replaced by the single structured format. Custom commit messages from reviewer approval (`extract_reviewer_commit_message`) are removed — the ralph trailer format is mandatory.