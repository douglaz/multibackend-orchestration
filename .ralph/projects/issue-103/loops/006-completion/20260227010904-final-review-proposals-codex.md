---
artifact: final-review-proposals
loop: 6
project: issue-103
backend: codex
role: final_reviewer
created_at: 2026-02-27T01:09:04Z
---

Reviewing the implementation as a code audit, starting by identifying the base branch and collecting the full source diff with `.ralph` excluded. Then I’ll inspect changed files and tests for correctness, safety, and robustness issues.
I’ve confirmed this branch tracks `origin/master`, so I’m using `master` as the base for the audit diff and then drilling into each changed file.
I have the main patch list and I’m now reading the full modified source files (not truncated diff) to verify behavior on success/error paths and review test adequacy.
Next I’m deep-reading `src/daemon/interactive_prd.rs`, since that’s where most error-path and logging behavior changed; I’ll trace each backend call path and timeout/error branch.
I’m now tracing the sync wrappers (`run_backend_sync`, `run_review_with_retry_sync`) line-by-line to verify that every failure mode logs exactly once and that retry behavior didn’t regress.
I’ve confirmed the core implementation paths; next I’m auditing the new/updated tests in `interactive_prd.rs` and `validate/tests_interactive_prd_logging.rs` for false positives and missing assertions.
I’m now validating the conformance test module for assertion quality and whether it can pass while missing real regressions.
I’ve finished static inspection of changed call sites; I’m running targeted tests next to validate the new retry-callback and logging paths behave as expected in practice.
I’m doing one final pass for scope hygiene (`git status` excluding `.ralph`) and then I’ll issue the amendment list with precise file/line citations.
The new validate cases are registered; I’m running the `interactive_prd_logging` conformance subset to verify they execute end-to-end, not just compile.
# Final Review: AMENDMENTS

## Amendment: AMEND-PRD-LOG-001

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

## Amendment: AMEND-TEST-002

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
