---
artifact: prompt-review
project: restore-real-time-streaming-of-stdout-to
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-19T01:37:37Z
---

# Prompt Review

## Issues Found
- The prompt presents multiple competing implementation strategies (`Mutex<Instant>`, `Notify`, `CancellationToken`) without selecting one, which creates avoidable ambiguity for implementers.
- Cancellation handling is described as problematic but not resolved with a single concrete pattern, which risks incorrect watchdog behavior or hanging tasks.
- Acceptance criteria mentions `ralph tail` while also declaring it out of scope, which is internally inconsistent and hard to test.
- References rely on exact line numbers, which are brittle as code shifts and can mislead downstream loops.
- Timeout race behavior at the activity/expiry boundary is unspecified, which can cause flaky behavior depending on `select!` branch ordering.
- The test plan depends on tight wall-clock timing without explicit anti-flake guidance, which can produce nondeterministic CI failures.
- Dependency guidance is ambiguous (`tokio-util` “if needed”), which can cause unnecessary dependency churn.
- The feature adds runtime behavior but does not require a new/updated validate conformance case, which conflicts with project guidance that new features need validate coverage.

## Refined Prompt
Implement an activity-aware idle timeout for non-tmux `CliBackend` execution, while preserving existing real-time log streaming and output normalization behavior.

### Context
- Real-time stdout streaming to `agent-output-*.log` during `CliBackend` execution is already implemented and must remain unchanged.
- Current timeout behavior is a fixed global duration from process start.
- Required behavior change: timeout should represent inactivity, not total runtime.

### Objective
Replace the fixed global timeout in `CliBackend::execute_streaming` with an idle timeout that resets whenever stdout or stderr receives a chunk.

### In Scope
- `CliBackend` execution path in `src/backend/mod.rs`
- Watchdog timeout logic
- Activity signaling from stdout and stderr read paths
- Unit tests in backend module
- Validate conformance coverage update for streaming timeout behavior

### Out of Scope
- `ralph tail` behavior changes
- `TmuxBackend` streaming model
- New config flags to choose global-timeout vs idle-timeout modes
- New progress event systems

### Required Behavior
1. `agent-output-*.log` continues to grow in real time during `CliBackend` execution.
2. Idle timeout resets on every stdout chunk.
3. Idle timeout resets on every stderr chunk.
4. If no stdout/stderr activity occurs for the configured timeout window, process group is killed with the same semantics as current timeout path.
5. `captured_stdout` accumulation and post-exit normalization (`String::from_utf8_lossy`) remain unchanged.
6. No duplicate log content is introduced.
7. Timeout/watchdog task is reliably cancelled after process/read-loop completion.
8. Existing streaming and timeout behaviors not targeted by this change do not regress.

### Implementation Constraints
1. Use a single approach: `tokio::sync::Notify` for activity reset plus the existing cancellation channel pattern.
2. Keep cancellation explicit and deterministic in watchdog loop.
3. In watchdog `tokio::select!`, prioritize cancellation and activity before timeout expiry (use `biased;`) to reduce boundary race false positives.
4. Do not introduce `tokio-util` unless strictly required by compilation constraints.
5. Keep Unix process-group kill behavior unchanged.

### File Targets
- `src/backend/mod.rs`
- `src/validate/tests_streaming.rs` (or equivalent existing streaming validate module)
- `src/validate/mod.rs` only if additional test registration is needed

### Testing Requirements
1. Existing unit tests pass, including:
- `cli_backend_streaming_preserves_exact_bytes_in_log`
- `cli_backend_timeout_kills_and_reaps_child_and_writes_footer`
2. Existing streaming validate tests pass.
3. Add unit test: `cli_backend_idle_timeout_resets_on_activity`
- Backend emits output periodically (interval less than timeout).
- Total runtime exceeds nominal timeout.
- Expected: process completes successfully (not timed out).
4. Add unit test (or strengthen existing timeout test): idle timeout fires on stall.
- Backend emits initial output then stalls beyond timeout.
- Expected: timeout error and cleanup behavior.
5. Add or extend validate conformance coverage for idle-timeout reset behavior in streaming path.
6. Timing tests must be CI-stable:
- Use generous margins between emit interval and timeout.
- Avoid brittle exact-duration assertions.
- Assert outcome and bounded completion windows.

### Acceptance Criteria
- [ ] Real-time log streaming for `CliBackend` remains intact.
- [ ] Idle timeout is activity-based (stdout or stderr resets timer).
- [ ] Stalled backends are terminated after inactivity window.
- [ ] Output normalization and logging semantics are preserved.
- [ ] No duplicate log writes appear.
- [ ] Existing relevant unit tests pass.
- [ ] Existing relevant validate tests pass.
- [ ] New unit test for slow-but-active survival is added and passing.
- [ ] Validate conformance coverage for the new timeout behavior is added/updated and passing.
