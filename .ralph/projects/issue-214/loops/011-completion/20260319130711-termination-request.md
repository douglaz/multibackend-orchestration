---
artifact: termination-request
loop: 11
project: issue-214
backend: claude(opus)
role: planner
created_at: 2026-03-19T13:07:11Z
---

# Project Completion Request

## Rationale
All acceptance criteria from the master prompt are satisfied. The five feature loops addressed the full implementation plus every amendment raised during final review rounds:

- **Loop 1** delivered the core `oracle_review_phase`: config wiring (4 fields across `WorkspaceConfig`, `EffectiveDaemonConfig`, `DaemonRuntimeConfig`, CLI surfaces), GitHub helpers (`OpenPrInfo`, `parse_open_prs`, `list_open_non_draft_prs`, `fetch_pr_diff`), persisted dedup state, comment idempotency via marker, oracle invocation through `process::run_command_with_timeout`, per-cycle cap, overflow warning, and full validate conformance test suite.
- **Loop 3** fixed the three Round 1 amendments: replaced the unsupported `--system` flag with the documented oracle CLI surface (ORACLE-REV-001), switched to unique temp files for atomic state writes (ORACLE-REV-002), and made the spawn-failure test exercise a real spawn error (ORACLE-REV-003).
- **Loop 5** corrected the system prompt to match the exact text specified in the master prompt.
- **Loop 7** separated comment-post success from readback failure so that state advances and per-cycle cap counts correctly even when the post-comment metadata fetch fails (ORACLE-REVIEW-FR-001).
- **Loop 9** tightened marker dedup from substring matching to exact first-line matching, preventing false-positive suppression from embedded markers (ORACLE-REV-FINAL-001).

The three external P2 PR-review suggestions (async phase execution, SHA-pinned diffs, informational summary) are enhancements beyond the master prompt's requirements and do not block acceptance.

## Summary of Work
- `src/daemon/oracle_review.rs` — new module: `OracleReviewState` (load/save with unique temp files), `oracle_review_phase` runtime algorithm, oracle invocation via documented CLI flags
- `src/daemon/github.rs` — `OpenPrInfo`, `parse_open_prs`, `list_open_non_draft_prs`, `fetch_pr_diff`, exact-marker-line matching helper, post-outcome separation (posted vs readback-failure vs post-failed)
- `src/daemon/mod.rs` — exports `oracle_review`
- `src/daemon/runtime.rs` — `DaemonRuntimeConfig` fields, `pub(crate)` truncation helpers, phase call site after `pr_review_phase`
- `src/config/global.rs` — 4 workspace config fields with defaults and validation
- `src/config/mod.rs` — `EffectiveDaemonConfig` wiring
- `src/cli/config.rs` — `config get` / `config show` support
- `src/cli/daemon.rs` — `DaemonRuntimeConfig` population
- `src/validate/tests_daemon_oracle_review.rs` — full conformance test suite covering all required scenarios
- `src/validate/mod.rs` — module registration

## Remaining Items
- P2: Move oracle reviews to a background task to avoid blocking the main poll loop during slow oracle invocations
- P2: Pin `gh pr diff` to the listed `headRefOid` to avoid race conditions if commits are pushed between list and diff fetch
- None of these block acceptance criteria

---
