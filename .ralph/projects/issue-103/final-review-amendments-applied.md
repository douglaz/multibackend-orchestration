# Final Review Amendments Applied

## Round 1

### Amendment: PRD-LOG-REVIEW-RUNTIME-001

### Problem
`run_review_with_retry_sync` does not persist a debug log entry when Tokio runtime construction fails.  
At [interactive_prd.rs:2196](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2196), the `map_err` path returns an error without calling `logger.log_attempt(...)`, unlike the equivalent failure path in [interactive_prd.rs:2381](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2381).  
Result: a real review-call failure mode can be non-recoverable from disk logs.

### Proposed Change
On runtime build failure in `run_review_with_retry_sync`, emit a best-effort log entry before returning:
- `label = "{label_prefix}-1-of-3"`
- `prompt = original prompt`
- `raw_output = None`
- `error = "failed to create tokio runtime: ..."`
- `validation = ValidationResult::NotChecked`

Keep current workflow/error behavior unchanged after logging.

### Affected Files
- [src/daemon/interactive_prd.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs) - add logging in the runtime-construction error path of `run_review_with_retry_sync`.

---

### Reviewer
codex


## Round 2

### Amendment: AMEND-PRD-LOG-001

### Problem
Synchronous debug logging is currently inside timeout/deadline-critical execution paths, so slow filesystem I/O can change retry/timeout outcomes.

- `run_review_with_retry_sync` wraps review execution in timeout at [`src/daemon/interactive_prd.rs:2235`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2235), but the per-attempt callback does synchronous `logger.log_attempt(...)` at [`src/daemon/interactive_prd.rs:2214`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2214).
- `run_backend_sync` also does synchronous log writes before returning in success/error branches at [`src/daemon/interactive_prd.rs:2412`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2412), [`src/daemon/interactive_prd.rs:2424`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2424), [`src/daemon/interactive_prd.rs:2436`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2436).
- The workflow uses shared absolute deadlines (for example [`src/daemon/interactive_prd.rs:1710`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:1710), [`src/daemon/interactive_prd.rs:1995`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:1995), [`src/daemon/interactive_prd.rs:2299`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs:2299)), so added log-write latency can consume backend budget and alter behavior.

### Proposed Change
Move log persistence off the critical path:

1. In `run_review_with_retry_sync`, collect attempt events in memory during timeout-wrapped execution, then persist logs after timeout/result resolution.
2. In `run_backend_sync`, make log persistence non-blocking (background queue/worker) or otherwise exclude log-write time from backend deadline accounting.
3. Keep current best-effort semantics (`eprintln!` on logging failure, no error propagation).

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/daemon/interactive_prd.rs) - decouple log writes from timeout/deadline paths.

### Reviewer
codex

### Amendment: AMEND-TEST-002

### Problem
The conformance test `review_retry_callback_captures_malformed_attempts` does not validate production wiring in `interactive_prd` and can pass for the wrong reason.

- It directly calls `run_review_with_retry` and manually invokes `logger.log_attempt` in test code at [`src/validate/tests_interactive_prd_logging.rs:184`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/validate/tests_interactive_prd_logging.rs:184), [`src/validate/tests_interactive_prd_logging.rs:195`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/validate/tests_interactive_prd_logging.rs:195).
- It only checks filenames (for example [`src/validate/tests_interactive_prd_logging.rs:232`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/validate/tests_interactive_prd_logging.rs:232)) and does not assert structured fields for malformed attempts (`raw_output`, `error`, `validation.status`).

### Proposed Change
Replace/extend this with a daemon-harness test that exercises the real interactive PRD review path and asserts:

1. Exactly three `draft-review-attempt-{N}-of-3` log entries are created on malformed review output.
2. Each entry has `raw_output` populated, `error == null`, and `validation.status == "review_parse_failed"`.
3. Logging occurs before retry/discard handling (validated by attempt numbering and final failure state).

### Affected Files
- [`src/validate/tests_interactive_prd_logging.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-103/src/validate/tests_interactive_prd_logging.rs) - strengthen assertions to prove production behavior.

### Reviewer
codex


## Round 3

### Amendment: FR-LOG-001

### Problem
`review_retry_per_attempt_timing_guarantee` can pass for the wrong reason.  
The mock backend intentionally exits with `41` when the previous attempt log is missing ([`tests_interactive_prd_logging.rs:382`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs:382), [`tests_interactive_prd_logging.rs:384`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs:384)), but the test only checks label presence/count ([`tests_interactive_prd_logging.rs:548`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs:548)-[`tests_interactive_prd_logging.rs:563`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs:563)).  
On backend error, production code still emits a labeled log entry with `raw_output = None` ([`interactive_prd.rs:2244`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs:2244)-[`interactive_prd.rs:2255`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs:2255)), so the current assertions don’t prove the timing guarantee.

### Proposed Change
Strengthen this test to assert semantic correctness of both entries:
1. `attempt-1` must have `raw_output` present, `error == null`, `validation.status == "review_parse_failed"`.
2. `attempt-2` must have `raw_output` present, `error == null`, `validation.status == "ok"`.
3. Optionally assert stderr does not contain the guard message `"missing prior attempt log"`.

### Affected Files
- [`src/validate/tests_interactive_prd_logging.rs`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs) - tighten assertions in `review_retry_per_attempt_timing_guarantee`.

### Reviewer
codex

### Amendment: FR-LOG-002

### Problem
There is no conformance coverage for the explicit error-schema contract (`raw_output = None`, `error = Some(...)`) on backend transport/runtime failures. Existing tests validate malformed-parse and success flows, but not non-zero/transport failures.

### Proposed Change
Add a conformance test that forces a backend transport/runtime failure (for example, make review attempt exit non-zero), then assert the emitted log entry has:
1. expected label,
2. `raw_output` null,
3. non-empty `error`,
4. `validation.status == "not_checked"`.

### Affected Files
- [`src/validate/tests_interactive_prd_logging.rs`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs) - add a new transport/runtime error-path conformance test and register it in `tests()`.

---

### Reviewer
codex

