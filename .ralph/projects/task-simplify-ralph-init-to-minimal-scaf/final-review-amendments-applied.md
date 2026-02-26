# Final Review Amendments Applied

## Round 1

### Amendment: RALPH-FR-001

### Problem
Sparse global config edits are incorrect when `ralph.toml` uses inline tables.

In [src/config/global.rs:1283](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1283), path traversal treats any non-`Table` as replaceable and in [src/config/global.rs:1285](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1285) replaces it with an empty table. If the user has `workflow = { ... }`, setting one `workflow.*` key drops sibling keys.

In [src/config/global.rs:1309](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1309), removals only traverse `Item::as_table_mut`, so clearing optional values (`null` / `none`) inside inline tables becomes a silent no-op.

Current sparse-save tests (for example [src/config/global.rs:3229](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3229)) don’t cover inline-table inputs, so this regression is untested.

### Proposed Change
Update sparse mutation/removal traversal to support inline tables without data loss:
1. Preserve existing inline-table entries when descending path segments.
2. Allow key removal from inline tables.
3. Add unit tests that use inline-table configs and assert:
1. sibling keys are preserved after `set`,
2. optional keys are actually removed after `null`/`none`.

### Affected Files
- [src/config/global.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs) - fix sparse traversal/removal for inline tables and add inline-table coverage tests.

---

### Reviewer
codex


## Round 2

### Amendment: FR-20260226-01

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

### Reviewer
codex

