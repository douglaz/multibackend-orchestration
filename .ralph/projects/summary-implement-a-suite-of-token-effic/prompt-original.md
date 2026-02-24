The revised specification has been written to `.ralph/quick-prd/7c0c9df46eef/draft.md`. Here's a summary of how each review issue was addressed:

**Issue 1 – Config Surface Completeness**: Added explicit `B-Config`, `C-Config`, and `D-Config` sub-sections that specify changes across all four config layers (`global.rs`, `project.rs`, `mod.rs`/`EffectiveWorkflowConfig`, `cli/config.rs`) with exact field names, types, parse functions, and merge tests.

**Issue 2 – Conformance Test Requirement**: Added a "Cross-cutting – Conformance tests" acceptance criteria section specifying 6 conformance tests in a new `src/validate/tests_sessions.rs` module, covering history capping, session lifecycle, rollback invalidation, prompt-change invalidation, and working directory invariants.

**Issue 3 – Normalization Integration Point**: Specified that `normalize_output` runs immediately after every `backend.execute_with_log()` call, before `parse_fn` sees the output. Added explicit code showing the integration point and graceful degradation on normalization errors.

**Issue 4 – Parse-Retry Attempt Semantics**: Defined a 4-attempt pipeline (up from 3) when sessions are active. Specified exact attempt numbering (1=initial, 2=session follow-up, 3=reformatter, 4=reminded). `ParseRetriesExhausted.attempts` reflects actual attempts executed (3 or 4).

**Issue 5 – Session ID Lifecycle**: Added D-SessionID section with 5 explicit rules: first call with/without session ID, resume with new/missing ID, and parse failure on first call still stores the record.

**Issue 6 – Backend Arg Rewriting Robustness**: Changed `effective_args` to return `Result<Vec<String>>`. Missing markers (`-p` for Claude, `"-"` for Codex) produce `Err`. The orchestrator catches this, logs a warning, and falls back to full prompt. Added unit tests for both success and failure paths, plus idempotency.

**Issue 7 – Bootstrap Hash Definition**: Added D-Bootstrap section specifying: per-role template hashing (only the role's own template), fallback content hashed when file is missing, `spec_hash` definition (empty string for planner), and no multi-template sorting needed since records are per-role.

**Issue 8 – Isolation vs Reset Flags**: Session removal for loops > target is now **unconditional** during rollback. The `session_reuse_reset_on_rollback` flag controls only the target loop's own sessions. This prevents stale reuse when loop numbers are recycled.

**Issue 9 – History Capping Consistency**: Specified explicit `sort_by_key(|e| e.iteration)` ascending before capping. Added a unit test with non-sequential insertion order proving sort governs, not insertion order.

**Issue 10 – Working Directory Invariant**: Added a `debug_assert!` in the orchestrator before each backend invocation, plus a conformance test (`sessions::working_directory_stays_at_repo_root`) that uses a mock script capturing `pwd`.

**Issue 11 – Session Role Allowlist Validation**: Dual policy: `config set` validates against known roles and returns `Err` for unknowns; the orchestrator at runtime uses warn-and-skip (filters out unrecognized roles with `tracing::warn!`). Added unit tests for both paths.

**Issue 12 – Token Metrics Semantics**: Metrics are logged per-attempt (not just final). Each log entry includes an `attempt` field (1-based) for distinguishing initial calls from retries.