---
artifact: final-review-proposals
loop: 4
project: task-simplify-ralph-init-to-minimal-scaf
backend: codex
role: final_reviewer
created_at: 2026-02-26T16:11:30Z
---

I’ll audit this by starting with the exact `git diff <base>...HEAD` view and then trace the touched files and tests for correctness/safety gaps, including concurrency and failure-path behavior. After that I’ll run focused checks to validate any suspicious paths and report concrete amendments only where needed.
I confirmed the branch context; next I’m pulling the full `origin/master...HEAD` diff (excluding `.ralph`) and then I’ll drill into the touched files and tests.
I have the changed file list. I’m now reviewing each touched source file (especially `src/config/global.rs` and `src/cli/init.rs`) with line-level checks for failure paths, atomicity, and concurrency safety, then I’ll validate tests actually cover the claimed behavior.
I’m now inspecting the sparse-write implementation and its unit tests in `src/config/global.rs` in detail, including key-path splitting and mutation semantics, because that’s the highest-risk surface for silent config corruption.
I’ve finished the core implementation files; next I’m reviewing conformance/unit test diffs for false positives, missing assertions, and any test logic that could pass for the wrong reason.
I found a potentially subtle persistence issue around `workflow.planner_max_prior_loops = none` and I’m running targeted tests/commands to confirm whether it actually round-trips on disk or silently reverts to the default.
I’ve finished static review and found at least one likely semantic gap; I’m now running broader tests to verify the changed paths and to make sure there aren’t additional regressions hidden by selective assertions.
# Final Review: AMENDMENTS

## Amendment: FR-20260226-01

### Problem
- `workflow.planner_max_prior_loops` accepts `"none"` as unlimited ([src/config/global.rs:1781](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1781)), but sparse save removes that key when `"none"` is used ([src/config/global.rs:1235](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1235), [src/config/global.rs:1216](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1216)).
- Missing key reloads to default `Some(10)` ([src/config/global.rs:914](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:914)), so the command silently does not persist unlimited.
- Reproduced on this branch: `ralph config set --global workflow.planner_max_prior_loops none` then `ralph config get --global workflow.planner_max_prior_loops` returns `10`.
- The table-driven sparse test masks this by save+reload normalizing expected state every case ([src/config/global.rs:3695](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3695), [src/config/global.rs:3697](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3697)).

### Proposed Change
- Make unlimited planner cap round-trip explicitly in TOML instead of key removal:
- Add custom serde for `workflow.planner_max_prior_loops` so `None` is persisted as an explicit sentinel (for example `"none"`) and deserialized back to `None`.
- Remove the `key_becomes_none` removal path for `workflow.planner_max_prior_loops`; write the explicit sentinel value during sparse save.
- Add regression tests:
- Unit: sparse save of `"workflow.planner_max_prior_loops", "none"` reloads as `None`.
- Conformance: `config set --global workflow.planner_max_prior_loops none` persists and `config get` reflects unlimited (not default `10`).
- Tighten the table-driven sparse test so lossy round-trips cannot be hidden by expected `save/load` normalization.

### Affected Files
- [src/config/global.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs) - persist `planner_max_prior_loops = none` correctly and add regression coverage.
- [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/validate/tests_commands.rs) - add CLI conformance test for `none` persistence.

---
