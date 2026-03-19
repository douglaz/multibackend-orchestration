---
artifact: final-review-votes
loop: 11
project: issue-214
backend: codex
role: final_reviewer
created_at: 2026-03-19T13:30:05Z
---

# Vote Results

## Amendment: ORACLE-REV-FR-001

### Vote
REJECT

### Rationale
The planner is correct. The amendment’s premise does not match the implementation.

`daemon start` builds a repo-specific `repo_dir` as `data_dir/<owner>/<repo>`, then loads the workspace from `repo_dir/.ralph`, and passes that repo-local path as `workspace_root`. `OracleReviewState::load/save` resolve their files under `workspace_root/daemon/oracle-review-state/`, so each monitored repo already gets its own `state.json` and temp files.

Because the state path is already repo-scoped, the two claimed failures do not follow:
- `repoA#11` and `repoB#11` do not share the same persisted map.
- Concurrent runtimes do not race on a shared oracle-review state file across repos.

A test for cross-repo isolation would be harmless, but the proposed code change is not justified by the current implementation.

## Amendment: ORACLE-REV-FR-002

### Vote
ACCEPT

### Rationale
The planner’s reasoning is sound. The oracle subprocess is timeout-bounded, but the GitHub CLI calls used by the same phase are not.

`list_open_non_draft_prs`, `fetch_pr_diff`, `fetch_issue_comments_with_gh_bin`, and `fetch_authenticated_login_with_gh_bin` all await `Command::output()` with no deadline. The oracle-review loop processes PRs sequentially, so a hung `gh` call can block the whole phase indefinitely and prevent later PRs from being reviewed. That defeats the intended non-fatal, per-PR-isolated behavior.

Adding bounded execution and treating timeouts as logged per-PR warnings is the right fix. I would apply that consistently to all `gh` calls in this phase, including comment post/readback paths, not just diff/comment fetch.
