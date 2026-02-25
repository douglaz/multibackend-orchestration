# Final Review Amendments Applied

## Round 1

### Amendment: AMND-PRD-002

### Problem
The new `prd_done_*` conformance tests claim dispatch behavior, but they do not assert the dispatched `--idea` payload.  
Current assertions only check stderr substrings and parser helper output ([`src/validate/tests_interactive_prd.rs:4962`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs#L4962), [`src/validate/tests_interactive_prd.rs:4983`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs#L4983)).  
Also, the daemon mock `ralph` script ignores args and exits immediately ([`src/validate/mock_scripts.rs:967`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/mock_scripts.rs#L967)), so runtime `raw_idea` regressions could pass undetected.

### Proposed Change
Capture the actual `--idea` argument in the daemon mock `ralph` script (write to a temp file), and assert:
1. PRD-done success paths dispatch approved cleaned spec.
2. Fallback paths dispatch exact `compose_raw_idea(title, body)` output.
3. Highest approved revision path dispatches the expected revision body.

### Affected Files
- [`src/validate/mock_scripts.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/mock_scripts.rs) - add argument-capturing daemon mock.
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs) - assert captured dispatched idea content.

### Reviewer
codex

### Amendment: AMND-PRD-003

### Problem
A timestamped implementation artifact was committed at repo root ([`1740527543-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md)). This appears unrelated to product/runtime behavior and is likely accidental scope creep.

### Proposed Change
Remove the file from the branch.

### Affected Files
- [`1740527543-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md) - delete.

---

### Reviewer
codex

### Amendment: STRAY-001

### Problem
A stray implementation notes file `1740527543-impl-notes.md` was committed to the repository root (in commit `2b14acf`). This is a build artifact / working document that does not belong in the project source tree. It contains internal implementation decisions and testing notes that are not relevant to the codebase.

### Proposed Change
Delete `1740527543-impl-notes.md` from the repository.

### Affected Files
- `1740527543-impl-notes.md` - delete entirely

---

## Summary of Verified Items

**Label gating** (`src/daemon/interactive_prd.rs:583-607`): Correct. `IN_PROGRESS_PRD_LABEL_NAMES` correctly excludes `ralph:prd-done`. `has_in_progress_prd_label` checks for `ralph:prd-done` first and returns `false`, giving it precedence over in-progress labels. `has_prd_label` is preserved unchanged for other call sites.

**Shared draft format constants** (`src/daemon/interactive_prd.rs:165-171`): Correct. `DRAFT_HEADING_PREFIX`, `DRAFT_FOOTER`, and `format_draft_comment()` are used consistently. Both draft-posting paths (lines 1209 and 1367) now use `format_draft_comment()` instead of inline formatting. No other inline occurrences of "Draft Engineering Specification" exist in `src/`.

**Approved spec extraction** (`src/daemon/interactive_prd.rs:2127-2221`): Correct. `parse_approved_spec_from_comments` correctly filters by bot login, finds highest approved revision via marker parsing, selects latest draft-vN in API order via `.rev().find()`, and delegates to `clean_draft_body`. The marker parsing correctly handles `<!-- ralph:prd:{N}:status-approved-v{N} -->` format. `clean_draft_body` correctly strips marker lines, heading (via `starts_with`), footer (via exact match after stripping trailing empty lines), and trims whitespace. The `extract_approved_spec` I/O wrapper correctly chains `.ok()?` for error-to-None conversion.

**Dispatch input selection** (`src/daemon/runtime.rs:772-809`): Correct. The `spawn_blocking_op` usage wraps `Option<String>` in `Ok(...)` to satisfy the `Result<T>` closure signature, and `.unwrap_or(None)` correctly converts join failures to the fallback path. Log messages contain the required substrings. Non-prd-done issues use `compose_raw_idea()` unchanged.

**Concurrency safety**: No shared mutable state introduced. The `spawn_blocking_op` call uses cloned values (`config.gh_bin.clone()`, etc.) and operates on an independent thread. No filesystem isolation concerns — the extraction is read-only GitHub API calls.

**Error handling**: All error/failure paths lead to the fallback (`compose_raw_idea`). Login failure, API failure, parsing failure, empty cleanup result — all produce `None` which triggers the fallback with appropriate logging.

**Tests**: All 73 unit tests pass (7 new). The 6 new conformance tests are properly registered and exercise the daemon end-to-end via `run_prd_done_daemon`. The roundtrip test (`format_draft_comment_and_parse_roundtrip_consistency`) confirms format/parse consistency. `cargo check` and `cargo check --tests` pass with zero warnings.

### Reviewer
claude

