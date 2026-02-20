### Feature
Persist `SessionStore` to disk so session reuse survives process restarts and cross-process commands (for example `ralph rollback`).

### Problem
`ProjectState.session_store` currently lives only in memory. When a process exits, session IDs are lost, so later invocations cannot reuse backend sessions.

### Goal
Make session persistence reliable, per-project, and testable with graceful read-side degradation.

### Scope
In scope:
1. Persist `SessionStore` to `<project_dir>/session-store.json` after every mutation.
2. Load `session-store.json` during project-state reconstruction.
3. Ensure rollback/prompt-change invalidation is persisted.
4. Add unit and validate conformance tests for restart persistence and invalidation behavior.

Out of scope:
1. Atomic temp-file + rename writes.
2. Session schema migration/versioning.
3. Encryption/obfuscation of session IDs.
4. Cross-project or daemon-level shared session caches.

### Required Behavior
1. File path is exactly `<project_dir>/session-store.json` (project-scoped, never global).
2. Serialization uses existing `SessionStore` serde support with `serde_json::to_string_pretty`.
3. Every successful mutation to `state.session_store` is followed by a persistence write before control returns from that mutation path.
4. `reconstruct_project_state_internal` attempts to load `session-store.json` and assign `state.session_store`.
5. Missing file or corrupt JSON must not crash reconstruction; fallback is empty `SessionStore`.
6. Read-side fallback must be observable via logging (`warn`/equivalent) on parse failure (not required for missing file).
7. Write failures must return `Err` (do not silently ignore persistence failures).
8. Session invalidation behavior must persist:
1. With `session_reuse_reset_on_rollback=true`, rollback removing loop `N` leaves zero records for loop `N` in `session-store.json`.
2. With `session_reuse_reset_on_prompt_change=true`, prompt-change restart clears affected loop records and persists.
3. With `session_reuse_reset_on_prompt_change=false`, prompt-change restart preserves records and persists.
9. Per-project isolation is mandatory: one project’s writes must never affect another project’s `session-store.json`.

### Implementation Requirements
1. In `src/project/lifecycle.rs`:
1. Add `SESSION_STORE_FILE: &str = "session-store.json"`.
2. Add `pub fn persist_session_store(project_dir: &Path, store: &SessionStore) -> Result<()>`.
3. Add a load helper (or inline equivalent) used by `reconstruct_project_state_internal`.
4. In reconstruction, load and parse session store; on missing/corrupt fallback to empty.
2. In `src/workflow/orchestrator.rs`:
1. Import `persist_session_store`.
2. After each `upsert_session_after_execution(...)` call site, persist immediately.
3. In `handle_prompt_change`, persist after final session clear/preserve state is decided.
3. In `src/cli/rollback.rs`:
1. Import `persist_session_store`.
2. Persist immediately after rollback-driven session invalidation mutation.
4. Do not rely on line numbers; anchor edits by function names and mutation sites.

### Testing Requirements
1. Unit tests in `src/project/lifecycle.rs` (or nearby test module):
1. `persist_and_load_session_store_roundtrip`: persist then load, assert equality.
2. `missing_session_store_file_yields_empty_store`: reconstruct without file, assert empty records.
3. `corrupt_session_store_file_yields_empty_store`: write invalid JSON, reconstruct, assert empty records and no crash.
2. Validate conformance tests:
1. Add/extend `src/validate/tests_sessions.rs`.
2. Register in `src/validate/mod.rs`.
3. Include test `session_persistence_survives_restart`:
1. Run once, capture persisted records.
2. Run again on same project/config.
3. Assert reuse deterministically (same key retains `session_id` and/or `call_count` increases for reused records; choose one deterministic rule and enforce it).
4. Include test `session_persistence_invalidated_by_rollback`:
1. Enable rollback reset.
2. Create sessions, run rollback to target loop `N`.
3. Assert persisted file has no loop `N` records.
5. Include prompt-change tests for both flags:
1. `session_persistence_invalidated_by_prompt_change_enabled`.
2. `session_persistence_preserved_on_prompt_change_disabled`.
3. Assert persisted file matches expected cleared/preserved behavior.

### Acceptance Criteria
1. `session-store.json` is created/updated after each session-store mutation path.
2. Reconstruction loads persisted sessions when file is valid.
3. Missing/corrupt file falls back to empty store without crashing.
4. Second `ralph run` can reuse prior sessions across process restart (as validated by deterministic test assertion).
5. CLI rollback persists invalidated store state.
6. Prompt-change restart persists cleared or preserved sessions according to config.
7. No cross-project session leakage.
8. All new unit tests and validate tests pass.