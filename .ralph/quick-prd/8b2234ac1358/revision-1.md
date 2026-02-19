The revised specification has been written. Here's how each review issue was addressed:

### Review Issue 1 — Scope alignment
The Summary now explicitly acknowledges that real-time streaming is already implemented and frames this spec as a **separate, related gap** discovered during streaming verification. The framing is honest: streaming works, the timeout model is what needs fixing to make streaming useful for long-running backends.

### Review Issue 2 — Acceptance criteria clarity
Removed all references to `ralph tail` from acceptance criteria. The criteria now reference only `tail -f` on `.log` files, which is what actually works today. The Out of Scope section explains that `ralph tail` follows `.md` artifacts and `state.json` by design (`src/cli/tail.rs`), not `.log` files.

### Review Issue 3 — Behavioral compatibility
Replaced the unconditional idle-timeout-only model with a **dual-limit** design:
- **Idle timeout** = `timeout_seconds` (resets on each chunk)
- **Absolute cap** = `10 * timeout_seconds` (hard wall-clock limit, never resets)

This prevents indefinitely-running chatty processes. The spec includes an explicit compatibility note explaining the semantic change and its impact on existing configurations. A dedicated `max_duration_seconds` config field is noted as a natural follow-up but explicitly out of scope.

### Review Issue 4 — Edge-case race handling
The `tokio::select!` uses `biased;` with explicit priority ordering: cancellation > absolute cap > activity > idle sleep. The spec explains exactly how this prevents false `BackendTimeout` results when activity arrives simultaneously with the idle timer expiring (the activity branch is checked first). Also added explanation that SIGKILL to a dead PID is harmless (returns `ESRCH`).

### Review Issue 5 — Testing coverage
Added three new items:
- **`cli_backend_stderr_activity_resets_idle_timeout`** — unit test verifying stderr-only activity resets the idle timer
- **`streaming::idle_timeout_active_backend`** — conformance test in `src/validate/tests_streaming.rs` for end-to-end idle-timeout behavior
- **`idle_timeout_active_mock_script()`** — new mock script helper in `src/validate/mock_scripts.rs`

Also switched from `tokio_util::CancellationToken` to a second `Arc<Notify>`, avoiding a new dependency since `tokio-util` is not in `Cargo.toml`.