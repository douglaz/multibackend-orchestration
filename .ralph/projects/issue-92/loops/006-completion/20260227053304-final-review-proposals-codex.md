---
artifact: final-review-proposals
loop: 6
project: issue-92
backend: codex
role: final_reviewer
created_at: 2026-02-27T05:33:04Z
---

I’m auditing the current branch as requested, starting with git history to identify the correct base branch and then inspecting `git diff <base>...HEAD -- . ':(exclude).ralph'` before reviewing the changed source and tests for correctness/safety issues.
I’ve identified `origin/master` as the likely base. Next I’m running the exact triple-dot diff against `HEAD` (excluding `.ralph`) and then drilling into each changed file plus related tests.
I have the full diff now. I’m inspecting `src/daemon/interactive_prd.rs` around each changed transition path to verify label ordering, failure behavior, and whether any edge paths skip persistence or mutate labels unsafely.
I’m now tracing the full control flow (`poll_and_advance_prd` + `finish_transition`) to confirm panic/error handling still persists failure state correctly and to check that the new waiting-label calls don’t introduce cross-worker interference or ordering regressions.
I’ve confirmed only three source files changed and there are no stray modifications outside that scope. Next I’m running targeted tests for `interactive_prd` behavior to validate the changed paths and catch any regressions not obvious from static inspection.
I’m checking the high-risk terminal transition tests now, especially partial-failure cases, to verify whether assertions genuinely validate state durability and label behavior rather than passing on incidental side effects.
I’m finishing the conformance review now: checking terminal-save-failure and dedup tests to ensure they assert the intended component behavior directly, not just proxy signals that could pass under incorrect implementations.
I’ve validated unit/integration tests; next I’m running the conformance suite filtered to `interactive_prd` against the built debug binary to verify the new lifecycle behavior under the harnessed daemon flow.
# Final Review: AMENDMENTS

## Amendment: FR-PRD-WAITING-POSTSAVE-001

### Problem
`Done` can be durably saved even when terminal label cleanup fails, which can leave stale `ralph:prd-active` and `ralph:waiting-feedback` labels indefinitely.

In [interactive_prd.rs:1505](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1505), removal of `ralph:prd-active` is fallible (`?`). If it fails, execution returns before the new waiting-label removal at [interactive_prd.rs:1518](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1518).  
Then `finish_transition` persists state again on error paths at [interactive_prd.rs:1675](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1675) without reverting `Done` unless save itself fails. This makes the issue terminal in state storage, so subsequent ticks skip transition logic and cleanup is never retried.

### Proposed Change
Make post-save `Done` cleanup failures retryable instead of terminally committed:

1. In `finish_transition`, when `result.is_err()` and `state.state == Done`, revert to `pre_transition_state` before saving so retries remain possible.
2. In `do_approval_transition`, attempt waiting-label removal even if active-label removal fails (collect cleanup errors, don’t short-circuit before trying both).
3. Add tests for the exact case “save succeeds, `--remove-label ralph:prd-active` fails” to prove state remains non-terminal and cleanup retries on next tick.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs` - fix `Done` error-path state handling and cleanup sequencing.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs` - add integration coverage for post-save active-label removal failure.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs` - add conformance coverage for the same failure mode.

---
