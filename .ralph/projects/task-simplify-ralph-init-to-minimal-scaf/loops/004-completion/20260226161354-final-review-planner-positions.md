---
artifact: final-review-planner-positions
loop: 4
project: task-simplify-ralph-init-to-minimal-scaf
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-02-26T16:13:54Z
---

I’m verifying amendment `FR-20260226-01` directly against the source, starting with `src/config/global.rs` and the related validate tests to confirm whether `workflow.planner_max_prior_loops=none` is actually lossy on save/reload.
I confirmed the sparse save path has a special removal branch for `planner_max_prior_loops`. Next I’m checking the parser/default behavior and the existing tests to see whether this creates a real lossy round-trip.
I’ve confirmed no conformance test currently covers `planner_max_prior_loops=none`. I’m now checking the CLI config set/get implementation path to verify this can surface exactly as described (`set --global ... none` then `get --global`).
The code path strongly indicates a lossy round-trip already, but I’m running a direct CLI reproduction in a temp workspace to confirm runtime behavior (`config set --global ... none` then `config get --global`).
# Planner Positions

## Amendment: FR-20260226-01

### Position
ACCEPT

### Rationale
This amendment identifies a real correctness bug in global config persistence.

- `planner_max_prior_loops` defaults to `Some(10)` when absent ([src/config/global.rs:379](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:379), [src/config/global.rs:914](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:914)).
- `"none"` is accepted and mapped to `None` in memory ([src/config/global.rs:1572](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1572), [src/config/global.rs:1781](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1781)).
- Sparse save deletes this key when value is `"none"` ([src/config/global.rs:1216](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1216), [src/config/global.rs:1235](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1235)).
- `config set --global` uses sparse save and then reloads config from disk ([src/cli/config.rs:309](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/cli/config.rs:309), [src/cli/config.rs:312](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/cli/config.rs:312)), so removed key reloads as default `10`.
- I verified behavior with the built binary: `config set --global workflow.planner_max_prior_loops none` followed by `config get --global workflow.planner_max_prior_loops` returns `10`.
- The table-driven sparse test masks this because expected state is re-saved/reloaded each iteration ([src/config/global.rs:3634](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3634), [src/config/global.rs:3695](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3695), [src/config/global.rs:3697](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3697)).
