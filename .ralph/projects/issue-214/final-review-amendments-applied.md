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


## Round 3

### Amendment: ORACLE-REV-FINAL-001

### Problem
The bot-marker dedup check is substring-based instead of exact-marker-based. In [src/daemon/github.rs#L2131](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2131), `find_bot_comment_with_marker_with_gh_bin` matches any bot-authored comment whose body `contains(marker)`. The oracle phase then trusts that result in [src/daemon/oracle_review.rs#L159](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L159) and persists the PR/SHA as reviewed.

That means a bot-authored comment that merely embeds the marker text anywhere in its body, instead of actually being the oracle review comment for that `(pr_number, head_sha)`, will incorrectly suppress oracle execution and advance dedup state. The current conformance coverage in [src/validate/tests_daemon_oracle_review.rs#L669](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs#L669) only tests the happy path where the marker is the actual leading marker line, so this false-positive case is unguarded.

### Proposed Change
Require an exact marker-line match for oracle review idempotency, not substring containment. The safest fix is to add an exact-match helper for this phase that accepts only comments whose first line is exactly the marker, or whose body starts with `{marker}\n`, and use that helper for both the pre-oracle existence check and the post-comment outcome path. Add unit and validate coverage for a bot-authored comment that contains the marker later in the body and verify oracle still runs.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs) - tighten or specialize marker matching semantics
- [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs) - use the exact-marker matcher for oracle dedup/state healing
- [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs) - add conformance coverage for embedded-marker false positives

---

## Context Provided
I reviewed the branch diff against `origin/master...HEAD`, then traced the implementation through [src/daemon/runtime.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/runtime.rs), [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs), [src/daemon/github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs), [src/config/global.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/config/global.rs), [src/config/mod.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/config/mod.rs), [src/cli/config.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/cli/config.rs), [src/cli/daemon.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/cli/daemon.rs), and [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs).

I also ran targeted checks: `cargo test parse_open_prs`, `cargo test oracle_review_state`, `cargo test post_bot_comment_outcome_treats_readback_failure_after_post_as_success`, and targeted validate cases for `eligible_pr_reviewed`, `missing_oracle_binary_does_not_advance_state`, and `comment_readback_failure_still_advances_state`.

## Master Prompt
Audit the completed oracle-review daemon phase for correctness, safety, robustness, resource isolation, persistence behavior, and test validity, using the committed source as the source of truth.

## Summary
One amendment is needed: oracle marker dedup currently accepts substring matches, which can falsely suppress reviews and self-heal state on the wrong comment. Aside from that, the runtime integration, config wiring, persistence ordering, failure isolation, and the exercised unit/validate paths looked sound.

### Reviewer
codex


## Round 4

### Amendment: ORACLE-REV-FR-002

### Problem
The oracle subprocess has a timeout, but all of the `gh` subprocesses used by the phase are unbounded. [`list_open_non_draft_prs`](//tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L241), [`fetch_pr_diff`](//tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L274), [`fetch_issue_comments_with_gh_bin`](//tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L1932), and [`fetch_authenticated_login_with_gh_bin`](//tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs#L2015) all use `Command::output().await` without any deadline. The phase then awaits those calls inline and sequentially in [`src/daemon/oracle_review.rs:112`](//tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs#L112).

If `gh pr diff` or `gh issue view --json comments` hangs, one PR can stall the entire oracle-review phase indefinitely, preventing later eligible PRs from being processed and blocking the poll loop from advancing. That breaks the intended non-fatal/per-PR-isolated behavior under real operational failures.

### Proposed Change
Add bounded execution for oracle-review `gh` calls. Reuse the existing timeout/kill behavior via `process::run_command_with_timeout` under `spawn_blocking`, or introduce an equivalent async helper that enforces a deadline and kills the child process group. The phase should log a stable timeout warning and continue to the next PR when comment lookup or diff fetch times out.

### Affected Files
- [src/daemon/github.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/github.rs) - add timeout-bounded execution for oracle-review-related `gh` helpers
- [src/daemon/oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/daemon/oracle_review.rs) - handle `gh` timeout failures as per-PR warnings and continue
- [src/validate/tests_daemon_oracle_review.rs](/tmp/ralph-daemon-orch/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-214/src/validate/tests_daemon_oracle_review.rs) - add coverage for hung `gh` comment/diff calls not wedging the cycle

---

## Context Provided
Reviewed the branch diff against the `master` merge-base, then traced the changed daemon/config/GitHub helper paths and the new validate module.

## Master Prompt
Add an independent `oracle_review_phase` that reviews open non-draft PRs with `oracle`, dedups by `(pr_number, head_sha)`, persists state, posts idempotent top-level PR comments, and remains non-fatal inside the daemon poll loop.

## Summary
I found two concrete robustness issues. The larger one is repo-isolation: the new persisted state is shared across all repos in a workspace even though the daemon can run multiple repo runtimes concurrently. The second is that a hung `gh` subprocess can still wedge the phase even though `oracle` itself is timeout-bounded. Focused unit tests I ran for `parse_open_prs`, `OracleReviewState`, and the new post-comment outcome helper passed, but they do not cover either of these failure modes.

### Reviewer
codex

