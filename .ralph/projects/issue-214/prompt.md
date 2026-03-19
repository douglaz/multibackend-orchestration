## Summary

Add a new daemon phase, `oracle_review_phase`, that automatically reviews open, non-draft pull requests in the monitored GitHub repository using the `oracle` CLI (`@steipete/oracle`) and posts the result as an idempotent top-level PR comment.

This phase is separate from the existing `pr_review_phase`. It must not share state, reuse that phase's amendment flow, or change that phase's behavior.

A PR should only be reviewed once per `(pr_number, head_sha)`. If new commits are pushed and the PR head SHA changes, the daemon should post a fresh oracle review.

## Required Behavior

### Poll-loop integration

Add `oracle_review_phase` to the daemon poll loop in `src/daemon/runtime.rs`, immediately after `pr_review_phase` and before later phases already in the loop.

The daemon must treat this phase as non-fatal:
- If the phase returns an error, log a warning and continue the poll loop.
- Failures reviewing one PR must not prevent other eligible PRs from being processed in the same cycle.

### Enablement and defaults

The phase is disabled by default.

It only runs when `daemon_oracle_review_enabled = true`.

Add these workspace-level config fields and defaults:
- `daemon_oracle_review_enabled: bool = false`
- `daemon_oracle_review_timeout_secs: u64 = 900`
- `daemon_oracle_review_authors: Vec<String> = []`
- `daemon_oracle_review_max_per_cycle: u32 = 3`

Validation rules:
- `daemon_oracle_review_timeout_secs` must be `> 0`
- `daemon_oracle_review_max_per_cycle` must be `> 0`

These fields must be wired through:
- `WorkspaceConfig`
- `EffectiveDaemonConfig`
- `DaemonRuntimeConfig`
- `set_global_config_value`
- `config get`
- `config show`

Use existing naming and parsing conventions for daemon config fields.

### PR selection

Add a GitHub helper that lists open PRs using:

`gh pr list --repo {owner}/{repo} --state open --json number,headRefOid,isDraft,author --limit 100`

Required behavior:
- Parse the response into a pure `parse_open_prs(json: &str) -> Result<Vec<OpenPrInfo>>` helper for unit testability.
- Filter out draft PRs during parsing. Draft PRs must never reach the phase logic.
- Return an overflow flag when exactly 100 PRs are returned, matching the existing overflow pattern used elsewhere in daemon GitHub polling.

Use this data structure:

```rust
pub struct OpenPrInfo {
    pub number: u32,
    pub head_sha: String,
    pub author: String,
}
```

If `daemon_oracle_review_authors` is empty, all non-draft PRs are eligible.

If it is non-empty, only review PRs whose author login matches an allowlisted value case-insensitively.

### Deduplication and persisted state

Persist dedup state at:

`{workspace_root}/daemon/oracle-review-state/state.json`

Use this JSON-backed structure:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OracleReviewState {
    pub reviewed: HashMap<String, String>,
}
```

Semantics:
- Key: PR number as a string
- Value: last successfully reviewed head SHA

Required load/save behavior:
- `load()` returns default state when the file does not exist
- `load()` returns an error on corrupt JSON; do not silently reset state
- `save()` must use atomic write semantics via temp file plus rename
- `save()` must create parent directories as needed

Update state only after a review comment is successfully posted.

Persist state immediately after each successful PR review, not once at end of cycle. This avoids losing progress if the daemon exits mid-cycle.

### Comment idempotency

Each oracle review comment must start with this marker:

`<!-- ralph:oracle-review:{pr_number}:{head_sha} -->`

Use `post_bot_comment_with_marker_with_gh_bin` with:
- configured `gh_bin`
- resolved `bot_login`
- the marker above

Required behavior:
- Only bot-authored comments count for marker deduplication
- User-authored spoofed markers must be ignored
- If a bot-authored comment with the exact marker already exists, skip the PR without invoking `oracle`
- This marker is per `(pr_number, head_sha)`, so a new SHA produces a new marker and allows a fresh review

### Oracle invocation

Implement the phase in a new module:

`src/daemon/oracle_review.rs`

Invoke `oracle` via `process::run_command_with_timeout` from `src/daemon/process.rs`, so the new phase inherits the existing process-group, output-draining, and timeout-kill behavior.

Use `spawn_blocking` around the synchronous process helper.

Input contract:
- Fetch the PR diff with `gh pr diff {pr_number} --repo {owner}/{repo}`
- Write the diff to a temporary file under `{workspace_root}/daemon/oracle-review-state/`
- Pass that file path to `oracle` as an argument
- Remove the temp file after the command completes or fails

Do not use stdin piping for this feature.

Use this system prompt exactly:

```text
You are a senior code reviewer. Review this PR diff for bugs, security issues, performance problems, and code quality. Be concise and actionable. Focus on substantive issues, not style nits.
```

Command requirements:
- Execute `oracle` through `process::run_command_with_timeout`
- Respect `daemon_oracle_review_timeout_secs`
- Treat timeout, non-zero exit, or spawn failure as a warning for that PR
- On oracle failure, do not update state
- On oracle failure, continue processing other eligible PRs until the per-cycle cap is reached or candidates are exhausted

### Comment formatting

The review comment body must be:

1. The marker line
2. A newline
3. The oracle review text

Before posting, truncate the review body with the existing GitHub comment truncation helper so the final posted comment stays within `GITHUB_COMMENT_LIMIT` (`65_536` chars).

Refactor existing helpers in `src/daemon/runtime.rs` to be reusable from `oracle_review.rs`:
- `truncate_for_github` -> `pub(crate)`
- `GITHUB_COMMENT_LIMIT` -> `pub(crate)`

Available body budget must account for:
- marker length
- one newline separator
- review text

### Per-cycle cap

Review at most `daemon_oracle_review_max_per_cycle` PRs per poll cycle.

Only increment the per-cycle count after a successful review comment is posted.

PRs skipped because they are draft, filtered by author, already deduped, or already have the exact marker do not count against the cap.

### Logging

Use warning logs for operational failures and keep the poll loop alive.

This warning string must match exactly for PR list overflow:

`warning: oracle review: gh pr list returned 100 PRs, results may be truncated`

For other failures, log warnings that clearly identify:
- the phase (`oracle review`)
- the PR number when applicable
- the failure type (`diff fetch`, `oracle timeout`, `oracle exit`, `oracle spawn`, `comment post`, `state load`, `state save`)

Exact wording for non-overflow warnings does not need to be fixed, but tests should be able to match stable substrings.

## Implementation Requirements

### Files to change

Required file/module changes:

- `src/daemon/mod.rs`
  - Export `pub mod oracle_review;`

- `src/daemon/oracle_review.rs`
  - New file
  - Implement `OracleReviewState`
  - Implement state load/save helpers
  - Implement `oracle_review_phase`

- `src/daemon/github.rs`
  - Add `OpenPrInfo`
  - Add `parse_open_prs`
  - Add `list_open_non_draft_prs`
  - Add `fetch_pr_diff`

- `src/config/global.rs`
  - Add the 4 `daemon_oracle_review_*` workspace config fields
  - Add default functions
  - Add `set_global_config_value` match arms with bounds validation

- `src/config/mod.rs`
  - Add oracle review fields to `EffectiveDaemonConfig`
  - Wire them in `resolve_daemon_config`

- `src/cli/config.rs`
  - Expose oracle review fields in `config get` and `config show`

- `src/cli/daemon.rs`
  - Populate `DaemonRuntimeConfig` with the new fields

- `src/daemon/runtime.rs`
  - Add fields to `DaemonRuntimeConfig`
  - Expose truncation helpers as `pub(crate)`
  - Call `oracle_review_phase` after `pr_review_phase`

- `src/validate/tests_daemon_oracle_review.rs`
  - New conformance test module

- `src/validate/mod.rs`
  - Register the new validate module

### Runtime algorithm

Implement `oracle_review_phase(config: &DaemonRuntimeConfig) -> Result<()>` with this order:

1. Return immediately if `!config.oracle_review_enabled`
2. List open non-draft PRs
3. If list size is exactly 100, emit the overflow warning
4. Apply author allowlist filtering
5. Load `OracleReviewState`
6. Resolve `bot_login` once for the whole phase
7. Iterate candidate PRs in listed order
8. For each PR:
   - Skip if state already records the same `head_sha`
   - Build the marker for `(pr_number, head_sha)`
   - Check for an existing bot-authored marker comment first; if found, update state to that `head_sha`, save state, and skip oracle invocation
   - Fetch diff
   - Invoke oracle
   - Truncate body
   - Post comment with `post_bot_comment_with_marker_with_gh_bin`
   - Update state to `head_sha`
   - Save state immediately
   - Increment success count
   - Stop once success count reaches `daemon_oracle_review_max_per_cycle`

This ordering is required so that existing bot comments prevent unnecessary oracle runs and so state can self-heal if a prior successful comment exists but the state file is missing or stale.

## Testing Requirements

Add validate conformance tests in `src/validate/tests_daemon_oracle_review.rs`. These tests are required, not optional.

Required conformance coverage:
- config defaults
- config set/get roundtrips for all 4 new fields
- bounds rejection for timeout `0`
- bounds rejection for max-per-cycle `0`
- `config show` includes the new daemon fields
- disabled phase is a no-op
- new eligible PR gets reviewed and commented
- draft PRs are skipped
- already-reviewed SHA is skipped
- SHA change triggers re-review
- author allowlist is enforced
- author allowlist matching is case-insensitive
- per-cycle max is enforced
- existing bot marker skips oracle invocation
- oracle timeout does not advance state
- oracle non-zero exit does not advance state
- missing oracle binary does not advance state and does not abort the cycle
- comment-post failure does not advance state
- 100-PR overflow logs the required warning

Add focused unit tests where appropriate for:
- `parse_open_prs`
- state load/save behavior
- corrupt state handling
- dedup behavior for same SHA vs changed SHA

## Constraints

- This phase must remain independent of `pr_review_phase`
- Do not add project-level overrides for these settings
- Do not add inline review comments or GitHub review API usage
- Do not paginate beyond the `gh pr list --limit 100` call
- Do not split oversized comments into multiple GitHub comments
- Do not retry failed oracle executions within the same cycle
- Do not add oracle model-selection config in this change

## Acceptance Criteria

The work is complete when all of the following are true:

- `oracle_review_phase` exists and is called from the daemon poll loop after `pr_review_phase`
- The phase is disabled by default and returns immediately unless enabled
- Draft PRs are excluded before phase-level processing
- Author allowlist behavior matches the spec above
- Dedup state persists at `{workspace_root}/daemon/oracle-review-state/state.json`
- Reviewing is keyed by `(pr_number, head_sha)`
- A changed `head_sha` causes a fresh review
- Existing bot-authored marker comments skip oracle invocation and reconcile state
- Oracle is invoked only through `process::run_command_with_timeout`
- Oracle timeout uses `daemon_oracle_review_timeout_secs`
- Review comments use the exact marker format specified above
- Comments are posted through `post_bot_comment_with_marker_with_gh_bin`
- Comment bodies are truncated with the shared GitHub helper before posting
- No more than `daemon_oracle_review_max_per_cycle` successful reviews are posted per cycle
- 100-PR overflow is detected and logged with the exact warning string
- Config wiring is complete across config parsing, resolution, and CLI surfaces
- Validate conformance tests are added and cover the required scenarios