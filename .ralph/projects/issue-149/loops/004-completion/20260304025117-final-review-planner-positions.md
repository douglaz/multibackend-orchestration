---
artifact: final-review-planner-positions
loop: 4
project: issue-149
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-03-04T02:51:17Z
---

Reviewing both amendments against the actual code now. I’ll open the referenced sections in `src/daemon/github.rs` and `src/daemon/runtime.rs`, then confirm whether each claimed defect is real and whether the proposed fix is technically justified.
I confirmed the `github.rs` classifier currently uses `err.to_string()` and defaults unknowns to retry. Next I’m inspecting `runtime.rs` teardown paths to verify the shutdown hang claim and compare behavior with other child-collection paths.
I’ve located `drain_all_children()` and confirmed it does direct `join_handle.await` in the force-kill path. I’m now checking how watcher tasks are implemented to confirm whether an unbounded join can realistically block shutdown and task completion.
# Planner Positions

## Amendment: FR-20260304-01

### Position
ACCEPT

### Rationale
This defect is real.

`is_retryable_push_error` classifies based on the full formatted error (`err.to_string()`), not raw git stderr ([`src/daemon/github.rs:911`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:911), [`src/daemon/github.rs:912`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:912)). Push failures embed branch name into that string ([`src/daemon/github.rs:1007`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:1007)-[`1010`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:1010)), while classifier patterns include numeric tokens like `"403"` and `"500"` ([`src/daemon/github.rs:921`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:921), [`src/daemon/github.rs:933`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:933)). That allows branch-text collisions to affect retryability.

Also, unknown errors currently default to retry (`true`) ([`src/daemon/github.rs:954`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:954)-[`955`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:955)), which can incorrectly delay permanent failures. Existing tests cover only simple literal messages and do not cover branch-collision or unknown-permanent cases ([`src/daemon/github.rs:2212`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:2212)-[`2278`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/github.rs:2278)).

## Amendment: FR-20260304-02

### Position
ACCEPT

### Rationale
This defect is real.

In `drain_all_children()` force-kill teardown, watcher joins are unbounded (`join_handle.await`) ([`src/daemon/runtime.rs:2015`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:2015), [`src/daemon/runtime.rs:2021`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:2021)). By contrast, other teardown paths use `await_watcher_with_timeout(...)` ([`src/daemon/runtime.rs:1801`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:1801), [`src/daemon/runtime.rs:1805`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:1805), [`src/daemon/runtime.rs:1967`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:1967), [`src/daemon/runtime.rs:1971`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:1971)) with explicit timeout+abort behavior ([`src/daemon/runtime.rs:1723`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:1723)-[`1738`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:1738)).

Watcher tasks do perform blocking operations through `spawn_blocking_op`, which has no timeout ([`src/daemon/runtime.rs:85`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:85)-[`93`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:93)); so an unbounded join can hang shutdown. `complete_task(..., "ralph:failed", ...)` is invoked only after those awaits ([`src/daemon/runtime.rs:2026`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:2026)-[`2033`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-149\/src\/daemon\/runtime.rs:2033)), so the claimed delay/skip risk is valid.
