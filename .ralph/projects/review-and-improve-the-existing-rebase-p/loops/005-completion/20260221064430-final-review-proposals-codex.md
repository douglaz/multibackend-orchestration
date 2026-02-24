---
artifact: final-review-proposals
loop: 5
project: review-and-improve-the-existing-rebase-p
backend: codex
role: final_reviewer
created_at: 2026-02-21T06:44:30Z
---

# Final Review: AMENDMENTS

## Amendment: AFCR-20260221-01-timeout-bounded-conflict-classification

### Problem
`execute_rebase` enforces a shared deadline for fetch/rebase/push, but conflict classification currently calls `classify_rebase_failure(...)`, which internally runs `git::has_conflicts(...)` without timeout. That creates an unbounded subprocess step after rebase failure, which violates the timeout-budget requirement ("no step may run without bounded timeout").

### Proposed Change
Use remaining deadline budget before conflict classification and perform conflict detection with a timeout-bounded git status call. Concretely:
- Compute remaining budget in `execute_rebase` before classification.
- Replace unbounded `git::has_conflicts(...)` in this path with `git::has_conflicts_with_timeout(...)`.
- If no budget remains, return a timeout error immediately.
- Keep unit coverage for classification criteria by splitting pure criteria logic from I/O-bound conflict probing as needed.

### Affected Files
- `src/daemon/runtime.rs` - apply bounded-time conflict classification in `execute_rebase`.
- `src/daemon/rebase_agent.rs` - refactor classifier helper shape if needed for pure-vs-I/O split and tests.
- `src/daemon/rebase_agent.rs` - add/adjust unit tests for timeout-bounded classification path.

## Amendment: AFCR-20260221-02-none-backend-normalization-and-error-actionability

### Problem
`execute_rebase` checks disabled mode with raw string equality (`agent_backend == "none"`). Backend parsing elsewhere trims and normalizes input, so values like `" none "` are accepted at config validation but miss the disabled fast path here. Also, some failure messages (notably `RebaseContinueFailed`) do not explicitly indicate that the rebase agent path was attempted/skipped, reducing actionability versus the prompt's error-message constraint.

### Proposed Change
Normalize backend handling in `execute_rebase` by parsing once with `parse_rebase_agent_backend` and branching on `RebaseAgentBackend` instead of raw string equality. Then standardize conflict-failure error messages to always state whether agent resolution was attempted or skipped (disabled).
- Disabled (`None`) path: explicitly report agent skipped/disabled while preserving existing failure flow.
- Agent-attempted path: ensure `--continue` failure messages retain that agent resolution had been attempted.

### Affected Files
- `src/daemon/runtime.rs` - parse backend enum once and branch on typed backend; standardize disabled-path error messaging.
- `src/daemon/rebase_agent.rs` - improve `AgentError::RebaseContinueFailed` mapping/message clarity.
- `src/daemon/rebase_agent.rs` - add tests for normalized `"none"` handling and actionable attempted/skipped messaging.
