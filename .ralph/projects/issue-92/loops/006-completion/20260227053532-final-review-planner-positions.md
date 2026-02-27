---
artifact: final-review-planner-positions
loop: 6
project: issue-92
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-02-27T05:35:32Z
---

I’m going to verify this amendment directly against the code paths in `interactive_prd.rs` and the current tests, then decide accept/reject strictly on correctness impact. I’ll inspect the cited lines and the surrounding transition/save error handling first.
I’ve confirmed the cited lines exist. Next I’m tracing the call flow into `do_approval_transition` and `finish_transition` to verify whether a failed `remove-label ralph:prd-active` can leave a persisted terminal `Done` that never retries cleanup.
I found the core flow: `do_approval_transition` runs before `finish_transition`, so I’m now checking whether `finish_transition`’s `pre_transition_state` is truly pre-transition or already mutated to `Done`. That determines whether retries are possible after cleanup failure.
I confirmed there are tests for add-`prd-done` failure and save-failure paths, but I still need to verify the specific post-save failure mode (`remove-label ralph:prd-active` fails). I’m checking polling/terminal behavior to confirm whether cleanup is retried later or permanently skipped.
I’ve confirmed terminal states short-circuit in `advance_issue`. I’m now verifying runtime-level filtering to see if stale `prd-active`/`waiting-feedback` labels remain indefinitely or get reconciled elsewhere.
I verified there’s no existing test that forces `--remove-label ralph:prd-active` to fail after a successful `Done` save. I’ll inspect the GitHub label helper behavior next, then finalize the technical verdict.
# Planner Positions

## Amendment: FR-PRD-WAITING-POSTSAVE-001

### Position
ACCEPT

### Rationale
This is a real correctness/robustness defect in current code.

- `Done` is persisted before cleanup: `do_approval_transition` sets `state.state = Done` and saves, then performs label cleanup afterward ([interactive_prd.rs:1487](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1487), [interactive_prd.rs:1494](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1494), [interactive_prd.rs:1505](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1505)).
- Active-label removal is fallible and short-circuits; waiting-label removal is never attempted if active removal fails ([interactive_prd.rs:1505](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1505), [interactive_prd.rs:1518](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1518)).
- The error then flows through `finish_transition`; on error it saves state again, and only reverts terminal state on save-failure. So with successful save, terminal `Done` remains persisted despite cleanup failure ([interactive_prd.rs:1309](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1309), [interactive_prd.rs:1678](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1678), [interactive_prd.rs:1686](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1686)).
- Subsequent ticks skip transition/cleanup for terminal states, so cleanup is not retried ([interactive_prd.rs:932](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:932)). Meanwhile polling still includes `ralph:prd-active` issues ([interactive_prd.rs:729](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:729)).
- Existing tests cover add-`prd-done` failure and save-failure cases, but not this specific post-save active-label-removal failure path ([tests/daemon_interactive_prd.rs:2044](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs:2044), [tests/daemon_interactive_prd.rs:2197](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs:2197), [src/validate/tests_interactive_prd.rs:2961](//tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:2961)).
