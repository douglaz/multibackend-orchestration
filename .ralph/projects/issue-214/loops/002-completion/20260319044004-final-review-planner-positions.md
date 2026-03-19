---
artifact: final-review-planner-positions
loop: 2
project: issue-214
backend: codex(gpt-5.4-xhigh)
role: planner
created_at: 2026-03-19T04:40:04Z
---

Reviewing the oracle review implementation and its validate coverage now. I’m reading the production launcher, state persistence, and the specific validate case to verify whether each amendment describes a real defect.
The implementation confirms the basic shapes in the amendments, but I’m checking the exact control flow now: how oracle is launched, how state writes happen on disk, and whether the validate test really reaches the `failed to spawn command` branch or just simulates it with stderr text.
I’ve confirmed the validate suite never hits the `node` wrapper branch: the test harness places a plain `oracle` script on `PATH`, so `resolve_oracle_cli_entrypoint()` returns `None` and the production-only `dist/bin/oracle-cli.js` path is unexercised. I’m pulling one more source reference for that harness behavior, then I’ll write the accept/reject positions.
# Planner Positions

## Amendment: ORACLE-REV-001

### Position
ACCEPT

### Rationale
The code does rely on private launcher internals. [src/daemon/oracle_review.rs:17](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L17) defines a JS wrapper that monkey-patches `commander` to inject a hidden `--system` option, and [src/daemon/oracle_review.rs:341](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L341) only enables that path when the resolved executable canonicalizes to `.../dist/bin/oracle-cli.js`. [src/daemon/oracle_review.rs:390](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L390) through [src/daemon/oracle_review.rs:415](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L415) then always pass `--system` either through that wrapper or directly to `oracle`. The validate suite does not cover this production-only wrapper path: the harness writes a plain temp executable named `oracle` ([src/validate/harness.rs:203](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/harness.rs#L203)), and the oracle-review tests use that mock executable ([src/validate/tests_daemon_oracle_review.rs:343](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L343), [src/validate/tests_daemon_oracle_review.rs:1234](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L1234)), so `resolve_oracle_cli_entrypoint()` will not match `dist/bin/oracle-cli.js`. The brittleness and coverage gap are real.

## Amendment: ORACLE-REV-002

### Position
ACCEPT

### Rationale
`OracleReviewState::save()` uses one fixed temp path, `path.with_extension("json.tmp")`, before renaming it into place ([src/daemon/oracle_review.rs:91](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L91)). That means concurrent writers target the same `state.json.tmp`: one process can overwrite the other’s temp contents, and one rename can remove the temp file before the other rename runs, causing either silent last-writer confusion or a spurious save failure. The phase does call `save()` repeatedly during normal operation ([src/daemon/oracle_review.rs:199](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L199), [src/daemon/oracle_review.rs:270](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L270)), so per-writer temp-file isolation is a real robustness improvement.

## Amendment: ORACLE-REV-003

### Position
ACCEPT

### Rationale
The test name overstates what it proves. `oracle_spawn_failure_isolated` sets `MOCK_ORACLE_FAIL_FIRST_MODE=spawn` ([src/validate/tests_daemon_oracle_review.rs:856](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L856), [src/validate/tests_daemon_oracle_review.rs:886](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L886)), but the mock’s `spawn` branch merely prints `oracle spawn: mock spawn failure` and exits `7` after startup ([src/validate/tests_daemon_oracle_review.rs:1287](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L1287)). The real spawn-failure path is in `run_command_with_timeout()`, where `Command::spawn()` itself returns an error and emits `failed to spawn command: ...` ([src/daemon/process.rs:19](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/process.rs#L19)). The test passes because `classify_oracle_error()` treats either `failed to spawn command` or any message containing `oracle spawn:` as a spawn error ([src/daemon/oracle_review.rs:450](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L450)). So this is a real false-positive coverage gap: it exercises error classification, not an actual spawn failure.
