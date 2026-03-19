---
artifact: final-review-votes
loop: 11
project: issue-214
backend: claude
role: final_reviewer
created_at: 2026-03-19T13:29:31Z
---

I've verified the source code. Here are my votes:

# Vote Results

## Amendment: ORACLE-REV-FR-001

### Vote
REJECT

### Rationale
The planner's position is correct. I verified that `workspace_root` is already scoped per repository. In `src/cli/daemon.rs:147`, `repo_dir` is computed as `args.data_dir.join(&owner).join(&repo_name)`, and the workspace is loaded from that repo-specific path at line 159. This per-repo `workspace_root` is threaded into `DaemonRuntimeConfig` at line 250.

The state file resolves via `oracle_review_state_dir` (`src/daemon/oracle_review.rs:278-283`) to `{workspace_root}/daemon/oracle-review-state/state.json`. Since each repo gets its own `workspace_root`, the state files are already fully isolated — e.g., `data_dir/orgA/repoA/.ralph/daemon/oracle-review-state/state.json` vs `data_dir/orgA/repoB/.ralph/daemon/oracle-review-state/state.json`.

The two "correctness problems" (PR number aliasing and concurrent write races) cannot occur because each repo runtime operates on its own state file in its own directory tree. The amendment is based on a misreading of the architecture.

## Amendment: ORACLE-REV-FR-002

### Vote
ACCEPT

### Rationale
I confirmed the planner's analysis by reading the source. Every `gh` subprocess call used by the oracle-review phase uses bare `.output().await` with no timeout:

- `list_open_non_draft_prs` at `src/daemon/github.rs:260`
- `fetch_pr_diff` at `src/daemon/github.rs:283`
- `fetch_issue_comments_with_gh_bin` at `src/daemon/github.rs:1949`
- `fetch_authenticated_login_with_gh_bin` at `src/daemon/github.rs:2018`

Meanwhile, the `oracle` subprocess itself correctly uses `process::run_command_with_timeout` at `src/daemon/oracle_review.rs:366`. This is an inconsistency: the most expensive subprocess is timeout-bounded, but the supporting `gh` calls that surround it are not.

The sequential PR processing loop at line 150 means a single hung `gh` call blocks all subsequent PRs and stalls the daemon poll loop for that repo. The existing per-PR error handling (`continue` on `Err`) is well-designed but useless if the subprocess never returns. This is a genuine robustness gap that should be addressed.
