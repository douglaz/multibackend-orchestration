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


## Round 2

### Amendment: FR-PRD-001

### Problem
Approved-draft selection is too permissive. In [`src/daemon/interactive_prd.rs:2157`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs:2157), the parser uses `c.body.contains(&draft_marker)`.  
That can match non-draft bot comments that merely mention/quote the marker, which can cause wrong spec extraction and wrong `raw_idea` dispatch content.

### Proposed Change
Match draft comments by exact marker line (trimmed line equality), not substring containment. Keep reverse scan to preserve “latest in API order.” Add a unit test where a later bot comment quotes `draft-vN` but is not a draft.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs) - tighten marker matching and add regression test.

### Reviewer
codex

### Amendment: FR-PRD-002

### Problem
Heading cleanup does not implement “first content line” semantics robustly. In [`src/daemon/interactive_prd.rs:2176`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs:2176), heading stripping checks `lines.first()` directly.  
If leading blank lines exist after marker removal, the draft heading is not stripped and leaks into extracted spec.

### Proposed Change
Skip leading empty lines before heading detection, then apply `DRAFT_HEADING_PREFIX` check to the first non-empty line. Add a unit test covering marker + blank line(s) + heading.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/daemon/interactive_prd.rs) - adjust cleanup logic and add test.

### Reviewer
codex

### Amendment: FR-PRD-003

### Problem
New validate tests don’t prove the dispatched idea payload.  
`run_prd_done_daemon` always injects a mock ralph that ignores args (see [`src/validate/tests_interactive_prd.rs:4916`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs:4916) and [`src/validate/mock_scripts.rs:967`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/mock_scripts.rs:967)).  
Tests mainly assert stderr substrings (example: [`src/validate/tests_interactive_prd.rs:4962`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs:4962)) plus direct parser calls, so a daemon bug that logs success but dispatches wrong `--idea` could still pass.

### Proposed Change
Capture and assert the actual `ralph auto --idea` argument in these conformance tests. Use a custom mock ralph script that writes `$3` to a file and assert exact expected payload for:
- approved-spec path
- fallback path
- highest-revision selection
- spoof-resistance case

### Affected Files
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/src/validate/tests_interactive_prd.rs) - strengthen end-to-end assertions on dispatched idea content.

### Reviewer
codex

### Amendment: FR-PRD-004

### Problem
A stray implementation-notes artifact was committed at repo root: [`1740527543-impl-notes.md:1`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md:1). This is unintended scope creep.

### Proposed Change
Remove the artifact from the branch.

### Affected Files
- [`1740527543-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-98/1740527543-impl-notes.md) - delete file.

---

### Reviewer
codex

### Amendment: STRAY-001

### Problem
A stray file `1740527543-impl-notes.md` was committed to the repository root in commit `2b14acf`. This is an implementation notes artifact from the loop 2 implementation phase and should not be part of the shipped source tree. It is not referenced by any code and appears to be an accidental commit of a workflow artifact.

### Proposed Change
Remove `1740527543-impl-notes.md` from the repository root.

### Affected Files
- `1740527543-impl-notes.md` - delete this stray file

---

## Summary of Review

### What was verified

**Label gating (`interactive_prd.rs:583-606`, `runtime.rs:735-744`)**:
- `IN_PROGRESS_PRD_LABEL_NAMES` correctly lists exactly `ralph:prd`, `ralph:prd-active`, `ralph:prd-approved`, `ralph:prd-failed` — line 583-588
- `has_in_progress_prd_label` correctly short-circuits to `false` when `ralph:prd-done` is present (line 600-601), even when mixed with in-progress labels
- `has_prd_label` is unchanged and still matches all 5 PRD labels including `ralph:prd-done` (line 591-593)
- `poll_and_claim` uses `has_in_progress_prd_label` at line 736, not the old `has_prd_label`
- PRD labels are NOT in `LIFECYCLE_LABELS` (`github.rs:14-19`), so `ralph:prd-done` + `ralph:ready` issues correctly pass the lifecycle check at runtime.rs:731 without triggering multi-lifecycle normalization

**Shared draft format constants (`interactive_prd.rs:165-171`)**:
- `DRAFT_HEADING_PREFIX` and `DRAFT_FOOTER` are used consistently in both draft-posting paths (lines 1209, 1367) via `format_draft_comment()` and in extraction logic (`clean_draft_body` at lines 2178, 2194)
- Round-trip test at line 4056 confirms format/parse consistency

**Approved spec extraction (`interactive_prd.rs:2127-2221`)**:
- `parse_approved_spec_from_comments` correctly filters to bot-authored comments only (line 2133-2136)
- Finds highest approved revision N by scanning `status-approved-vN` markers (lines 2139-2152)
- Uses `.rev().find()` for draft selection to get latest in API order (lines 2158-2161)
- `clean_draft_body` strips markers, heading, footer, trims whitespace, returns `None` on empty (lines 2170-2204)
- `extract_approved_spec` resolves bot login and fetches comments from live API (lines 2211-2221)
- All failure modes (login fail, API fail, no markers, no matching draft, empty body) propagate as `None`

**Dispatch input selection (`runtime.rs:772-809`)**:
- `has_prd_done` flag correctly detected from issue labels (line 774)
- `extract_approved_spec` called via `spawn_blocking_op` (lines 780-789), join failures collapse to `None` via `.unwrap_or(None)` (line 789)
- Success logs `"prd-done: using approved spec"` (line 794), fallback logs `"approved spec not found, falling back"` (line 801) — both contain required substrings
- Non-prd-done issues use `compose_raw_idea` unchanged (line 808)

**Statelessness**: No local `InteractivePrdState` is read for this feature; all spec recovery is from live comments + bot identity

**Unit tests** (11 new tests, all passing):
- Label gating: positive/negative/mixed/precedence tests
- Parsing: highest revision, duplicate draft latest, bot-only filtering, no marker, no matching draft
- Cleanup: marker/heading/footer removal, empty body → `None`
- Round-trip: `format_draft_comment` → `parse_approved_spec_from_comments` consistency

**Validate conformance tests** (6 new tests registered at `validate/mod.rs:121`):
- `prd_done_dispatch_uses_approved_spec`: end-to-end daemon + parser verification
- `prd_done_mixed_labels_not_blocked`: prd-done + prd-approved not blocked
- `prd_done_missing_markers_fallback`: fallback + warning on missing markers
- `prd_done_comments_api_failure_fallback`: fallback when gh fails
- `prd_done_user_spoof_ignored`: user-authored markers ignored
- `prd_done_highest_revision_wins`: multi-revision selection

**Build**: `cargo check` passes, `cargo test` passes 1017 tests with 0 failures

**Concurrency safety**: The `extract_approved_spec` call in `poll_and_claim` runs via `spawn_blocking_op` on a separate blocking task. It uses only its own stack-local state and makes stateless API calls — no shared mutable state concerns.

**No other issues found**: Error paths are handled correctly, the code follows existing patterns, and no regressions are introduced.

### Reviewer
claude

