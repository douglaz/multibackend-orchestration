---
artifact: final-review-planner-positions
loop: 11
project: issue-214
backend: claude(opus)
role: planner
created_at: 2026-03-19T13:28:56Z
---

# Planner Positions

## Amendment: ORACLE-REV-FR-001

### Position
REJECT

### Rationale
The core claim — that oracle-review state is shared across repos — is factually wrong. Each repo gets its own `workspace_root` derived from a repo-specific directory.

In `src/cli/daemon.rs:147`, the repo directory is computed per slug:
```rust
let repo_dir = args.data_dir.join(&owner).join(&repo_name);
```

At line 159, the workspace is loaded from that repo-specific path:
```rust
let workspace = Workspace::load(repo_dir.join(".ralph"))?;
```

And at line 250, this per-repo root is threaded into the runtime config:
```rust
workspace_root: workspace.root.clone(),
```

`Workspace::load` (in `src/workspace/mod.rs:36-40`) stores the root as-is. So for `orgA/repoA` the workspace root is `{data_dir}/orgA/repoA/.ralph/`, and for `orgA/repoB` it is `{data_dir}/orgA/repoB/.ralph/`.

The state file (`src/daemon/oracle_review.rs:278-283`) resolves to `{workspace_root}/daemon/oracle-review-state/state.json`, which is already per-repo:
- `data_dir/orgA/repoA/.ralph/daemon/oracle-review-state/state.json`
- `data_dir/orgA/repoB/.ralph/daemon/oracle-review-state/state.json`

The two "correctness problems" described — PR number aliasing and concurrent write races — cannot occur because each repo runtime operates on its own state file in its own directory tree. The amendment mischaracterizes the implementation.

## Amendment: ORACLE-REV-FR-002

### Position
ACCEPT

### Rationale
Verified by reading the source. All `gh` subprocess calls used by the oracle-review phase lack timeouts:

- `list_open_non_draft_prs` (`src/daemon/github.rs:260`) — `.output().await` with no deadline
- `fetch_pr_diff` (`src/daemon/github.rs:282`) — `.output().await` with no deadline
- `fetch_issue_comments_with_gh_bin` (`src/daemon/github.rs:1939`) — `.output().await` with no deadline (called via `find_bot_comment_with_marker_exact_with_gh_bin` at line 2155, and again via `post_bot_comment_with_marker_outcome_with_gh_bin` at lines 2198 and 2245)
- `fetch_authenticated_login_with_gh_bin` (`src/daemon/github.rs:2018`) — `.output().await` with no deadline

Meanwhile, the `oracle` subprocess itself correctly uses `process::run_command_with_timeout` at `src/daemon/oracle_review.rs:366`. This inconsistency means a single hung `gh` call (network partition, unresponsive GitHub API) can block the entire oracle-review phase indefinitely, which in turn stalls the daemon poll loop for that repo. The phase processes PRs sequentially (the `for pr in candidates` loop at line 150), so one hung call prevents all subsequent PRs from being processed.

This is a genuine robustness gap. The per-PR error handling (the `continue` on `Err` paths) is well-designed but cannot trigger if the subprocess never returns.
