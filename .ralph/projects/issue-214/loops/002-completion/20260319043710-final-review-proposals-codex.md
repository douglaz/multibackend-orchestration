---
artifact: final-review-proposals
loop: 2
project: issue-214
backend: codex
role: final_reviewer
created_at: 2026-03-19T04:37:10Z
---

Reviewing the completed change by tracing the actual source and tests. I’m starting with the git delta and key daemon/config files to look for correctness, failure handling, and test gaps before deciding whether amendments are needed.
I have the branch topology now. Next I’m diffing the branch against `master` and then reading the daemon/config/test files touched by the change to check whether the implementation and its tests actually enforce the intended behavior.
The change is concentrated where expected: runtime/config wiring, GitHub helpers, a new `oracle_review` module, and a very large validate module. I’m reading those paths in detail now, especially the new phase’s state/comment/oracle execution paths and whether the tests really prove the claimed edge cases.
I’ve read the core phase. The implementation mostly follows the intended ordering, but I’m now drilling into the tests because the new module includes a lot of behavior hidden behind wrappers and mocks, and that’s the easiest place for real gaps or false confidence to slip through.
I have enough static context to start validation. I’m running a targeted test slice now to see whether the new unit coverage actually executes and to catch any behavioral issues that aren’t obvious from reading alone.
There’s one integration area I can’t validate from local code alone: whether the external `oracle` CLI actually accepts the flags and launch shape this code assumes. I’m checking the upstream primary source for that now, because that directly affects whether the daemon phase works outside the mocks.
I’ve narrowed this to two high-signal problems: one real runtime fragility in how `oracle` is launched, and one conformance test that doesn’t actually exercise the failure path it claims. I’m pulling exact line references now so the amendments are concrete.
# Final Review: AMENDMENTS

## Amendment: ORACLE-REV-001 [P1]

### Problem
The production launcher in [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L17) and [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L341) depends on private `@steipete/oracle` internals instead of its documented CLI surface. It monkey-patches `commander`, requires the resolved binary to canonicalize to `dist/bin/oracle-cli.js`, then always passes `--system` at [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L407). The official oracle README only documents `--prompt`, `--file`, and `--write-output`, not `--system` or this wrapper entrypoint: <https://github.com/steipete/oracle>.

That means a normal upstream CLI update can break oracle review even when `oracle` itself still works from the shell. The current validate coverage does not exercise this wrapper path, so this brittleness is untested.

### Proposed Change
Stop relying on hidden `--system` injection and private package layout. Invoke `oracle` only through its supported flags, folding the review instruction into the prompt text if necessary, or switch to another upstream-supported interface after confirming it exists. Add a focused test for the exact production invocation path that remains.

### Affected Files
- `src/daemon/oracle_review.rs` - replace the private wrapper/`--system` path with a supported oracle invocation
- `src/validate/tests_daemon_oracle_review.rs` - add coverage for the real production launch shape

## Amendment: ORACLE-REV-002 [P2]

### Problem
`OracleReviewState::save()` writes to a fixed temp path, `state.json.tmp`, before renaming it at [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L91). If two daemon processes hit this workspace at once, they race on the same temp file: one writer can overwrite the other’s temp contents or remove it before the second rename, producing lost updates or spurious save failures. This is shared mutable state in the new phase, so the temp file needs per-writer isolation.

### Proposed Change
Write to a unique temp file in the same directory for each save, then atomically rename it into place. `tempfile::NamedTempFile::new_in(parent)` or a unique suffix based on PID/timestamp is sufficient. Add a unit test that at least verifies unique temp naming behavior or concurrent saves do not collide.

### Affected Files
- `src/daemon/oracle_review.rs` - use unique temp files for atomic state writes and add regression coverage

## Amendment: ORACLE-REV-003 [P3]

### Problem
The validate case named `oracle_spawn_failure_isolated` does not actually exercise a spawn failure. The test at [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L856) drives the mock with `MOCK_ORACLE_FAIL_FIRST_MODE=spawn`, but the mock implementation at [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L1287) still successfully starts the process and then exits `7` after printing `oracle spawn: ...`. The test passes because the classifier keys off stderr text, not because the real spawn-failure path was taken.

### Proposed Change
Make this test trigger an actual spawn error for the first PR, or rename it to what it really covers. If the intended contract is “spawn failure on one PR does not abort later PRs,” the test should force `run_command_with_timeout()` to return `failed to spawn command` on the first invocation and still verify the second PR is processed.

### Affected Files
- `src/validate/tests_daemon_oracle_review.rs` - replace the fake “spawn” mode with a real spawn failure or rename/re-scope the test

---

## Context Provided

Reviewed `git diff a8a8b72c03e42dc9bf028163468c15673660e235...HEAD -- . ':(exclude).ralph'` and traced the new daemon phase across runtime integration, GitHub helpers, config wiring, and the added unit/validate coverage.

## Master Prompt

Audited the completed `oracle_review_phase` implementation for correctness, safety, failure isolation, shared-state handling, and whether the tests prove the behaviors they claim.

## Summary

The main config/runtime wiring is in place and the targeted unit tests for `parse_open_prs` and `OracleReviewState` pass, but three amendments are still needed. The highest-risk issue is that the oracle launcher depends on unsupported upstream CLI internals; the new state writer also uses a non-isolated temp file; and one validate test is a false positive because it never reaches a real spawn-failure path.
