---
artifact: termination-request
loop: 5
project: summary-improve-backend-execution-observ
backend: claude(opus)
role: planner
created_at: 2026-02-19T04:17:47Z
---

# Project Completion Request

## Rationale
All requirements from the master prompt have been fully implemented across three completed loops:

1. **Loop 1 (Error model enrichment)** — Completed: Added `TimeoutKind::{Idle, Walltime}`, enriched `RalphError::BackendTimeout` with `idle_seconds` and `timeout_kind`, updated orchestrator retry logging with all required fields.

2. **Loop 2 (Inactivity timeout semantics)** — Completed: Replaced wall-clock timeouts with inactivity-based timeouts in both non-tmux `execute_streaming` and tmux `wait_for_exit` paths, added stderr capture tracking, added conformance tests in `src/validate/tests_streaming.rs`.

3. **Loop 3 (Claude stream-json arg normalization and NDJSON output parsing)** — Completed: Claude fresh/resume arg paths emit exactly one `--output-format stream-json`, added `normalize_claude_stream_json` with proper NDJSON event handling, implemented auto-detection routing between stream NDJSON and single-object JSON.

The project state confirms Loop 4 was a completion attempt with verdict "Complete", confirming all acceptance criteria are satisfied.

## Summary of Work
- **`src/error.rs`** — `TimeoutKind` enum and enriched `BackendTimeout` variant with `idle_seconds` and `timeout_kind` fields
- **`src/backend/mod.rs`** — Claude arg normalization producing exactly one `--output-format stream-json`, stripping duplicate/conflicting variants, idempotent behavior
- **`src/backend/output_normalizer.rs`** — `normalize_claude_stream_json` for NDJSON parsing with delta text accumulation, session_id/usage extraction, auto-detection routing in `normalize_output`
- **`src/backend/tmux.rs`** / **`src/backend/tmux_backend.rs`** — Inactivity-based timeouts via capture-file size growth, dedicated stderr capture
- **`src/workflow/orchestrator.rs`** — Retry logging with backend, role, attempt, idle_seconds, total_elapsed_secs, timeout_kind
- **`src/validate/tests_streaming.rs`** — Conformance tests for active-stream, hanging-backend, and regression scenarios
- **`src/validate/mod.rs`** — Test registration

## Remaining Items
- None
