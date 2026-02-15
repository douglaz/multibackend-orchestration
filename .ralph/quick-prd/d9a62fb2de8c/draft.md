## Summary

Persist stdout and stderr from non-tmux (`CliBackend`) backend invocations to timestamped log files in the loop artifacts directory. Currently, `CliBackend::execute()` (`src/backend/mod.rs:150-196`) captures stdout/stderr in memory via `Stdio::piped()` and discards both after the call completes. This makes post-mortem debugging of agent failures impossible. The tmux path already captures stdout to a file via `tee`; this feature brings comparable observability to the non-tmux path.

The approach generalizes the existing `SharedTmuxContext` / `TmuxExecutionContext` into a `SharedBackendContext` / `BackendExecutionContext` that carries a pre-resolved `loop_dir` path and `role`. The context uses a direct `loop_dir: Option<PathBuf>` rather than separate `project_dir` + `loop_slug` fields, so callers that already know the loop directory (all orchestrator callsites post-planning) can set it directly, while callers where no loop directory exists yet (planner, prompt reviewer) simply leave it `None`. When `loop_dir` and `role` are populated, `CliBackend::execute()` writes stdout and stderr to files in that directory after the process completes and logs the file paths. When the context fields are absent, behavior is unchanged.

Output files are **post-mortem debug artifacts** — they are written after `wait_with_output()` returns, not streamed during execution. Live `tail -f` during execution is explicitly out of scope for this version.

## Acceptance Criteria

- [ ] Non-tmux `CliBackend::execute()` writes stdout to `<loop_dir>/<YYYYMMDDHHMMSS>-agent-output-<role>-<N>.log` when execution context has `loop_dir` and `role` set (where `<N>` is a monotonic invocation counter for uniqueness)
- [ ] stderr captured separately to `<loop_dir>/<YYYYMMDDHHMMSS>-agent-output-<role>-<N>.stderr`
- [ ] File names use timestamp as prefix, consistent with existing artifact naming convention (`<YYYYMMDDHHMMSS>-<descriptor>`)
- [ ] File paths logged at `info!` level when files are written (e.g., `"agent output saved to <path>"`) so they appear in normal operator-visible logs
- [ ] Existing output parsing (return value of `execute()`) is byte-for-byte identical — stdout is still returned as `String`
- [ ] Output files are written on both success and failure (non-zero exit) — the error is returned to the caller after writing
- [ ] On timeout (`BackendTimeout`), no output files are written (buffered output is unavailable); a `debug!` message notes that output was not captured due to timeout
- [ ] On spawn failure, no output files are written (no process output exists)
- [ ] All existing tests pass; no behavioral change when execution context `loop_dir` is `None` (PRD pipeline, gap analysis, daemon refine, quick PRD, planner phase before loop dir creation)
- [ ] Log files persist after invocation (no automatic cleanup in v1)
- [ ] Tmux mode behavior is completely unchanged
- [ ] Conformance test in `src/validate/` verifies output files are created for non-tmux backend invocations

## Technical Approach

### 1. Generalize the execution context

Rename `TmuxExecutionContext` → `BackendExecutionContext` and add a pre-resolved `loop_dir` field:

```rust
// src/backend/tmux_backend.rs (kept here alongside TmuxBackend which is the primary consumer)
#[derive(Debug, Clone, Default)]
pub struct BackendExecutionContext {
    pub loop_number: Option<u32>,
    pub role: Option<String>,
    pub loop_dir: Option<PathBuf>,
}
```

The `loop_dir` field holds the fully-resolved path (e.g., `{project_dir}/loops/001-user-auth/`). This avoids the need for `project_dir` + `loop_slug` separately and sidesteps the problem of the slug being unavailable during planner execution — the orchestrator simply sets `loop_dir: None` for the planner callsite and `loop_dir: Some(...)` for all post-planning callsites where the directory already exists.

Rename `SharedTmuxContext` → `SharedBackendContext` (same `Arc<Mutex<...>>` pattern). Rename `set_tmux_context` → `set_backend_context` on `BackendRegistry`. Update all callsites.

The existing `TmuxBackend` continues to read `loop_number` and `role` from the same context — no behavioral change.

### 2. Thread the context into `CliBackend`

Add an `Option<SharedBackendContext>` field to `CliBackend` (set at construction time). In `backend_with_optional_tmux()` (`mod.rs:459-475`), pass the shared context to `CliBackend` in the non-tmux branch as well (currently only `TmuxBackend` receives it):

```rust
fn backend_with_optional_tmux(
    mut backend: CliBackend,
    tmux: &BackendRegistryTmuxConfig,
    shared_ctx: SharedBackendContext,
) -> Arc<dyn Backend> {
    if tmux.enabled {
        Arc::new(TmuxBackend::new(backend, ..., shared_ctx))
    } else {
        backend.set_shared_context(shared_ctx);
        Arc::new(backend)
    }
}
```

### 3. Add monotonic invocation counter

Reuse the existing `AtomicU64` pattern from `tmux_backend.rs:14` (`INVOCATION_COUNTER`). Add a second static counter (or share the existing one) for `CliBackend` output file naming:

```rust
static CLI_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);
```

Each call to `execute()` that writes output files increments this counter. The counter value is appended to the filename, guaranteeing uniqueness even when multiple invocations for the same role occur within the same second (e.g., parse retries, QA iterations, timeout retries with fast backoff).

### 4. Write output files in `CliBackend::execute()`

After `child.wait_with_output()` completes (both success and failure paths) in `CliBackend::execute()`, if the context contains a valid `loop_dir` + `role`:

1. Snapshot the context via `self.shared_context.get().await`
2. Increment `CLI_OUTPUT_COUNTER`
3. Generate timestamp via `now_timestamp_yyyymmddhhmmss()`
4. Build file paths:
   - `{loop_dir}/{timestamp}-agent-output-{role}-{counter}.log`
   - `{loop_dir}/{timestamp}-agent-output-{role}-{counter}.stderr`
5. Write stdout bytes to `.log`, stderr bytes to `.stderr`
6. Log both paths at `info!` level: `"agent output saved to {path}"`

The writes are fire-and-forget: failures emit `warn!` but do not propagate as errors to the caller. The existing return value (`Ok(stdout_string)` or `Err(BackendCommandFailed)`) is unchanged.

**Restructuring the error path:** Currently, non-zero exit returns early at line 189-193 before any file writing could occur. The revised flow captures the `Output` struct, unconditionally writes files if context is present, and *then* checks `output.status.success()` to decide the return value. This ensures files are written for failed invocations — the most valuable debugging scenario.

### 5. Timeout and spawn failure behavior

**Timeout:** When `tokio::time::timeout()` fires, `wait_with_output()` never returns, so there is no buffered output to write. The timeout error path will log at `debug!` level: `"backend timed out; no output captured to disk"`. No files are written. (Streaming capture that would preserve partial output on timeout is a follow-up.)

**Spawn failure:** The process never started, so there is no output. No files are written. No special handling needed beyond the existing `BackendCommandFailed` error.

### 6. Orchestrator callsite updates

Update all ~9 `set_tmux_context(TmuxExecutionContext { ... })` calls in `orchestrator.rs` to `set_backend_context(BackendExecutionContext { ... })` with the additional `loop_dir` field:

| Callsite | `loop_dir` value |
|---|---|
| Prompt reviewer (line ~250) | `None` (no loop context) |
| Planner (line ~371) | `None` (loop dir does not exist yet — slug is determined by planner output) |
| Implementer initial (line ~512) | `Some(project_dir.join("loops").join(format!("{:03}-{}", loop_number, slug)))` |
| Implementer QA response (line ~609) | Same as above |
| Implementer prompt change (line ~710) | Same as above |
| QA (line ~867) | Same as above |
| Reviewer (line ~1057) | Same as above |
| Completer (line ~1289) | Same as above (slug is `"completion"`) |
| QA in acceptance (line ~1376) | Same as above |

The `loop_dir` is built from `project_dir`, `loop_number`, and `slug` which are all available at these callsites. A helper like `fn loop_dir_path(project_dir: &Path, loop_number: u32, slug: &str) -> PathBuf` keeps this DRY.

### 7. Cleanup strategy (v1: keep all)

No automatic cleanup. Log files accumulate alongside existing artifacts in the loop directory. They are small relative to the markdown artifacts already stored there. Future work can add retention policies.

## Files & Modules

| File | Change |
|---|---|
| `src/backend/tmux_backend.rs` | Rename `TmuxExecutionContext` → `BackendExecutionContext`. Add `loop_dir: Option<PathBuf>` field. Update `build_label()` and all internal references. No logic changes to `TmuxBackend` — it continues reading `loop_number` and `role` as before. |
| `src/backend/mod.rs` | Rename `SharedTmuxContext` → `SharedBackendContext`. Add `Option<SharedBackendContext>` field and `set_shared_context()` method to `CliBackend`. Add `static CLI_OUTPUT_COUNTER: AtomicU64`. Add output-file writing logic to `CliBackend::execute()` (after `wait_with_output`, before success/failure branching). Update `backend_with_optional_tmux()` to pass context to both tmux and non-tmux branches. Rename `set_tmux_context` → `set_backend_context` on `BackendRegistry`. |
| `src/workflow/orchestrator.rs` | Update all ~9 `set_tmux_context(TmuxExecutionContext { ... })` calls to `set_backend_context(BackendExecutionContext { ... })` with `loop_dir` populated where available (all post-planning callsites) and `None` where unavailable (planner, prompt reviewer). Add `loop_dir_path()` helper or inline the path construction. |
| `src/validate/tests_run.rs` | Add conformance test `run::non_tmux_backend_writes_output_logs` that runs a backend invocation with `tmux = false`, then asserts `.log` and `.stderr` files exist in the loop directory with expected naming pattern and content. |
| `src/validate/mod.rs` | Register new conformance test(s) from `tests_run.rs`. |
| `src/util/time.rs` | No changes (already provides `now_timestamp_yyyymmddhhmmss()`). |
| `src/project/artifacts.rs` | No changes (loop dir structure is reused but not modified). |

## Testing Strategy

### Unit tests (in `src/backend/mod.rs` `#[cfg(test)]`)

1. **Output files written on success** — Create a `CliBackend` backed by `echo "hello"` with a populated `SharedBackendContext` (loop_dir pointing at a `tempdir`, role = `"impl"`). Call `execute()`. Assert:
   - A `.log` file exists in the loop dir subdirectory
   - File content equals stdout returned by `execute()`
   - A `.stderr` file exists (may be empty)
   - File names match `{14-digit-timestamp}-agent-output-impl-{N}.log` / `.stderr` pattern

2. **Output files written on failure (non-zero exit)** — Backend command: `sh -c "echo stdout-data; echo stderr-data >&2; exit 1"`. Call `execute()`, expect error. Assert:
   - `.log` file contains `"stdout-data\n"`
   - `.stderr` file contains `"stderr-data\n"`
   - Files are written despite the error

3. **No files when context loop_dir is None** — Create a `CliBackend` with `SharedBackendContext` where `loop_dir = None` (simulating planner/PRD paths). Call `execute()`. Assert no files written to any directory.

4. **No files when shared context is absent** — Create a `CliBackend` without a shared context at all. Call `execute()`. Assert no files written.

5. **Filename uniqueness across rapid invocations** — Call `execute()` twice in quick succession (same role, same loop_dir). Assert two distinct pairs of files exist (different counter values, possibly same timestamp).

6. **Timestamp-prefix format** — Assert file names start with a 14-digit timestamp prefix followed by `-agent-output-`, consistent with the artifact convention in `artifacts.rs`.

7. **Timeout produces no output files** — Create a `CliBackend` with a very short timeout and a command that sleeps. Call `execute()`, expect `BackendTimeout`. Assert no `.log` or `.stderr` files exist.

### Existing test suite

8. **`cargo test`** — Run full unit + integration test suite to confirm no regressions. `TmuxBackend` tests use `MockTmuxRunner` and don't exercise `CliBackend::execute()` directly, so they are unaffected by the rename.

### Conformance test (in `src/validate/tests_run.rs`)

9. **`run::non_tmux_backend_writes_output_logs`** — End-to-end black-box test using `RalphHarness`:
   - Initialize workspace, create project with a mock ralph backend (tmux disabled)
   - Trigger a backend invocation via `ralph run`
   - Assert loop directory contains files matching `*-agent-output-*.log` and `*-agent-output-*.stderr`
   - Assert file content is non-empty (contains expected mock backend output)
   - Assert ralph's stderr/stdout logs contain `"agent output saved to"` at operator-visible level

10. **`run::non_tmux_backend_no_output_logs_for_planner`** — Verify that the planner invocation (where `loop_dir` is `None`) does NOT produce output log files, confirming the opt-in behavior.

### Manual testing

11. Run `ralph` with `tmux = false`, trigger a full feature loop (planner → implementer → QA → reviewer), confirm:
    - No output files for planner invocation
    - Output files appear for implementer, QA, and reviewer invocations
    - Files contain the expected agent stdout/stderr
    - File paths appear in ralph's logs

## Out of Scope

- **Streaming capture / `tail -f` support** — `CliBackend` currently uses `wait_with_output()` which buffers all output in memory. Streaming to a file during execution would require refactoring to `tokio::io::copy` with a tee into both a `Vec<u8>` and a `File`, which is a larger change. Output files are post-mortem debug artifacts in v1. Streaming is deferred to a follow-up.
- **Partial output capture on timeout** — When a timeout occurs, `wait_with_output()` never returns and no buffered output is available. Capturing partial output would require the streaming refactor above. Deferred.
- **Planner phase output capture** — The planner executes before the loop directory exists (the slug is determined by planner output). Capturing planner output would require either a temporary staging path or a two-phase write. Deferred — planner failures are already surfaced via the orchestrator error chain.
- **PRD pipeline / gap analysis / daemon refine output capture** — These callsites don't set loop context. Could be added later by threading context through those modules.
- **Automatic log rotation / cleanup** — v1 keeps all files. Retention policies are future work.
- **Tmux mode changes** — Already captures stdout via `tee`. stderr visibility in tmux is handled by the terminal pane. No changes needed.
- **Prompt input logging** — The prompt is already captured in artifact files. Not duplicated here.
- **Structured output format** — Log files are raw text, not JSON or frontmatter-wrapped. They are debug artifacts, not orchestration artifacts.
