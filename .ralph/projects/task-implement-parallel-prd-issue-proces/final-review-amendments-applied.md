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

