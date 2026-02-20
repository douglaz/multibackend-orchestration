I now have everything I need. Here is the engineering specification:

---

## Summary

The in-memory `SessionStore` (a `Vec<SessionRecord>` on `ProjectState`) is lost whenever the orchestrator process exits. This means `--resume`-style session IDs cannot survive across separate `ralph run` invocations, crashes, or CLI rollback commands (which run in a different process). The fix is straightforward: persist `SessionStore` to a JSON file (`session-store.json`) inside the project directory, write it after every mutation, and load it during state reconstruction. The `SessionStore` and `SessionRecord` types already derive `Serialize`/`Deserialize`, and a serde roundtrip test already exists (`session_store_serde_roundtrip` at `state.rs:608`), so no new serialization work is needed.

## Acceptance Criteria

1. After each backend call that mutates the session store, a `session-store.json` file exists at `<project_dir>/session-store.json` containing the current `SessionStore` as JSON.
2. `reconstruct_project_state_internal` loads `session-store.json` (if present) into `state.session_store` before returning, enabling session reuse across process restarts.
3. A second `ralph run` invocation on the same project reuses session IDs from the previous run (verified via `session_store.records` having matching `session_id` values).
4. CLI `ralph rollback` persists the invalidated session store to `session-store.json` after clearing sessions (per `session_reuse_reset_on_rollback`).
5. Internal rollback (`rollback_current_loop`) and `handle_prompt_change` persist updated session state to disk after mutation.
6. When `session_reuse_reset_on_rollback` is enabled and rollback targets loop N, `session-store.json` contains no records for loop N.
7. When `session_reuse_reset_on_prompt_change` is enabled and a prompt change triggers `RestartLoop`, `session-store.json` contains no records for the restarted loop.
8. When `session_reuse_reset_on_prompt_change` is **disabled**, `session-store.json` preserves sessions through the restart.
9. Each project's `session-store.json` lives inside its own `<project_dir>/`, preventing cross-project session leakage.
10. Corrupt or missing `session-store.json` degrades gracefully to an empty `SessionStore` (no crash).
11. Tests verify cross-restart persistence, rollback invalidation with persistence, and prompt-change invalidation with persistence.

## Technical Approach

### File location and format

- **Path**: `<project_dir>/session-store.json` (alongside `.last-prompt-hash`, `prompt.md`, `project.toml`)
- **Format**: `serde_json::to_string_pretty(&state.session_store)` — human-readable, consistent with how `ProjectState` is serialized for `project show --json`
- **Constant**: Add `const SESSION_STORE_FILE: &str = "session-store.json";` in `lifecycle.rs` (or a shared location used by both `lifecycle.rs` and `orchestrator.rs`)

### Persistence helper

Add a free function (co-located with the constant, likely in `project/lifecycle.rs` or a new thin `project/session_persistence.rs` module to avoid circular deps):

```rust
pub fn persist_session_store(project_dir: &Path, store: &SessionStore) -> Result<()> {
    let path = project_dir.join(SESSION_STORE_FILE);
    let json = serde_json::to_string_pretty(store)?;
    fs::write(&path, json)?;
    Ok(())
}
```

Atomic write (write-to-temp + rename) is unnecessary here because:
- The project lock already prevents concurrent CLI commands.
- Within a single orchestrator run, writes are sequential.
- On corrupt read, the fallback is an empty store (no data loss beyond sessions, which are an optimization).

### Write points (6 total)

All 5 `upsert_session_after_execution` call sites in `orchestrator.rs` (lines 633, 789, 948, 1177, 1425) — add a `persist_session_store(&project_dir, &state.session_store)?;` call immediately after each `upsert_session_after_execution(...)` call. These cover:
1. Initial implementation (line 633)
2. Review-iteration implementation (line 789)
3. QA-iteration implementation (line 948)
4. QA execution (line 1177)
5. Reviewer execution (line 1425)

Plus 1 write point in `handle_prompt_change` (after the session save/restore dance at line 2333) — persist the final state whether sessions were cleared or preserved.

The internal `rollback_current_loop` (line 2273, `state.remove_loop()`) does not need its own persist because every caller of `rollback_current_loop` either:
- Is `handle_prompt_change`, which persists afterward (covered above), or
- Leads to subsequent backend calls that will persist (QA/review limit exceeded cases re-enter the loop).

### Write point in CLI rollback

In `cli/rollback.rs`, after the session invalidation block (after line 156), add:

```rust
persist_session_store(&project_dir, &state.session_store)?;
```

This ensures the CLI rollback command (which runs in a separate process) writes the invalidated store to disk.

### Read point

In `lifecycle.rs:reconstruct_project_state_internal`, after `state` is fully constructed (line 323, just before `Ok(state)`), add:

```rust
let session_path = project_dir.join(SESSION_STORE_FILE);
if let Ok(json) = fs::read_to_string(&session_path) {
    if let Ok(store) = serde_json::from_str::<SessionStore>(&json) {
        state.session_store = store;
    }
}
```

The double-`if let` pattern provides graceful degradation: missing file → empty store, corrupt JSON → empty store.

### Interaction with existing invariants

- **Bootstrap hash staleness**: Already handled. `resolve_session_for_role` compares the stored `bootstrap_hash` against the freshly computed one. If prompt/spec/template changed between runs, the hash won't match, and the stored session is ignored (fresh call).
- **Project lock**: CLI rollback already acquires `ProjectLock` before modifying state. The orchestrator doesn't hold a project lock, but it's the only writer during its run. No new locking needed.
- **`ProjectState::remove_loop`**: Calls `session_store.remove_for_loop()` in memory. The caller is responsible for persisting afterward — this is the existing pattern (analogous to how loop artifact directories are removed but state isn't serialized to disk).

## Files & Modules

| File | Change |
|------|--------|
| `src/project/lifecycle.rs` | Add `SESSION_STORE_FILE` constant. Add `pub fn persist_session_store(...)`. Load `session-store.json` in `reconstruct_project_state_internal` (~5 lines at line 323). |
| `src/workflow/orchestrator.rs` | Import `persist_session_store`. Add persist call after each of the 5 `upsert_session_after_execution` sites (lines 641, 789, 948, 1177, 1425). Add persist call after session restore in `handle_prompt_change` (line 2333). |
| `src/cli/rollback.rs` | Import `persist_session_store`. Add persist call after session invalidation block (after line 156). |
| `src/project/state.rs` | No changes needed. `SessionStore` and `SessionRecord` already derive `Serialize`/`Deserialize`. |
| `src/validate/tests_sessions.rs` | Add 3 new E2E tests (see Testing Strategy). |
| `src/project/lifecycle.rs` (tests module) | Add 2 unit tests for `persist_session_store` and the load path. |

## Testing Strategy

### Unit tests (in `lifecycle.rs` or a dedicated test module)

1. **`persist_and_load_session_store_roundtrip`** — Create a `SessionStore` with records, call `persist_session_store`, then simulate the load logic from `reconstruct_project_state_internal`. Assert records match.
2. **`missing_session_store_file_yields_empty_store`** — Call reconstruct on a project dir with no `session-store.json`. Assert `session_store.records` is empty.
3. **`corrupt_session_store_file_yields_empty_store`** — Write garbage to `session-store.json`, reconstruct, assert empty store (graceful degradation).

### E2E conformance tests (in `tests_sessions.rs`)

4. **`session_persistence_survives_restart`** — Enable session reuse. Run `ralph run --loops 1`. Load state, capture session IDs. Run `ralph run --loops 1` again (second invocation). Load state, assert session records exist with the same `session_id` values from the first run (or at least that `call_count > 1`, proving reuse).
5. **`session_persistence_invalidated_by_rollback`** — Enable session reuse + `reset_on_rollback=true`. Run loop 1. Verify `session-store.json` has records. Run `ralph rollback 0`. Verify `session-store.json` has zero records. Run again, assert fresh sessions.
6. **`session_persistence_invalidated_by_prompt_change`** — Enable session reuse + `reset_on_prompt_change=true`. Run loop 1. Edit prompt. Run again (triggers `RestartLoop`). Verify session records were cleared from `session-store.json`.

### Existing tests remain valid

- All existing `tests_sessions.rs` tests continue to pass without modification — they test in-memory behavior which is unchanged.
- The existing `session_store_serde_roundtrip` unit test already validates the serialization format.

## Out of Scope

- **Atomic/journaled writes**: Unnecessary given the existing project lock and sequential single-writer access pattern. Corrupt files degrade gracefully to empty store.
- **Session store migration/versioning**: The current `SessionRecord` schema is stable. If it changes in the future, a migration can be added then. The `#[serde(default)]` on `SessionStore` already handles missing fields.
- **Compaction/pruning of old session records**: Records are scoped by `(loop_number, role, backend_spec)` and cleaned up on loop removal. No unbounded growth.
- **Encryption or obfuscation of session IDs on disk**: Session IDs are opaque strings managed by the backend. They live in the `.ralph/` directory alongside other project artifacts.
- **Persisting session store on every orchestrator status change**: Only mutation events (backend execution, rollback, prompt change) trigger writes. Read-only status checks do not.
- **Moving the file into loop-specific subdirectories**: The store spans all loops for a project, so it belongs at the project directory level.
- **Daemon-level session caching or cross-project session sharing**: Sessions are strictly per-project.