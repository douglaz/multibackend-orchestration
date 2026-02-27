---
artifact: prompt-review
project: task-enable-real-time-streaming-of-backe
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-16T03:34:58Z
---

# Prompt Review

## Issues Found
- The provided text is a status summary, not an execution-ready prompt; it lacks a clear objective, scope boundaries, and explicit deliverables.
- It references “all 9 call sites” but does not identify where they are, which risks partial implementation.
- “Byte-identical to `wait_with_output()`” is ambiguous without defining stdout/stderr handling, so it is hard to validate correctly.
- Retry logging requires separators but does not define a stable separator schema, making attribution brittle and hard to test.
- Timeout handling mentions kill/reap but does not fully specify behavior for streaming I/O failure paths.
- Test descriptions are high-level and do not state concrete assertions or registration requirements, which can leave gaps.

## Refined Prompt
### Goal
Implement deterministic, append-only backend output logging across orchestration roles, with byte-preserving streaming, reliable timeout cleanup, and complete conformance coverage.

### Scope
Update backend execution/streaming paths used by planner, implementer, QA, reviewer, prompt-reviewer, and parse-retry fallback flows. Add unit and conformance tests.

### Required Behavior

1. **Deterministic log paths**
- Add one helper that derives log path from `(project_dir, loop_number, role)`.
- If `loop_number` exists:  
  `{project_dir}/loops/{loop_number:03}/agent-output-{role}.log`
- If `loop_number` is absent and role is `prompt-reviewer`:  
  `{project_dir}/agent-output-prompt-reviewer.log`
- Do not use `loop_slug` or backend name in filenames.
- Ensure parent dirs exist before open.

2. **Retry attribution and append semantics**
- Open log files in create+append mode.
- All attempts for the same `(loop_number, role)` must append to the same file.
- Before each attempt, append a stable separator containing:
  - attempt number
  - backend label (sanitized)
  - fallback flag
  - timestamp
- No overwrite behavior.

3. **Streaming semantics (byte preservation)**
- Replace line-based reads with chunk-based `read_buf()` (e.g., `BytesMut`).
- Preserve exact bytes (`\r`, partial lines, progress output).
- Append raw bytes to in-memory buffer and log sink as received.
- Keep existing final conversion behavior (`String::from_utf8_lossy()` only at the end).
- Acceptance: captured bytes match process-emitted bytes for each stream.

4. **Timeout and process cleanup**
- On timeout: explicitly call `child.kill()` then `child.wait()`.
- Apply same cleanup for fatal stream I/O error paths.
- Write timeout footer to log before returning timeout error.
- Return existing timeout error type (`BackendTimeout`).
- Ensure no zombie child remains.

5. **Path safety and logging failure policy**
- Add `sanitize_for_filename()` for backend labels used in headers/separators.
- Log filenames remain role-derived safe ASCII.
- Log I/O is best-effort:
  - on open/write/flush failure: `tracing::warn!`
  - disable further log writes for that run
  - continue execution without changing backend result semantics

### Test Requirements

1. **Conformance tests**
- Add `src/validate/tests_streaming.rs`.
- Register in `src/validate/mod.rs`.
- Include at least:
  - `mid_execution_visibility` (log grows while process is running)
  - `retry_append_behavior` (single file append across retries with separators)
  - `timeout_cleanup` (timeout footer + partial output + process dead)
  - `prompt_reviewer_path` (root-level prompt-reviewer log path)

2. **Unit tests**
- CR/partial-line preservation with chunked reads.
- `sanitize_for_filename()` behavior for unsafe chars.
- Log write/open failure resilience (warn + continue).
- Child kill/reap verification on timeout/error path.

### Non-Goals
- No change to artifact naming, loop slug generation, backend selection policy, or CLI flags.
- No behavioral changes beyond logging and stream-capture correctness.

### Acceptance Criteria
- `cargo test` passes.
- Validate suite passes including new streaming tests.
- No code path derives log filenames from `loop_slug`.
- Retries/fallbacks are attributable in a single append-only file per `(loop_number, role)`.
- Timeout paths always kill and reap child processes.
- Stream capture preserves raw bytes prior to final lossy string conversion.
