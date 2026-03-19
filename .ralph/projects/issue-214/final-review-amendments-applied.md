# Final Review Amendments Applied

## Round 1

### Amendment: ORACLE-REV-001

### Problem
The production launcher in [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L17) and [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L341) depends on private `@steipete/oracle` internals instead of its documented CLI surface. It monkey-patches `commander`, requires the resolved binary to canonicalize to `dist/bin/oracle-cli.js`, then always passes `--system` at [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L407). The official oracle README only documents `--prompt`, `--file`, and `--write-output`, not `--system` or this wrapper entrypoint: <https://github.com/steipete/oracle>.

That means a normal upstream CLI update can break oracle review even when `oracle` itself still works from the shell. The current validate coverage does not exercise this wrapper path, so this brittleness is untested.

### Proposed Change
Stop relying on hidden `--system` injection and private package layout. Invoke `oracle` only through its supported flags, folding the review instruction into the prompt text if necessary, or switch to another upstream-supported interface after confirming it exists. Add a focused test for the exact production invocation path that remains.

### Affected Files
- `src/daemon/oracle_review.rs` - replace the private wrapper/`--system` path with a supported oracle invocation
- `src/validate/tests_daemon_oracle_review.rs` - add coverage for the real production launch shape

### Reviewer
codex

### Amendment: ORACLE-REV-002

### Problem
`OracleReviewState::save()` writes to a fixed temp path, `state.json.tmp`, before renaming it at [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L91). If two daemon processes hit this workspace at once, they race on the same temp file: one writer can overwrite the other’s temp contents or remove it before the second rename, producing lost updates or spurious save failures. This is shared mutable state in the new phase, so the temp file needs per-writer isolation.

### Proposed Change
Write to a unique temp file in the same directory for each save, then atomically rename it into place. `tempfile::NamedTempFile::new_in(parent)` or a unique suffix based on PID/timestamp is sufficient. Add a unit test that at least verifies unique temp naming behavior or concurrent saves do not collide.

### Affected Files
- `src/daemon/oracle_review.rs` - use unique temp files for atomic state writes and add regression coverage

### Reviewer
codex

### Amendment: ORACLE-REV-003

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

### Reviewer
codex


## Round 2

### Amendment: ORACLE-REVIEW-FR-001

### Problem
The oracle phase treats the entire `post_bot_comment_with_marker_with_gh_bin(...)` call as the success boundary, and only increments `success_count` plus persists state on `Ok(_)` ([src/daemon/oracle_review.rs#L229](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L229)). That helper, however, returns `Err` not only when `gh issue comment` fails, but also when the follow-up comment readback fails after a successful post ([src/daemon/github.rs#L2208](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2208), [src/daemon/github.rs#L2230](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2230), [src/daemon/github.rs#L2257](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2257), [src/daemon/github.rs#L1923](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L1923)).

That creates a real misclassification path: the review comment is already on GitHub, but the daemon logs `comment post failed`, does not save `(pr_number, head_sha)`, and does not count the posted review toward `daemon_oracle_review_max_per_cycle`. In a transient readback failure, one cycle can therefore post more comments than the configured cap and the next cycle has to self-heal via marker scan instead of having correct state immediately.

### Proposed Change
Make the success boundary match the actual post operation, not the metadata readback.

A concrete fix is:
1. Split bot-comment posting into distinct outcomes: `already_exists`, `posted`, and `post_failed`.
2. Treat a zero-exit `gh issue comment` as `posted` even if the optional fetch-back of comment metadata fails afterward.
3. In `oracle_review_phase`, advance state and increment `success_count` on `posted`, skip counting `already_exists`, and reserve `comment post failed` for genuine comment-command failures.
4. Add a validate case where `issue comment` succeeds but the subsequent `issue view --json comments` call fails, asserting that state still advances and the per-cycle cap still reflects the already-posted review.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs) - separate successful post semantics from best-effort readback.
- [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs) - count and persist successful posts correctly.
- [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs) - add a conformance test for post-success/readback-failure behavior.

---

## Context Provided
Reviewed `git diff a8a8b72c03e42dc9bf028163468c15673660e235...HEAD -- . ':(exclude).ralph'`, traced the new oracle-review flow and helper contracts, and ran:
- `nix develop -c cargo test oracle_review -- --nocapture`
- `nix develop -c cargo test parse_open_prs -- --nocapture`
- `nix develop -c cargo run -- validate --bin target/debug/ralph --filter daemon_oracle_review --verbose`

I also spot-checked the upstream Oracle CLI contract and verified the integration flags used here are documented: https://github.com/steipete/oracle

## Master Prompt
The review focused on correctness, failure isolation, persisted dedup state, comment idempotency, cap enforcement, and whether the new phase stays non-fatal inside the daemon poll loop.

## Summary
One amendment is required. The new phase’s state/cap accounting is tied to comment readback rather than the actual GitHub post, which makes transient readback failures behave like false negatives and breaks the intended per-cycle limits.

### Reviewer
codex

