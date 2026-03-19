---
artifact: completer-verdict
loop: 8
project: issue-214
backend: claude(opus)
role: completer
created_at: 2026-03-19T07:17:15Z
---

I've now verified every requirement against the actual source code. Here's my verdict:

# Verdict: COMPLETE

The project satisfies all requirements:

- **`oracle_review_phase` exists and is called from the daemon poll loop after `pr_review_phase`**: Confirmed in `src/daemon/runtime.rs:904`, placed immediately after `pr_review_phase` at line 899.
- **Phase is disabled by default and returns immediately unless enabled**: Default `false` in `global.rs:964-966`; early return in `oracle_review.rs:108-110`.
- **Draft PRs are excluded before phase-level processing**: `parse_open_prs` in `github.rs:2444` filters `.filter(|pr| !pr.is_draft)`.
- **Author allowlist behavior**: Case-insensitive matching via `normalized_author_allowlist` in `oracle_review.rs:271-276` using `to_ascii_lowercase()`. Empty allowlist admits all PRs (line 129).
- **Dedup state persists at `{workspace_root}/daemon/oracle-review-state/state.json`**: Confirmed in `oracle_review.rs:282-284`.
- **Reviewing is keyed by `(pr_number, head_sha)`**: `OracleReviewState.reviewed` is `HashMap<String, String>` with PR number as key and SHA as value (`oracle_review.rs:19-22`).
- **Changed `head_sha` causes a fresh review**: `reviewed_sha_matches` at line 94-99 compares SHA; mismatch proceeds to review.
- **Existing bot-authored marker comments skip oracle invocation and reconcile state**: Lines 160-188 check for existing marker, update state and skip on match.
- **Oracle is invoked only through `process::run_command_with_timeout`**: Confirmed at `oracle_review.rs:366`.
- **Oracle timeout uses `daemon_oracle_review_timeout_secs`**: Passed as `Duration::from_secs(timeout_secs)` at line 366.
- **Review comments use the exact marker format**: `oracle_review_marker` at line 287 produces `<!-- ralph:oracle-review:{pr_number}:{head_sha} -->`.
- **Comments are posted through `post_bot_comment_with_marker_outcome_with_gh_bin`**: Confirmed at line 229, with `PostCommentOutcome` handling for Posted/AlreadyExists/PostFailed.
- **Comment bodies are truncated with the shared GitHub helper**: `truncate_for_github` and `GITHUB_COMMENT_LIMIT` used at lines 224-227; both are `pub(crate)` in `runtime.rs:110,647`.
- **No more than `daemon_oracle_review_max_per_cycle` successful reviews per cycle**: Success count checked at line 151 and incremented at line 241 only on `PostCommentOutcome::Posted`.
- **100-PR overflow detected and logged with the exact warning string**: Confirmed at lines 121-123, exact match: `"warning: oracle review: gh pr list returned 100 PRs, results may be truncated"`.
- **Config wiring is complete**: All 4 fields wired through `WorkspaceConfig` (global.rs:90-97), `EffectiveDaemonConfig` (mod.rs:125-128), `DaemonRuntimeConfig` (runtime.rs:80-86), `set_global_config_value` (global.rs:1443-1466), `config get` (config.rs:183-186), `config show` (config.rs:296-299), and `cli/daemon.rs:258-261`.
- **Validate conformance tests**: All 19 required test scenarios are covered in `tests_daemon_oracle_review.rs` including config defaults, roundtrips, bounds rejection, config show, disabled no-op, eligible PR reviewed, draft skipping, dedup/re-review, author allowlist (both enforcement and case-insensitivity), per-cycle max, existing bot marker, timeout/exit/spawn failures, comment-post failure, readback-failure recovery, and overflow warning.
- **Unit tests**: Present in `oracle_review.rs:411-511` covering state load/save, corrupt state, dedup behavior, concurrent saves, temp path uniqueness, and marker format.
- **Module export**: `pub mod oracle_review;` in `daemon/mod.rs:4`.
- **Validate module registration**: `tests_daemon_oracle_review` registered in `validate/mod.rs:21,143`.
- **No project-level overrides**: Oracle review config only in workspace-level (`global.rs`), not in any project config.
- **`spawn_blocking` used for oracle invocation**: Confirmed at `oracle_review.rs:336`.
- **Temp file cleanup**: Diff and output temp files removed after oracle completes at lines 387-388.
- **Non-fatal phase**: Error logged as warning at `runtime.rs:905`, poll loop continues.
- **State saved immediately after each successful review**: Save calls at lines 243 and 253 (inside per-PR loop), not batched at end of cycle.
- **System prompt matches spec exactly**: Confirmed at line 16, verified by test at line 398-401.

---
