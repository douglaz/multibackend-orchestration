---
artifact: prompt-review
project: summary-improve-backend-execution-observ
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-18T22:07:56Z
---

# Prompt Review

## Issues Found
- The conformance timing example is self-contradictory (`sleep timeout+2s` with `timeout+1s` idle limit cannot succeed), so the expected result is unclear and non-testable.
- Stream-format detection is underspecified: it alternates between “starts with `{`” and “first valid JSON line has `type`”, which can route inputs inconsistently.
- `session_id` extraction source is ambiguous (`message.id` vs true session/conversation ID), which risks storing the wrong identifier.
- Tmux stderr handling changes output plumbing but does not clearly state whether downstream normalization should consume stdout only or combined streams, risking behavior regressions.
- Timeout semantics do not explicitly define race/cancellation behavior when process exits near timeout, which can produce flaky failures.
- `TimeoutKind` introduces `Walltime` but no current producer is specified; without explicit “Idle only for this change,” implementations may diverge.
- Validate test integration step is missing in the original testing plan (registering `tests_streaming` in `src/validate/mod.rs`), so tests may never run.
- Logging requirements are partially specified but not fully normalized (field names/units), reducing observability consistency.

## Refined Prompt
### Title
Improve backend observability and timeout resilience (Claude stream-json + inactivity timeouts)

### Objective
Implement four coordinated changes:
1. Default Claude backend invocations to `--output-format stream-json`.
2. Replace wall-clock timeout behavior with inactivity-based timeout behavior in both non-tmux and tmux execution paths.
3. Extend Claude output normalization to parse streaming NDJSON (`stream-json`) safely and completely.
4. Enrich `RalphError::BackendTimeout` with idle-duration context and timeout kind for diagnostics.

### Scope
In scope:
- `src/backend/mod.rs`
- `src/backend/output_normalizer.rs`
- `src/backend/tmux.rs`
- `src/backend/tmux_backend.rs`
- `src/error.rs`
- `src/workflow/orchestrator.rs`
- `src/validate/tests_streaming.rs`
- `src/validate/mod.rs` (test registration)

Out of scope:
- New config keys (do not add `max_walltime`)
- Config migration (keep `timeout_seconds` key unchanged)
- Backend spec syntax changes
- Codex arg-format changes (existing Codex JSON args unchanged)
- Structural changes to `BackendTimeoutExhausted`

### Required Behavior

#### 1) Claude arg normalization (`stream-json`)
- For Claude fresh calls (`ensure_json_output_args` path), always emit exactly one pair: `--output-format stream-json`.
- For Claude resumed calls (`effective_args_claude` path), also emit exactly one pair: `--output-format stream-json`.
- Strip all existing output-format variants before appending:
  - `--output-format <value>`
  - `--output-format=<value>`
- Behavior must be idempotent (running normalization twice still leaves one pair).

#### 2) Claude stream NDJSON normalization
- Add `normalize_claude_stream_json(raw: &str) -> Result<NormalizedOutput>`.
- Detection in `normalize_output`:
  - Parse lines in order until the first valid JSON object is found.
  - If that object has a `"type"` field, route to stream NDJSON normalizer.
  - Otherwise route to existing single-object Claude JSON normalizer.
  - If no valid JSON objects exist, keep raw-text fallback behavior.
- Stream event handling:
  - `message_start`: extract `session_id` from `message.id` (for this change, this is the canonical source).
  - `content_block_delta`: append `delta.text` (concatenate in arrival order).
  - `message_delta` and/or summary usage events: extract `tokens_in`, `tokens_out`, `cached_in`.
  - `content_block_start`, `content_block_stop`, `message_stop`, `ping`: ignore gracefully.
  - Unknown `type`: ignore (forward-compatible).
  - Malformed/non-JSON lines: skip, do not fail immediately.
- Return behavior:
  - If at least one JSON event parsed and text was accumulated: return normalized text + extracted metadata.
  - If at least one JSON event parsed but no text accumulated: return `RalphError::ParseError(...)`.
  - If zero JSON events parsed: return raw text fallback (existing behavior).

#### 3) Inactivity timeout semantics (non-tmux `execute_streaming`)
- `timeout_seconds` now means max idle duration, not total wall-clock duration.
- Activity is any successful read of `>0` bytes from stdout or stderr.
- Watchdog checks idle time periodically (1s interval is acceptable).
- Timeout condition: `idle_duration >= timeout_seconds`.
- On timeout:
  - mark timed-out state
  - kill process group
  - produce `RalphError::BackendTimeout { timeout_kind: Idle, idle_seconds, ... }`
- Ensure watchdog cancellation on normal process exit to avoid race-induced false timeouts.

#### 4) Inactivity timeout semantics (tmux path)
- Update `wait_for_exit` to receive both stdout and stderr capture paths.
- Track activity by file-size growth on either capture file.
- Timeout based on idle duration since last observed growth.
- Add dedicated stderr capture file in tmux backend command wiring.
- Persist stderr artifact separately while preserving existing stdout artifact behavior for normalization path.

#### 5) Error model and logging
- Add:
  - `TimeoutKind::{Idle, Walltime}`
  - `RalphError::BackendTimeout { backend, idle_seconds: u64, timeout_kind: TimeoutKind }`
- For this feature, all newly produced backend timeouts must use `TimeoutKind::Idle`.
- Update orchestrator retry logging to include:
  - `backend`
  - `role`
  - `attempt`
  - `idle_seconds`
  - `total_elapsed_secs`
  - `timeout_kind`

### Acceptance Criteria
- Claude fresh/resume arg paths each output exactly one `--output-format stream-json`.
- Duplicate or conflicting output-format args are removed before append.
- Stream NDJSON text deltas concatenate correctly (not last-wins).
- `session_id`, `tokens_in`, `tokens_out`, `cached_in` are extracted when present.
- Detection correctly distinguishes NDJSON stream vs single-object JSON.
- Existing single-object JSON and raw-text fallback behavior remains passing.
- Non-tmux backend times out only on inactivity, not on active streaming.
- Tmux backend uses same inactivity semantics via capture-file growth checks.
- `BackendTimeout` includes idle context and timeout kind; retry warn log includes required fields.
- No new config key; no config migration.
- At least one Codex conformance test verifies shared inactivity-timeout behavior.

### Test Plan

#### Unit tests
- `src/backend/output_normalizer.rs`
  - accumulates multiple `content_block_delta` entries
  - extracts `session_id` from `message_start.message.id`
  - extracts usage fields from `message_delta`/summary
  - errors when JSON events exist but no text deltas
  - skips unknown event types and malformed lines
  - preserves single-object JSON behavior
  - validates detection routing logic
- `src/backend/mod.rs`
  - Claude arg normalization produces exactly one `--output-format stream-json`
  - stripping handles both flag syntaxes
  - idempotence checks
- `src/error.rs` (or equivalent)
  - `BackendTimeout` display/debug includes `idle_seconds` and `timeout_kind`

#### Conformance tests
Create `src/validate/tests_streaming.rs` with:
- Active-stream test: backend emits output periodically at intervals `< timeout_seconds`; total runtime `> timeout_seconds`; expect success (no timeout).
- Hanging Codex test: emits partial output then stalls `> timeout_seconds`; expect timeout, cleanup/kill, and partial output retention.
- Regression: existing timeout cleanup scenario still passes under inactivity semantics.

Register tests in `src/validate/mod.rs`:
- `mod tests_streaming;`
- `tests.extend(tests_streaming::tests());`

### Implementation Notes
- Prefer monotonic timing primitives for idle measurement.
- Keep behavior backward-compatible except for intentional timeout semantic shift from wall-clock to inactivity.
- Do not introduce unnecessary new dependencies.

### Definition of Done
- Code changes complete across listed files.
- Unit and conformance tests added and passing.
- Existing relevant tests pass without regressions.
- `nix develop -c cargo test` and validate suite for streaming/timeout paths pass.
