# Feature: Persist Backend CLI Output Logs as Loop Artifacts

## Goal
Capture backend CLI output as post-mortem debug artifacts for loop-scoped invocations, without changing orchestration behavior or adding live streaming.

## Problem
When a backend response is malformed, empty, or causes retries, operators currently lack loop-local raw output evidence. This makes troubleshooting parse retries, QA loops, and fast retry sequences difficult.

## In Scope
1. Write one debug log file per backend invocation when process output is available from `wait_with_output()`.
2. Use loop-scoped artifact paths only (`.ralph/projects/<id>/loops/<NNN-slug>/`).
3. Ensure filename uniqueness for rapid same-second retries.
4. Log artifact paths at `info!` level when files are written.
5. Add unit and validate conformance coverage.

## Out of Scope
1. Live streaming or `tail -f` support during execution.
2. Planner output capture before loop slug exists.
3. Prompt-review (project-scoped) output capture.
4. Cross-process/global uniqueness guarantees beyond current process lifetime.

## Requirements

### 1) Execution Context Shape
Use a single optional loop directory in backend execution context:

- `loop_dir: Option<PathBuf>`

Keep existing context fields needed by tmux labeling. `loop_dir` is the source of truth for whether output artifacts should be written.

### 2) Callsite Contract (Must Be Implemented Exactly)

| Callsite | Role | `loop_dir` |
|---|---|---|
| prompt reviewer | `prompt_reviewer` | `None` |
| planner | `planner` | `None` |
| implementer (notes) | `implementer` | `Some(<current loop dir>)` |
| implementer (review response) | `implementer` | `Some(<current loop dir>)` |
| implementer (QA response) | `implementer` | `Some(<current loop dir>)` |
| QA | `qa` | `Some(<current loop dir>)` |
| reviewer | `reviewer` | `Some(<current loop dir>)` |
| completer | `completer` | `Some(<completion loop dir>)` |
| acceptance QA | `qa` | `Some(<completion loop dir>)` |

### 3) Filename and Uniqueness
For each written output artifact, filename must be:

`{YYYYMMDDHHMMSS}-agent-output-{role}-{counter}.log`

Rules:
1. Timestamp is UTC and uses existing `YYYYMMDDHHMMSS` convention (timestamp prefix).
2. `counter` is from `static CLI_OUTPUT_COUNTER: AtomicU64`.
3. Counter guarantees uniqueness for rapid retries within a single process.

### 4) Write Conditions
Write artifact only when all conditions are true:
1. Backend invocation reached `wait_with_output()` and produced an `Output`.
2. Execution context has `loop_dir: Some(...)`.

Do not write artifact when:
1. Spawn fails before process starts.
2. Timeout occurs and no `Output` is returned.
3. `loop_dir` is `None`.

### 5) File Content Contract
Each `.log` file must include both streams in a stable format:

1. Metadata header lines: backend name, role, exit status.
2. `stdout` section (raw captured stdout as lossy UTF-8).
3. `stderr` section (raw captured stderr as lossy UTF-8).

Do not add YAML frontmatter.

### 6) Logging Contract
1. `info!` when a file is successfully written, including full path, backend, role.
2. `debug!` when file write is intentionally skipped (`loop_dir: None`).
3. `debug!` when timeout/spawn-failure results in no output artifact.

### 7) Error Handling
1. Output artifact writing is best-effort.
2. If artifact write fails, emit `warn!` and preserve original backend return behavior.
3. Do not alter retry logic, parse logic, or timeout semantics.

## Technical Approach
1. Add `CLI_OUTPUT_COUNTER: AtomicU64` in `src/backend/mod.rs` (reuse tmux counter pattern).
2. Add helper(s) in `src/backend/mod.rs` to build filename and write artifact.
3. Call helper immediately after `wait_with_output()` returns and before backend result is interpreted.
4. Update orchestrator context-setting callsites to provide `loop_dir` according to the table.
5. Reuse existing timestamp helpers in `src/util/time.rs`.

## Acceptance Criteria
1. Artifact filenames use timestamp prefix and match `*-agent-output-<role>-<N>.log`.
2. Same-second rapid retries do not collide in filenames.
3. Planner invocation writes no output artifact.
4. Prompt-review invocation writes no output artifact.
5. Implementer/QA/reviewer/completer/acceptance-QA invocations write artifacts when output exists.
6. Timeout path writes no artifact and emits `debug!`.
7. Spawn failure path writes no artifact and emits `debug!`.
8. On non-zero exit with captured output, artifact is still written.
9. `info!` log includes artifact path when file is written.
10. File write failures do not change orchestration success/failure outcome.
11. Existing artifact naming conventions for other artifacts remain unchanged.
12. Validate conformance tests are added for this feature.

## Test Plan

### Unit Tests
Add/extend tests around CLI backend execution to verify:
1. Artifact written on successful command execution.
2. Artifact written on non-zero exit when output exists.
3. No artifact when `loop_dir` is `None`.
4. Filename format includes timestamp prefix.
5. Counter uniqueness across rapid invocations.
6. No artifact on timeout.
7. No artifact on spawn failure.

### Validate Conformance Tests
Add tests in `src/validate/tests_run.rs`:
1. End-to-end run creates `agent-output` files for post-planning loop phases.
2. Planner path (no slug/no loop_dir) produces no planner `agent-output` artifact.

## Files to Modify
1. `src/backend/mod.rs`
2. `src/workflow/orchestrator.rs`
3. `src/validate/tests_run.rs`
4. Any directly related unit test file(s) under `tests/` or `src/backend/` as appropriate