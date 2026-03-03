# Final Review Amendments Applied

## Round 1

### Amendment: AM1

### Problem
`20260303T023119Z-impl-notes.md` is a scratch/notes artifact committed at repo root and does not belong in the source tree.

### Proposed Change
Remove the file from the repository.

### Affected Files
- `20260303T023119Z-impl-notes.md`

### Reviewer
claude

### Amendment: AM2

### Problem
`plan_actions()` in `src/cli/init.rs` is now an obsolete thin wrapper around `plan_full_actions()` and has no call sites, with an explicit `#[allow(dead_code)]`.

### Proposed Change
Delete the `plan_actions()` function and its `#[allow(dead_code)]` attribute.

### Affected Files
- `src/cli/init.rs`

### Reviewer
claude

### Amendment: FR-001

### Problem
`save_sparse()` has unsafe table-path traversal for valid TOML that uses inline tables.

At [src/config/global.rs:1260](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1260), [src/config/global.rs:1268](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1268), and [src/config/global.rs:1270](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1270), `ensure_tables()` assumes each segment is a standard table and panics otherwise.  
At [src/config/global.rs:1188](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1188) and [src/config/global.rs:1254](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1254), optional-key removal silently no-ops when the path is inline-table-backed.

This causes:
1. Crash (`exit 101`) on normal `config set` for inline-table configs.
2. False success on `null` clears where disk state is unchanged.

### Proposed Change
Make sparse path navigation fully non-panicking and inline-table-aware:
1. Change `ensure_tables()`/`navigate_tables_mut()` to return `Result<...>` (or explicit error state), not panic.
2. When encountering an inline table at a path segment, convert it to a mutable normal table (preserving existing key-values) before descending.
3. If a path segment is a non-table scalar/array, return a validation/orchestration error instead of panicking or silently succeeding.
4. Add regression tests for:
   - set on inline-table path (must not panic, value persisted),
   - clear (`null`) on inline-table path (must remove key from disk).

### Affected Files
- [src/config/global.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs) - robust sparse path handling and error propagation
- [src/validate/tests_commands.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/validate/tests_commands.rs) - CLI-level regression coverage for inline-table configs

### Reviewer
codex

### Amendment: FR-002

### Problem
`init --copy-files` overlay merge skips nested default insertion when an existing key is represented as an inline table.

At [src/cli/init.rs:298](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/cli/init.rs:298) and [src/cli/init.rs:305](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/cli/init.rs:305), `merge_tables()` only recurses for `Item::Table`, not inline tables. So keys like `workspace = { default_backend = "codex" }` do not get missing nested defaults inserted under `workspace`.

Also, the conformance test named “fills missing keys” does not prove on-disk key insertion: [src/validate/tests_init.rs:338](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/validate/tests_init.rs:338) only asserts deserialized effective config equality, which can pass via serde defaults even when keys were never merged into TOML.

### Proposed Change
1. Update overlay merge to handle inline-table existing nodes (convert to table then recurse, or equivalent inline-table merge).
2. Add a unit test in `init.rs` covering inline-table input and verifying missing nested defaults are physically added.
3. Strengthen `copy_files_overlay_fills_missing_keys` to assert key presence in file content (not only deserialized equality).

### Affected Files
- [src/cli/init.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/cli/init.rs) - inline-table-aware overlay merge
- [src/validate/tests_init.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/validate/tests_init.rs) - assertions that actually verify “fills missing keys”

### Reviewer
codex

### Amendment: FR-003

### Problem
A stray implementation artifact was added at repo root: [20260303T023119Z-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/20260303T023119Z-impl-notes.md). This is outside runtime/test source scope and should not ship as product code content.

### Proposed Change
Remove the file from the tracked changes (or move it under `.ralph` loop artifacts if it is intentionally archival metadata).

### Affected Files
- [20260303T023119Z-impl-notes.md](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/20260303T023119Z-impl-notes.md)

---

## Context Provided
Reviewed `git diff origin/master...HEAD -- . ':(exclude).ralph'`, audited changed source/test files, and manually reproduced runtime behavior for sparse writes and overlay merge edge cases.

## Master Prompt
The amendments focus on correctness and safety invariants over nominal conformance: no panics on valid user configs, no false-success config writes, and tests that verify claimed behavior on disk rather than only deserialized defaults.

### Reviewer
codex

