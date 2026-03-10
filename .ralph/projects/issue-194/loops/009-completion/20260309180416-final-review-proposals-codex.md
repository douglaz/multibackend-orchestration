---
artifact: final-review-proposals
loop: 9
project: issue-194
backend: codex
role: final_reviewer
created_at: 2026-03-09T18:04:16Z
---

Reviewing the full change set for correctness and safety now. I’ll start by identifying the base commit, run the required diff command, then trace critical code paths and tests around queueing, orchestration boundaries, and failure handling.
I found the branch state and will now diff against the merge base with `origin/master` to capture the full project changes (excluding `.ralph`) before drilling into high-risk files and tests.
I captured the full changed file list. Next I’m tracing core logic in `src/project/amendments.rs`, orchestrator integrations, and validate tests to verify crash-safety, rollback behavior, and completion-guard correctness.
I’ve finished the first pass through queue/core logic and am now checking the rest of `amendments.rs` tests/helpers plus orchestrator call sites for race/error-path behavior and whether rollback can duplicate or drop requests.
I’m now validating the quick-dev path and shared state transitions to ensure the new drain/rollback hooks don’t introduce duplicate processing or persistence races.
I’ve traced the main code paths and will now run focused tests (`amendments`, orchestrator/quick-dev conformance slices, and CLI integration) to verify the behaviors match the code-level assumptions and catch hidden regressions.
The shell image doesn’t have `cargo` directly, so I’m rerunning the checks through `nix develop -c` to match the project’s build/test environment.
Unit and integration tests pass so far; next I’m running the amendment conformance suite through `ralph validate` to verify end-to-end CLI/orchestrator behavior under the harness.
`nix build` is still running through the static release build; once it finishes I’ll immediately run the `validate` filter for amendment conformance tests.
The `nix build` pipeline is running the full conformance suite (not just compile), which is useful for this audit; I’m waiting for the final pass/fail summary before concluding findings.
I’ve confirmed functional behavior and test coverage; I’m doing one final pass for out-of-scope/stray artifacts and subtle data-loss edge cases before issuing the amendment verdict.
# Final Review: AMENDMENTS

## Amendment: AMEND-QUEUE-LOSS-001 [P1]

### Problem
`drain_amendment_queue_with_hook` can delete already-processed queue items and still return `Err` on a later file operation, which creates a loss path for drained amendments.  
Key points:
- It processes files incrementally and deletes each parsed inflight file ([src/project/amendments.rs:239](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs:239)).
- Any later `?`-propagated IO error aborts the whole drain ([src/project/amendments.rs:168](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs:168)).
- Callers treat drain failure as fatal and cannot rollback because they never receive the partial drained vector ([src/workflow/orchestrator.rs:604](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:604), [src/workflow/quick_dev_orchestrator.rs:347](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs:347)).

### Proposed Change
Make drain failure non-lossy:
1. On fatal mid-drain error, best-effort re-enqueue already drained items before returning `Err`.
2. Add a unit test that injects a mid-drain failure and asserts no amendment disappears.

### Affected Files
- `src/project/amendments.rs` - add internal rollback-on-error behavior in drain path and test coverage.

## Amendment: AMEND-TEST-SEMANTICS-002 [P2]

### Problem
The conformance test `quick_dev_checkpoint_failure_no_rollback_after_durable_success` does not actually assert that the checkpoint failure path occurred; it ignores command status (`let _output = ...`) and only checks queue emptiness ([src/validate/tests_amendments.rs:736](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs:736), [src/validate/tests_amendments.rs:775](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs:775)).  
That means the test can pass even when no checkpoint failure happened, so the name/claim is stronger than what it proves.

### Proposed Change
Make the test prove the intended path:
1. Assert non-zero run result and checkpoint/commit failure evidence in stderr, or
2. If deterministic failure cannot be guaranteed, rename the test to reflect current semantics and add a separate deterministic failure-path test.

### Affected Files
- `src/validate/tests_amendments.rs` - tighten assertions (or rename + split test).

---
