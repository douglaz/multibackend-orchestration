## Summary

Add a `daemon_oracle_review_phase` to the daemon poll loop that automatically reviews open, non-draft PRs on the monitored repository using the `oracle` CLI (`@steipete/oracle`) and posts the review as an idempotent PR comment. The phase is independent of the existing `pr_review_phase` (which handles human review comments → amendment dispatch). Deduplication is keyed on `(pr_number, head_sha)` so re-reviews happen only when new commits are pushed.

## Acceptance Criteria

- New phase function `oracle_review_phase` added to the daemon poll loop in `runtime.rs`, called after `pr_review_phase`
- Phase is skipped when `daemon_oracle_review_enabled` is `false` (default)
- Draft PRs are never reviewed (`isDraft` filtered at parse time)
- Dedup state persists across poll cycles via a JSON file keyed on `pr_number → head_sha`, stored under `{workspace_root}/daemon/oracle-review-state/state.json`
- When a PR's `head_sha` changes, a fresh oracle review is posted
- Oracle is invoked via `process::run_command_with_timeout` (from `src/daemon/process.rs`) with the PR diff written to a temp file and passed as an argument, reusing the existing process-group, output-draining, and timeout-kill semantics
- Oracle invocation respects `daemon_oracle_review_timeout_secs` (default: 900)
- Review comment includes the idempotent HTML marker `<!-- ralph:oracle-review:{pr_number}:{head_sha} -->` and skips posting if the marker already exists on a bot-authored comment
- Comments are posted via `post_bot_comment_with_marker_with_gh_bin`, using the configured `gh_bin` and resolved `bot_login`, to prevent user-authored marker spoofing
- Review comment body is truncated to fit within `GITHUB_COMMENT_LIMIT` (65 536 chars) using the existing `truncate_for_github` helper before posting
- At most `daemon_oracle_review_max_per_cycle` (default: 3) oracle invocations per poll cycle
- PR list overflow is detected and logged when exactly 100 PRs are returned from `gh pr list --limit 100`, matching the `poll_issues_with_gh_bin` overflow pattern
- Author allowlist (`daemon_oracle_review_authors`) is respected: empty = review all non-draft PRs, non-empty = only review PRs by listed authors (case-insensitive)
- Phase runs independently from `pr_review_phase` — no shared state or coupling
- New config fields are wired through `WorkspaceConfig` → `EffectiveDaemonConfig` → `DaemonRuntimeConfig`, including `set_global_config_value` match arms, `config get`/`config show` output in `src/cli/config.rs`, and type-appropriate bounds parsing
- Validate conformance tests cover all acceptance criteria under `src/validate/tests_daemon_oracle_review.rs`

## Technical Approach

### 1. Configuration

**`src/config/global.rs` — `WorkspaceConfig` fields (~line 94)**

Add four fields following the existing `daemon_*` naming convention:

```rust
#[serde(default)]
pub daemon_oracle_review_enabled: bool,                  // default: false
#[serde(default = "default_daemon_oracle_review_timeout_secs")]
pub daemon_oracle_review_timeout_secs: u64,              // default: 900
#[serde(default)]
pub daemon_oracle_review_authors: Vec<String>,           // default: []
#[serde(default = "default_daemon_oracle_review_max_per_cycle")]
pub daemon_oracle_review_max_per_cycle: u32,             // default: 3
```

Add corresponding default functions in the defaults section (~line 880):

```rust
fn default_daemon_oracle_review_timeout_secs() -> u64 { 900 }
fn default_daemon_oracle_review_max_per_cycle() -> u32 { 3 }
```

**`src/config/global.rs` — `set_global_config_value` match arms (~line 1358)**

Add four match arms following the existing pattern used by `daemon_auto_rebase_enabled`, `daemon_rebase_timeout_seconds`, `daemon_pr_review_whitelist`, etc.:

```rust
"workspace.daemon_oracle_review_enabled" => {
    config.workspace.daemon_oracle_review_enabled = cfg_parse_bool(value)?;
}
"workspace.daemon_oracle_review_timeout_secs" => {
    let v = cfg_parse_u64(value)?;
    if v == 0 { return Err(anyhow!("daemon_oracle_review_timeout_secs must be > 0")); }
    config.workspace.daemon_oracle_review_timeout_secs = v;
}
"workspace.daemon_oracle_review_authors" => {
    config.workspace.daemon_oracle_review_authors = cfg_parse_string_list(value)?;
}
"workspace.daemon_oracle_review_max_per_cycle" => {
    let v = cfg_parse_u32(value)?;
    if v == 0 { return Err(anyhow!("daemon_oracle_review_max_per_cycle must be > 0")); }
    config.workspace.daemon_oracle_review_max_per_cycle = v;
}
```

Bounds validation: timeout must be > 0, max_per_cycle must be > 0. Uses existing `cfg_parse_bool`, `cfg_parse_u64`, `cfg_parse_u32`, and `cfg_parse_string_list` helpers already defined in `global.rs:1671+`.

**`src/config/mod.rs` — `EffectiveDaemonConfig` (~line 106)**

Add four fields:

```rust
pub oracle_review_enabled: bool,
pub oracle_review_timeout_secs: u64,
pub oracle_review_authors: Vec<String>,
pub oracle_review_max_per_cycle: u32,
```

**`src/config/mod.rs` — `resolve_daemon_config` (~line 479)**

Wire the four fields directly from `global.workspace.*` (no project-level override initially, matching the `pr_review_whitelist` / `max_backend_retries` pattern for non-overridable fields):

```rust
oracle_review_enabled: global.workspace.daemon_oracle_review_enabled,
oracle_review_timeout_secs: global.workspace.daemon_oracle_review_timeout_secs,
oracle_review_authors: global.workspace.daemon_oracle_review_authors.clone(),
oracle_review_max_per_cycle: global.workspace.daemon_oracle_review_max_per_cycle,
```

**`src/cli/config.rs` — show/get output (~lines 171–183, 280–292)**

Add the four fields to both the global and project-scoped daemon JSON objects so they appear in `config show` and `config get daemon.*` output:

```rust
"oracle_review_enabled": effective.daemon.oracle_review_enabled,
"oracle_review_timeout_secs": effective.daemon.oracle_review_timeout_secs,
"oracle_review_authors": effective.daemon.oracle_review_authors,
"oracle_review_max_per_cycle": effective.daemon.oracle_review_max_per_cycle,
```

**`src/daemon/runtime.rs` — `DaemonRuntimeConfig` (~line 30)**

Add four fields, populated from `EffectiveDaemonConfig` at construction time in `src/cli/daemon.rs`:

```rust
pub oracle_review_enabled: bool,
pub oracle_review_timeout_secs: u64,
pub oracle_review_authors: Vec<String>,
pub oracle_review_max_per_cycle: u32,
```

### 2. GitHub helpers (`src/daemon/github.rs`)

Add two new async functions reusing existing patterns:

**`list_open_non_draft_prs`**

```rust
pub async fn list_open_non_draft_prs(
    gh_bin: &str,
    owner: &str,
    repo: &str,
) -> Result<(Vec<OpenPrInfo>, bool)>
```

Calls `gh pr list --repo {owner}/{repo} --state open --json number,headRefOid,isDraft,author --limit 100` via `tokio::process::Command`. Parses JSON, filters out `isDraft == true` at parse time. Returns `(Vec<OpenPrInfo>, overflow)` where `overflow = items.len() == 100`, matching the `poll_issues_with_gh_bin` pattern (line 146). The `OpenPrInfo` struct:

```rust
pub struct OpenPrInfo {
    pub number: u32,
    pub head_sha: String,
    pub author: String,
}
```

A pure `parse_open_prs(json: &str) -> Result<Vec<OpenPrInfo>>` function handles deserialization and draft filtering for unit-testability.

**`fetch_pr_diff`**

```rust
pub async fn fetch_pr_diff(
    gh_bin: &str,
    owner: &str,
    repo: &str,
    pr_number: u32,
) -> Result<String>
```

Calls `gh pr diff {pr_number} --repo {owner}/{repo}` and returns the raw diff string. Follows the `Command` + error-handling pattern used throughout `github.rs`.

### 3. Oracle review module (`src/daemon/oracle_review.rs`)

New module, added to `src/daemon/mod.rs` as `pub mod oracle_review;`.

**`OracleReviewState`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OracleReviewState {
    /// Maps `pr_number` (as string key for JSON compat) → last reviewed `head_sha`.
    pub reviewed: HashMap<String, String>,
}
```

Stored at `{workspace_root}/daemon/oracle-review-state/state.json`, following the subdirectory convention used by `pr-review-state/`, `pr-review-amendments/`, `pr-review-pending/`, and `tasks/`. Load/save use the atomic temp-file + rename pattern from `PrReviewState` (`pr_review.rs:54`):

- `load()`: Returns `Ok(default)` when file missing; returns `Err` on corrupt JSON (never silently resets dedup state)
- `save()`: Writes to `.json.tmp` then renames atomically; creates parent dir via `fs::create_dir_all`

State path function:

```rust
fn state_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join("daemon")
        .join("oracle-review-state")
        .join("state.json")
}
```

**`oracle_review_phase`**

```rust
pub async fn oracle_review_phase(config: &DaemonRuntimeConfig) -> Result<()>
```

The main phase function:

1. **Early return** if `!config.oracle_review_enabled`
2. **Fetch candidate PRs** via `list_open_non_draft_prs(gh_bin, owner, repo)`. Log a warning if the overflow flag is true: `"warning: oracle review: gh pr list returned 100 PRs, results may be truncated"` (matching the pattern from `poll_issues_with_gh_bin`)
3. **Filter by author allowlist**: If `config.oracle_review_authors` is non-empty, retain only PRs where `pr.author` matches an entry case-insensitively (`.to_lowercase()` comparison). Empty allowlist = all non-draft PRs eligible
4. **Load `OracleReviewState`** from disk
5. **Resolve bot login** via `github::fetch_authenticated_login_with_gh_bin(&config.gh_bin)`, cached across the phase call. This is required by the bot-scoped marker helpers
6. **Iterate** candidate PRs, enforcing `max_per_cycle` via a counter:
   - **Dedup check**: Skip if `state.reviewed[pr_number] == pr.head_sha` (already reviewed at this SHA)
   - **Fetch diff** via `fetch_pr_diff(gh_bin, owner, repo, pr.number)`
   - **Invoke oracle** via `process::run_command_with_timeout` (see §4 below). On timeout or non-zero exit, log a warning and **skip** to the next PR without updating state (so the PR is retried next cycle)
   - **Truncate** the oracle output: Compute available space as `GITHUB_COMMENT_LIMIT - marker.chars().count() - 1` (for newline separator), then call `truncate_for_github(&formatted_body, available)`. The `truncate_for_github` function and `GITHUB_COMMENT_LIMIT` constant are currently private to `runtime.rs` — refactor to `pub(crate)` visibility so `oracle_review.rs` can reuse them
   - **Post review** via `post_bot_comment_with_marker_with_gh_bin(gh_bin, owner, repo, pr.number, &marker, &truncated_body, &bot_login)`. The built-in idempotency check (bot-scoped marker lookup) prevents duplicates and rejects spoofed user-authored markers
   - **Update state**: Set `state.reviewed[pr_number] = pr.head_sha` and save to disk. State is only advanced after a successful post — if posting fails, log a warning and continue without updating so the PR is retried next cycle
   - **Increment counter** and break when `count >= config.oracle_review_max_per_cycle`

The marker format is `<!-- ralph:oracle-review:{pr_number}:{head_sha} -->`, placed at the top of the comment body.

### 4. Oracle process invocation

Oracle is invoked as a synchronous subprocess using the existing `process::run_command_with_timeout` from `src/daemon/process.rs`. This function already handles:

- **Process groups**: Spawns with `.process_group(0)` so the oracle process and its descendants form a killable group
- **Output draining**: Concurrent threads drain stdout/stderr to prevent pipe-buffer deadlocks
- **Timeout + kill**: On timeout, sends `SIGKILL` to the entire process group, then reaps the child
- **Missing binary**: Spawn failure returns `RalphError::Orchestration("failed to spawn command: ...")`

Since `run_command_with_timeout` is synchronous, invoke it from `spawn_blocking` (matching the rebase-agent pattern):

```rust
let output = spawn_blocking_op(move || {
    let mut cmd = std::process::Command::new("oracle");
    cmd.args(["--prompt", &system_prompt, "--pipe"]);
    cmd.stdin(std::process::Stdio::piped());
    // Write diff to a temp file and pass via --file, or pipe via stdin
    process::run_command_with_timeout(&mut cmd, Duration::from_secs(timeout_secs))
}).await?;
```

Stdin piping approach: Write the diff to a temporary file in `{workspace_root}/daemon/oracle-review-state/` and pass it via the `oracle` CLI's file argument, avoiding the need to coordinate stdin writes with the timeout loop. If `oracle` supports `--pipe` with stdin, pipe the diff in by taking ownership of `child.stdin` before calling `run_command_with_timeout` — but note that `run_command_with_timeout` already takes a `&mut Command` and spawns internally, so the diff must either be passed as a file argument or pre-written to a temp file that is passed via shell redirection. The exact approach will be validated during implementation against the oracle CLI's interface; the key contract is: diff in, review on stdout, with the full process-lifecycle guarantees from `process.rs`.

**Error handling**:

- **Timeout** (`"command timed out"`): Log warning, skip PR, do not update state
- **Non-zero exit**: Log warning with stderr, skip PR, do not update state
- **Spawn failure** (oracle not installed): Log warning, skip PR. The phase does not abort — it continues to the next candidate

The system prompt:

```
You are a senior code reviewer. Review this PR diff for bugs, security issues, performance problems, and code quality. Be concise and actionable. Focus on substantive issues, not style nits.
```

### 5. Integration into daemon loop (`src/daemon/runtime.rs`)

Insert the call after `pr_review_phase` (~line 893), before the interactive PRD phase:

```rust
// Oracle review phase: autonomous code review on open PRs
if config.oracle_review_enabled {
    if let Err(err) = super::oracle_review::oracle_review_phase(&config).await {
        eprintln!("warning: oracle review phase failed: {err}");
    }
}
```

No interaction with `children`, `repo_root_lock`, or `pr_review_phase` state.

### 6. Visibility refactoring in `runtime.rs`

Promote `truncate_for_github` from `fn` to `pub(crate) fn` and `GITHUB_COMMENT_LIMIT` from `const` to `pub(crate) const` so they can be reused by `oracle_review.rs`. No logic changes.

### 7. Dedup marker format

The HTML comment marker `<!-- ralph:oracle-review:{pr_number}:{head_sha} -->` is embedded at the top of each review comment. This serves dual purpose:
- `post_bot_comment_with_marker_with_gh_bin` uses it for bot-scoped idempotent posting (prevents duplicates on the same SHA and rejects user-authored spoof markers)
- Different SHAs produce different markers, allowing new reviews when code changes

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/mod.rs` | Add `pub mod oracle_review;` |
| `src/daemon/oracle_review.rs` | **New file**: `OracleReviewState`, `oracle_review_phase`, state path helpers |
| `src/daemon/github.rs` | Add `list_open_non_draft_prs`, `fetch_pr_diff`, `OpenPrInfo` struct, and pure `parse_open_prs` parser |
| `src/config/global.rs` | Add 4 `daemon_oracle_review_*` fields to `WorkspaceConfig`, 2 default fns, 4 `set_global_config_value` match arms with bounds validation |
| `src/config/mod.rs` | Add 4 fields to `EffectiveDaemonConfig`, wire in `resolve_daemon_config` |
| `src/cli/config.rs` | Add 4 fields to both global and project-scoped daemon JSON output blocks in show/get |
| `src/cli/daemon.rs` | Wire 4 fields from `EffectiveDaemonConfig` into `DaemonRuntimeConfig` construction |
| `src/daemon/runtime.rs` | Add 4 fields to `DaemonRuntimeConfig`; promote `truncate_for_github` and `GITHUB_COMMENT_LIMIT` to `pub(crate)`; insert `oracle_review_phase` call in main loop |
| `src/validate/tests_daemon_oracle_review.rs` | **New file**: conformance tests |
| `src/validate/mod.rs` | Register `tests_daemon_oracle_review` module |

## Testing Strategy

### Validate conformance tests (`src/validate/tests_daemon_oracle_review.rs`)

Following AGENTS.md requirements and the `tests_daemon_rebase.rs` pattern: export `pub fn tests() -> Vec<ConformanceTest>`, register in `src/validate/mod.rs`, each test takes `&RalphHarness` and returns `TestResult` via `run_case(|| { ... })`.

**Config tests:**

- `oracle_review::config_defaults` — verify `config get workspace.daemon_oracle_review_enabled` returns `false`, `daemon_oracle_review_timeout_secs` returns `900`, `daemon_oracle_review_max_per_cycle` returns `3`, `daemon_oracle_review_authors` returns `[]`
- `oracle_review::config_set_enabled` — `config set workspace.daemon_oracle_review_enabled true` roundtrips correctly
- `oracle_review::config_set_timeout` — set and get timeout, verify bounds rejection for `0`
- `oracle_review::config_set_max_per_cycle` — set and get max_per_cycle, verify bounds rejection for `0`
- `oracle_review::config_set_authors` — set and get author allowlist via `cfg_parse_string_list` format
- `oracle_review::config_show_includes_fields` — verify `config show` output includes all four oracle review fields in the `daemon` section

**Phase behavior tests** (using mock `gh` and `oracle` scripts via `harness.write_mock_script` + `ralph_with_path`):

- `oracle_review::phase_disabled_is_noop` — phase does nothing when `daemon_oracle_review_enabled = false`; no state file created, no `gh` or `oracle` calls
- `oracle_review::phase_posts_review_for_new_pr` — mock `gh pr list` returns one non-draft PR, mock `gh pr diff` returns a diff, mock `oracle` returns review text, mock `gh issue comment` succeeds; verify comment was posted (check mock script was invoked)
- `oracle_review::phase_skips_draft_prs` — mock `gh pr list` returns only draft PRs; oracle is never invoked
- `oracle_review::phase_skips_already_reviewed_sha` — pre-seed state file with `pr_number → head_sha`; verify oracle is not invoked for that PR
- `oracle_review::phase_rereviews_on_sha_change` — pre-seed state with old SHA; mock returns PR with new SHA; verify oracle is invoked and state is updated
- `oracle_review::phase_respects_author_allowlist` — configure allowlist with `["alice"]`; mock returns PRs from alice and bob; verify only alice's PR is reviewed
- `oracle_review::phase_author_allowlist_case_insensitive` — configure `["Alice"]`; mock returns PR from `alice`; verify it is reviewed
- `oracle_review::phase_max_per_cycle_enforced` — mock returns 5 eligible PRs; configure `max_per_cycle = 2`; verify only 2 oracle invocations occur
- `oracle_review::phase_skips_when_marker_exists` — mock `gh` to return that a bot comment with the marker already exists; verify oracle is not invoked
- `oracle_review::phase_timeout_skips_without_state_update` — mock `oracle` to sleep longer than timeout; verify phase logs warning and state is not updated for that PR
- `oracle_review::phase_oracle_nonzero_exit_skips` — mock `oracle` exits with code 1; verify warning logged and state not updated
- `oracle_review::phase_oracle_missing_binary` — configure PATH without oracle; verify warning logged, phase continues
- `oracle_review::phase_post_failure_does_not_advance_state` — mock `gh issue comment` to fail; verify state is not updated for that PR
- `oracle_review::phase_overflow_logged` — mock `gh pr list` returns exactly 100 PRs; verify overflow warning is emitted to stderr

**Unit tests** (in-module `#[cfg(test)]` within the respective source files):

- `oracle_review.rs`:
  - `OracleReviewState` load/save roundtrip (follows `pr_review.rs:855` pattern)
  - State load returns default when file missing
  - State load returns error on corrupt JSON (not silent reset)
  - Dedup logic: skip when `state.reviewed[pr_number] == head_sha`
  - Dedup logic: re-review when SHA differs

- `github.rs`:
  - `parse_open_prs` pure parser: valid JSON, drafts filtered, author extracted
  - `parse_open_prs` returns empty vec for `"[]"` input
  - `parse_open_prs` returns error for malformed JSON

**What is NOT tested:**

- End-to-end daemon loop integration (oracle review is a leaf phase with no shared state; the conformance tests cover the phase function directly)
- Actual oracle CLI behavior (mocked in all tests)

## Out of Scope

- Project-level config overrides for oracle review settings (can be added later following the `ProjectDaemonOverrides` pattern)
- Inline/file-level review comments (GitHub review API) — this phase posts top-level PR comments only
- Re-reviewing when PR description changes (only head SHA changes trigger re-review)
- Oracle model/backend selection — uses whatever `oracle` CLI is configured to use
- Retrying failed oracle invocations within the same cycle (failed PRs are naturally retried next cycle)
- Interaction with `pr_review_phase` amendments (oracle reviews are informational, not actionable amendments)
- Rate limiting against GitHub API beyond the per-cycle cap
- Pagination beyond the 100-PR `gh pr list` limit (overflow is detected and logged but not paginated through)
- Multi-part comment splitting for oversized oracle output (truncation with `[truncated]` marker is used instead)
- Graceful SIGTERM escalation for oracle subprocess (the process-group SIGKILL on timeout is sufficient since oracle is a stateless, read-only tool with no cleanup requirements)