## Summary

The in-memory `SessionStore` (a `Vec<SessionRecord>` on `ProjectState`) is lost whenever the orchestrator process exits. This means `--resume`-style session IDs cannot survive across separate `ralph run` invocations, crashes, or CLI rollback commands (which run in a different process). The fix is straightforward: persist `SessionStore` to a JSON file (`session-store.json`) inside the project directory, write it after every mutation, and load it during state reconstruction. The `SessionStore` and `SessionRecord` types already derive `Serialize`/`Deserialize`, and a serde roundtrip test already exists (`session_store_serde_roundtrip` at `state.rs:608`), so no new serialization work is needed.

To survive crashes mid-write, persistence uses an atomic write pattern (write to a temporary file in the same directory, `fsync`, then rename over the target). The `tempfile` crate is already a dependency.

## Acceptance Criteria

1. After each backend call that mutates the session store, a `session-store.json` file exists at `<project_dir>/session-store.json` containing the current `SessionStore` as JSON.
2. `reconstruct_project_state_internal` loads `session-store.json` (if present) into `state.session_store` before returning, enabling session reuse across process restarts.
3. A second `ralph run` invocation that resumes an in-progress loop reuses session IDs from the previous run (verified via `session_store.records` having matching `session_id` values for the same loop number).
4. CLI `ralph rollback` persists the invalidated session store to `session-store.json` after clearing sessions (per `session_reuse_reset_on_rollback`).
5. Internal rollback (`rollback_current_loop`) persists session state to disk at every call site, including paths where the process returns immediately afterward (QA/review iteration limit exceeded with `until_complete=false`). `handle_prompt_change` also persists after its session save/restore dance.
6. When `session_reuse_reset_on_rollback` is enabled and rollback targets loop N, `session-store.json` contains no records for loop N.
7. When `session_reuse_reset_on_prompt_change` is enabled and a prompt change triggers `RestartLoop`, `session-store.json` contains no records for the restarted loop.
8. When `session_reuse_reset_on_prompt_change` is **disabled**, `session-store.json` preserves sessions through the restart.
9. Each project's `session-store.json` lives inside its own `<project_dir>/`, preventing cross-project session leakage.
10. Corrupt or missing `session-store.json` degrades gracefully to an empty `SessionStore` (no crash, no panic). A warning is logged on corrupt parse.
11. A crash during `persist_session_store` cannot leave a truncated `session-store.json`; the atomic write pattern preserves the last-known-good file.
12. Tests verify cross-restart persistence, rollback invalidation with persistence, and prompt-change invalidation with persistence.

## Technical Approach

### File location and format

- **Path**: `<project_dir>/session-store.json` (alongside `.last-prompt-hash`, `prompt.md`, `project.toml`)
- **Format**: `serde_json::to_string_pretty(&state.session_store)` — human-readable, consistent with how `ProjectState` is serialized for `project show --json`
- **Constant**: Add `pub const SESSION_STORE_FILE: &str = "session-store.json";` in `project/lifecycle.rs` (exported for use by `orchestrator.rs` and `rollback.rs`)

### Persistence helper (atomic write)

Add a free function in `project/lifecycle.rs`:

```rust
pub fn persist_session_store(project_dir: &Path, store: &SessionStore) -> Result<()> {
    let path = project_dir.join(SESSION_STORE_FILE);
    let json = serde_json::to_string_pretty(store)?;
    // Atomic write: temp file in same directory → fsync → rename.
    // Guarantees that `session-store.json` is either the old complete file
    // or the new complete file, never a truncated/partial write.
    let mut tmp = tempfile::NamedTempFile::new_in(project_dir)?;
    tmp.write_all(json.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path)?;
    Ok(())
}
```

This uses `tempfile::NamedTempFile` (already a dependency in `Cargo.toml`) to write to a temporary file in the same directory, `fsync` the contents, then atomically rename over the target. On crash:
- If the process dies before `persist()`: the temp file is cleaned up by the OS or on next write; `session-store.json` retains its previous contents.
- If the process dies during `persist()` (rename): on POSIX, `rename(2)` is atomic — the file is either fully replaced or not.

### Write points (8 total)

**5 upsert sites in `orchestrator.rs`** — add `persist_session_store(&project_dir, &state.session_store)?;` immediately after each `upsert_session_after_execution(...)` call:
1. Initial implementation (line ~633)
2. Review-iteration implementation (line ~789)
3. QA-iteration implementation (line ~948)
4. QA execution (line ~1177)
5. Reviewer execution (line ~1425)

**1 write in `handle_prompt_change`** (after the session save/restore dance at line ~2333) — persist the final state whether sessions were cleared or preserved.

**2 writes after `rollback_current_loop` in iteration-limit-exceeded paths** — the QA iteration limit (line ~1053) and review iteration limit (line ~1941) call sites both have a path where `until_complete` is false and the function returns `Err` immediately, without any subsequent backend call that would trigger a persist. A persist call is needed after `rollback_current_loop` returns at each of these sites, before the `checkpoint_phase_transition` call (so the on-disk state is consistent with the checkpoint commit):
- Line ~1053: after `rollback_current_loop(...)`, before `checkpoint_phase_transition(...)`
- Line ~1941: same pattern

These two writes are necessary because when `until_complete=false`, the process returns an error immediately after `checkpoint_phase_transition` — no further `upsert_session_after_execution` calls occur, so the in-memory session invalidation from `remove_loop()` would never reach disk.

When `until_complete=true`, the `continue` re-enters the loop and the next backend call will persist anyway, but persisting eagerly after rollback is harmless and ensures correctness regardless of the code path taken.

**1 write in CLI rollback** (`cli/rollback.rs`) — after the session invalidation block (after line ~156):

```rust
persist_session_store(&project_dir, &state.session_store)?;
```

This ensures the CLI rollback command (which runs in a separate process) writes the invalidated store to disk.

### Read point

In `lifecycle.rs:reconstruct_project_state_internal`, after `state` is fully constructed (line ~323, just before `Ok(state)`), add:

```rust
let session_path = project_dir.join(SESSION_STORE_FILE);
match fs::read_to_string(&session_path) {
    Ok(json) => match serde_json::from_str::<SessionStore>(&json) {
        Ok(store) => state.session_store = store,
        Err(e) => warn!(path = %session_path.display(), error = %e, "corrupt session-store.json, starting with empty session store"),
    },
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // expected for new projects
    Err(e) => warn!(path = %session_path.display(), error = %e, "could not read session-store.json, starting with empty session store"),
}
```

This provides graceful degradation (missing file → empty store, corrupt JSON → empty store with warning, I/O error → empty store with warning) while logging enough information to diagnose issues.

### Interaction with existing invariants

- **Bootstrap hash staleness**: Already handled. `resolve_session_for_role` compares the stored `bootstrap_hash` against the freshly computed one. If prompt/spec/template changed between runs, the hash won't match, and the stored session is ignored (fresh call).
- **Project lock**: CLI rollback already acquires `ProjectLock` before modifying state. The orchestrator is the sole writer during its run. No new locking needed.
- **`ProjectState::remove_loop`**: Calls `session_store.remove_for_loop()` in memory. The caller is responsible for persisting afterward — this is the existing pattern (analogous to how loop artifact directories are removed but state isn't serialized to disk).

## Files & Modules

| File | Change |
|------|--------|
| `src/project/lifecycle.rs` | Add `pub const SESSION_STORE_FILE` constant. Add `pub fn persist_session_store(...)` with atomic write. Load `session-store.json` in `reconstruct_project_state_internal` (~8 lines at line ~323). |
| `src/workflow/orchestrator.rs` | Import `persist_session_store` and `SESSION_STORE_FILE`. Add persist call after each of the 5 `upsert_session_after_execution` sites (lines ~633, ~789, ~948, ~1177, ~1425). Add persist call after `rollback_current_loop` in QA limit path (line ~1053) and review limit path (line ~1941). Add persist call after session restore in `handle_prompt_change` (line ~2333). Total: 8 new persist calls. |
| `src/cli/rollback.rs` | Import `persist_session_store`. Add persist call after session invalidation block (after line ~156). |
| `src/project/state.rs` | No changes needed. `SessionStore` and `SessionRecord` already derive `Serialize`/`Deserialize`. |
| `src/validate/tests_sessions.rs` | Add 3 new E2E tests (see Testing Strategy). |
| `src/project/lifecycle.rs` (tests module) | Add 3 unit tests for persist/load roundtrip and degradation. |

## Testing Strategy

### Unit tests (in `lifecycle.rs` tests module)

1. **`persist_and_load_session_store_roundtrip`** — Create a `SessionStore` with records, call `persist_session_store`, then simulate the load logic from `reconstruct_project_state_internal`. Assert records match field-by-field.
2. **`missing_session_store_file_yields_empty_store`** — Point at a project dir with no `session-store.json`. Run the load logic. Assert `session_store.records` is empty and no panic.
3. **`corrupt_session_store_file_yields_empty_store`** — Write garbage bytes to `session-store.json`, run the load logic, assert empty store (graceful degradation).

### E2E conformance tests (in `tests_sessions.rs`)

4. **`session_persistence_survives_restart`** — Enable session reuse. Run `ralph run --until-review` to leave loop 1 in progress (mock script completes implementation but the run stops before review finishes). Capture session IDs from state via `h.load_state()`. Run `ralph run --loops 1` again on the same project (which continues the in-progress loop 1). Load state and assert session records exist with the same `session_id` values and `call_count > 1`, proving cross-invocation reuse on the same loop.
5. **`session_persistence_invalidated_by_rollback`** — Enable session reuse + `reset_on_rollback=true`. Run loop 1 to completion. Verify `session-store.json` has records via `h.load_state()`. Run `ralph rollback 0`. Load state, assert `session_store.records` has zero entries. Run again, assert fresh sessions (new `session_id` values).
6. **`session_persistence_invalidated_by_prompt_change`** — Enable session reuse + `reset_on_prompt_change=true`. Set `prompt_change_action=restart-loop`. Run `ralph run --until-review` to leave loop 1 in progress. Verify session records exist. Edit `prompt.md`. Run `ralph run --loops 1` (triggers prompt-change detection → `RestartLoop`). Verify session records for the restarted loop were cleared from `session-store.json`. The test must use `--until-review` (or similar) to ensure the loop is in-progress when the prompt change is detected, because `handle_prompt_change` short-circuits with a simple hash update when no loop is in progress.

### Existing tests remain valid

- All existing `tests_sessions.rs` tests continue to pass without modification — they test in-memory behavior which is unchanged. The addition of disk persistence is additive.
- The existing `session_store_serde_roundtrip` unit test already validates the serialization format.

## Out of Scope

- **Session store migration/versioning**: The current `SessionRecord` schema is stable. If it changes in the future, a migration can be added then. The `#[serde(default)]` on `SessionStore` already handles missing fields.
- **Compaction/pruning of old session records**: Records are scoped by `(loop_number, role, backend_spec)` and cleaned up on loop removal. No unbounded growth.
- **Encryption or obfuscation of session IDs on disk**: Session IDs are opaque strings managed by the backend. They live in the `.ralph/` directory alongside other project artifacts.
- **Persisting session store on every orchestrator status change**: Only mutation events (backend execution, rollback, prompt change) trigger writes. Read-only status checks do not.
- **Moving the file into loop-specific subdirectories**: The store spans all loops for a project, so it belongs at the project directory level.
- **Daemon-level session caching or cross-project session sharing**: Sessions are strictly per-project.
- **fsync of parent directory**: On Linux, `rename` atomicity guarantees the file is either fully replaced or not. An `fsync` on the parent directory would ensure the directory entry is durable after a power loss, but this is overkill for session data — the worst case is reverting to the pre-rename state (previous valid file or no file), which degrades gracefully to an empty store. The trade-off is acceptable given sessions are a performance optimization, not critical data.
