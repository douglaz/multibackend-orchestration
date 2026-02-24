# Final Review Amendments Applied

## Round 1

### Amendment: REFRESH-CONFORMANCE-001

### Problem
The master prompt requires a "Repo refresh ordering test" in conformance coverage (`src/validate/tests_interactive_prd.rs`): "Assert refresh occurs once per non-empty tick and before any backend invocation." The integration test `refresh_repo_clone_once_before_processing` in `tests/daemon_interactive_prd.rs` covers this, but there is no corresponding conformance test in `src/validate/tests_interactive_prd.rs`. The `pub fn tests()` vector has no entry for refresh ordering. Grepping for "refresh" and "repo_clone" in the conformance file yields zero matches.

### Proposed Change
Add a conformance test `concurrent_refresh_ordering` (or similar) to `src/validate/tests_interactive_prd.rs` that mirrors the integration test logic: mock `git` to log "refresh" events and `gh` to log "edit:NNN" events to a shared log file, then assert refresh is the first event and appears exactly once. Register it in the `pub fn tests()` vector.

### Affected Files
- `src/validate/tests_interactive_prd.rs` - Add conformance test for repo refresh ordering + register in `tests()` vector

---

### Reviewer
claude


## Round 2

### Amendment: FR-002

### Problem
A loop artifact file was added at repository root and is unrelated to product code/tests: [1740440931-impl-notes.md:1](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/1740440931-impl-notes.md:1).

### Proposed Change
Remove this file from the tracked source tree (or relocate it under `.ralph` artifacts if it must be preserved).

### Affected Files
- [1740440931-impl-notes.md](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/1740440931-impl-notes.md) - Remove from repo root.

---

### Reviewer
codex

### Amendment: ORPHANED-IMPL-NOTES

### Problem
The file `1740440931-impl-notes.md` was committed at the repository root. This is a build/loop artifact from the automated implementation process and does not belong in the source tree. It was introduced in this branch (visible in `git diff master...HEAD --name-only`).

### Proposed Change
Remove the file from the repository. It contains no information needed by the codebase and will clutter the repo root.

### Affected Files
- `1740440931-impl-notes.md` - delete this file

### Reviewer
claude

