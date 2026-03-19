---
artifact: final-review-proposals
loop: 13
project: issue-214
backend: claude
role: final_reviewer
created_at: 2026-03-19T14:10:48Z
---

Everything looks correct. Now let me finalize my review.

# Final Review: NO AMENDMENTS

## Summary

The oracle review phase implementation is correct, safe, and complete. I verified all 12 changed files totaling 3,382 lines of additions across the full diff from `master`.

**Core logic (`src/daemon/oracle_review.rs`):**
- The phase algorithm follows the spec precisely: early-return when disabled, PR listing with overflow detection, author filtering with case-insensitive matching, state-based dedup, exact marker checking, oracle invocation via `process::run_command_with_timeout` inside `spawn_blocking`, truncation-aware comment posting, per-cycle cap enforcement, and immediate state persistence after each successful review.
- Error handling is thorough — every failure path logs a warning and continues (lines 123-126, 143-146, 154-157, 213-218, 231-238, 277-282). The phase never propagates errors upward, keeping the poll loop alive.
- Temp file cleanup (lines 405-406) runs in all success and error paths via the closure-then-cleanup pattern.
- Atomic state saves via temp-file-plus-rename (lines 60-91) with uniqueness from pid+timestamp+nonce (lines 308-322).

**Config wiring (`src/config/global.rs`, `src/config/mod.rs`, `src/cli/config.rs`, `src/cli/daemon.rs`):**
- All 4 fields (`daemon_oracle_review_enabled`, `daemon_oracle_review_timeout_secs`, `daemon_oracle_review_authors`, `daemon_oracle_review_max_per_cycle`) are correctly threaded through `WorkspaceConfig`, `EffectiveDaemonConfig`, `DaemonRuntimeConfig`, `set_global_config_value` (with bounds validation for timeout=0 and max_per_cycle=0), `config get`, and `config show`.

**GitHub helpers (`src/daemon/github.rs`):**
- `parse_open_prs` correctly filters drafts during parsing and returns an overflow flag at exactly 100 PRs.
- `marker_matches_exact_marker_line` correctly requires the marker to be the first line or the entire body, rejecting embedded markers.
- The `PostCommentOutcome` enum properly separates post failures from readback failures, allowing the caller to advance state on `Posted` even when readback fails.
- All timeout-bounded variants correctly use `run_gh_with_timeout` → `process::run_command_with_timeout`, inheriting process-group kill behavior.

**Comment body truncation (`src/daemon/oracle_review.rs:241-244`):**
- Budget calculation: `available = 65536 - marker_len - 1` (accounting for marker and newline). The `post_bot_comment_with_marker_outcome_with_gh_bin_with_timeout` then combines `marker + "\n" + truncated_body`, totaling exactly `≤ GITHUB_COMMENT_LIMIT`.

**Poll loop integration (`src/daemon/runtime.rs:904-906`):**
- Placed correctly after `pr_review_phase` and before the PRD phase, with non-fatal error handling.

**Tests (`src/validate/tests_daemon_oracle_review.rs`, plus unit tests in `oracle_review.rs` and `github.rs`):**
- 25 conformance tests cover all required scenarios: config defaults/roundtrips/bounds, disabled no-op, eligible PR review, draft skipping, dedup same SHA, SHA change re-review, author allowlist (including case-insensitive matching), per-cycle cap, existing bot marker skip, embedded marker non-suppression, oracle timeout/exit/spawn failures (all isolated, not advancing state), comment post failure, readback failure advancing state, overflow warning, and timeout-bounded gh operations for all stages.
- Unit tests verify state load/save/corrupt behavior, concurrent save safety, temp path uniqueness, marker format, and parse_open_prs.

**Minor observations (not worth amendments):**
- `list_open_non_draft_prs` and `fetch_pr_diff` (non-timeout variants) in `github.rs` are defined but never called — the timeout variants supersede them. This is harmless dead code on a pub API surface (no compiler warning).
- `oracle_review_phase` always returns `Ok(())` since all errors are handled internally, making the `Err` branch in `runtime.rs:904` unreachable — this is benign defense-in-depth.
