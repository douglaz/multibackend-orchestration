---
artifact: final-review-proposals
loop: 3
project: issue-90
backend: codex
role: final_reviewer
created_at: 2026-03-03T03:09:44Z
---

Reviewing the implementation by inspecting the actual git diff and then tracing key code paths and tests for correctness, safety, and robustness. I’ll start by finding the base branch and collecting the scoped diff excluding `.ralph`, then audit the modified files and corresponding tests.
I confirmed the working branch is `ralph/issue-90` ahead of `origin/master`; next I’m reviewing `git diff origin/master...HEAD -- . ':(exclude).ralph'` and then drilling into touched files and tests.
I’ve collected the changed file list; now I’m tracing the new init/config code paths and test updates (`copy_files`, sparse TOML writes, overlay merge) to identify correctness gaps.
I’m now validating the changed tests against the behavior claims and checking for coverage gaps where tests may pass for the wrong reason.
I’ve identified a few potential edge-case risks in `save_sparse` and I’m now running targeted tests to confirm behavior (especially dotted env keys and update semantics) before finalizing amendments.
I’m now probing a manual edge case with inline-table TOML to verify whether the new sparse writer can panic on valid-but-unusual config formatting.
I confirmed two concrete correctness defects in `save_sparse` with inline-table TOML (panic on set, silent no-op on clear), and I’m now finalizing amendment-level findings with exact file/line references.
# Final Review: AMENDMENTS

## Amendment: FR-001

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

## Amendment: FR-002

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

## Amendment: FR-003

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
