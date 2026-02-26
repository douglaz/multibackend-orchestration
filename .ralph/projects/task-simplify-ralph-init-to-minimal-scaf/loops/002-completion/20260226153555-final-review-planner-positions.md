---
artifact: final-review-planner-positions
loop: 2
project: task-simplify-ralph-init-to-minimal-scaf
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-02-26T15:35:55Z
---

Reviewing `src/config/global.rs` around the cited line ranges first to confirm how sparse set/remove traversals handle inline tables, then I’ll check the related tests for coverage gaps.
I confirmed the key traversal helpers currently use `Item::is_table` / `Item::as_table_mut`; next I’m checking the sparse-save call flow and null-removal path to verify this affects real behavior, not dead code.
I’m now checking the nearby sparse-save tests for nested keys to see whether any inline-table case already exercises this path.
I found the `toml_edit` crate source locally; I’m checking `Item::is_table` and `Item::as_table_mut` semantics to confirm whether inline tables are excluded as suspected.
# Planner Positions

## Amendment: RALPH-FR-001

### Position
ACCEPT

### Rationale
`save_config_sparse` applies edits through `set_document_key` and removals through `remove_document_key` ([src/config/global.rs#L1202](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L1202), [src/config/global.rs#L1271](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L1271), [src/config/global.rs#L1295](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L1295)).

In `ensure_table_path`, any path segment that is not `Item::Table` is replaced with a fresh empty table ([src/config/global.rs#L1283](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L1283), [src/config/global.rs#L1285](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L1285)). For inline-table parents (for example `workflow = { ... }`), this causes sibling-key data loss during `set`.

`remove_from_table` only descends via `Item::as_table_mut` ([src/config/global.rs#L1309](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L1309)), so inline-table parents cannot be traversed and `null`/`none` removals become silent no-ops.

Existing sparse-save tests do not cover inline-table inputs; they use standard table forms/default saves ([src/config/global.rs#L3229](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L3229), [src/config/global.rs#L3390](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs#L3390)). This is a real correctness and data-preservation defect, and the proposed fix/test direction has technical merit.
