---
artifact: completer-verdict
loop: 10
project: issue-214
backend: claude(opus)
role: completer
created_at: 2026-03-19T07:54:16Z
---

I've now thoroughly verified all the key implementation files. Let me compile the final verification against every acceptance criterion.

# Verdict: COMPLETE

The project satisfies all requirements:

- **`oracle_review_phase` exists and is called from the daemon poll loop after `pr_review_phase`**: Confirmed in `src/daemon/runtime.rs:904` — called immediately after `pr_review_phase` at line 899, wrapped in non-fatal error handling.
- **Phase is disabled by default and returns immediately unless enabled**: `oracle_review.rs:108` checks `config.oracle_review_enabled` and returns `Ok(())` if false. Default is `false` in `global.rs:964`.
- **Draft PRs are excluded before phase-level processing**: `parse_open_prs` in `github.rs:2484` filters `!pr.is_draft` during parsing. Test `draft_prs_skipped` confirms.
- **Author allowlist behavior matches spec**: `oracle_review.rs:125-131` normalizes allowlist to lowercase and filters with case-insensitive matching. Empty allowlist permits all. Tests `author_allowlist_enforced` and `author_allowlist_case_insensitive` confirm.
- **Dedup state persists at `{workspace_root}/daemon/oracle-review-state/state.json`**: `oracle_review.rs:282-284` constructs the correct path. State uses `HashMap<String, String>` keyed by PR number string.
- **Reviewing is keyed by `(pr_number, head_sha)`**: `reviewed_sha_matches` at line 94 and `mark_reviewed` at line 101 implement this correctly.
- **Changed `head_sha` causes a fresh review**: Test `dedup_same_sha_and_rereview_on_change` confirms this behavior.
- **Existing bot-authored marker comments skip oracle invocation and reconcile state**: `oracle_review.rs:160-188` checks for existing exact marker, updates state if found, and skips oracle. Test `existing_bot_marker_skips_oracle` confirms.
- **Oracle is invoked only through `process::run_command_with_timeout`**: `oracle_review.rs:366` calls `process::run_command_with_timeout`.
- **Oracle timeout uses `daemon_oracle_review_timeout_secs`**: `oracle_review.rs:366` passes `Duration::from_secs(timeout_secs)`.
- **Review comments use the exact marker format**: `oracle_review.rs:287` produces `<!-- ralph:oracle-review:{pr_number}:{head_sha} -->`.
- **Comments posted through `post_bot_comment_with_marker_outcome_with_gh_bin`**: `oracle_review.rs:229` calls this function, which distinguishes post-failure from readback-failure.
- **Comment bodies are truncated with the shared GitHub helper**: `oracle_review.rs:224-227` computes available budget accounting for marker and newline, then calls `truncate_for_github`.
- **`truncate_for_github` and `GITHUB_COMMENT_LIMIT` are `pub(crate)`**: `runtime.rs:110` and `runtime.rs:647` confirmed.
- **No more than `daemon_oracle_review_max_per_cycle` successful reviews per cycle**: `oracle_review.rs:151` checks cap before processing; only incremented on `Posted` outcome at line 241. Test `per_cycle_max_enforced` confirms.
- **100-PR overflow detected and logged with exact warning string**: `oracle_review.rs:122` matches the required string exactly. Test `overflow_warning_logged` verifies exact match.
- **Config wiring complete across all surfaces**: All 4 fields present in `WorkspaceConfig` (`global.rs`), `EffectiveDaemonConfig` (`config/mod.rs`), `DaemonRuntimeConfig` (`runtime.rs`), `set_global_config_value` (`global.rs`), `config get`/`config show` (`cli/config.rs`), and `cli/daemon.rs`.
- **Validation rules enforced**: `global.rs:1447-1465` rejects timeout=0 and max-per-cycle=0 with `must be > 0`. `config/mod.rs:657-669` validates at resolution time.
- **Atomic write semantics for state save**: `oracle_review.rs:60-90` uses temp file with unique name (pid+timestamp+nonce) then rename.
- **Corrupt JSON returns error, not default**: `oracle_review.rs:38-43` returns error with "corrupted oracle review state" message. Unit test confirms.
- **State persisted immediately after each successful PR**: `oracle_review.rs:243` and `oracle_review.rs:252` save state right after marking reviewed.
- **`spawn_blocking` used for oracle invocation**: `oracle_review.rs:336` wraps the synchronous process helper.
- **Diff written to temp file under oracle-review-state dir**: `oracle_review.rs:346-348` writes diff to the correct location and cleans up at lines 387-388.
- **System prompt matches spec exactly**: `oracle_review.rs:16` verified, test `eligible_pr_reviewed` asserts exact match.
- **Exact marker line matching (not substring)**: `github.rs:2291-2293` uses `marker_matches_exact_marker_line` requiring body starts with marker+newline or equals marker. Test `embedded_marker_does_not_suppress_oracle` confirms embedded markers don't match.
- **Validate module registered**: `validate/mod.rs:21` imports module, line 143 registers tests.
- **All required conformance tests present**: 20 tests covering config defaults, set/get roundtrips, bounds rejection, config show, disabled noop, eligible PR, draft skip, dedup/rereview, author allowlist (+ case insensitive), per-cycle max, existing bot marker, embedded marker, oracle timeout/exit/spawn failures, missing binary, comment post failure, readback failure, and overflow warning.
- **`pub mod oracle_review` in daemon/mod.rs**: Confirmed at line 4.
- **Phase independent of `pr_review_phase`**: No shared state, separate module, no cross-references.

---
