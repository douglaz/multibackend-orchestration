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

