---
artifact: completer-verdict
loop: 4
project: issue-93
backend: openrouter(openai/gpt-5.3-codex)
role: completer
created_at: 2026-02-28T21:38:08Z
---

# Verdict: CONTINUE

## Missing Requirements
1. **Git pollution prevention is only partially implemented**:  
   Requirement says `commit_and_push_phase_transition` must explicitly unstage tracked generated artifacts (e.g. via `git rm --cached`) and safely ignore missing pathspecs.  
   In `src/git/commit.rs`, `commit_and_push_phase_transition()` currently only does:
   - `git add -A`
   - `git rm --cached -r --ignore-unmatch .ralph`  
   It does **not** explicitly unstage other generated artifacts like `/SPEC.md` (which is referenced in runtime and gitignored, but tracked files still need explicit `rm --cached` behavior per requirement).

2. **The required 12 conformance tests are not fully present at the required behavior level**:  
   While `src/validate/tests_pr_runtime.rs` and `src/validate/tests_pr_lifecycle.rs` add related tests, several required scenarios are only partially covered or replaced with weaker unit checks:
   - Draft watcher tests mostly validate helpers (`has_commits_ahead_of_base`, cancellation token behavior) rather than the actual watcher task behavior and push→create ordering assertions.
   - No explicit validate test found that proves `create_pr` passes `--draft` through command args.
   - Ready/close lifecycle tests mostly check predicate helpers, not end-to-end PR state transitions.
   - “Real-binary e2e draft create → ready/close” appears replaced by a CLI flag smoke test (`--pr-url` accepted), which is not equivalent.

## Recommended Next Features
1. **Complete git pollution hardening in phase-transition commits**:
   - In `commit_and_push_phase_transition`, add explicit `git rm --cached --ignore-unmatch` for generated artifact path(s) like `SPEC.md` (and any other known generated outputs), without deleting working-tree files.
   - Keep `.ralph` unstage behavior as-is.

2. **Add/upgrade validate tests to match the 12 required cases exactly**:
   - Add true watcher-level tests for draft creation trigger, push-before-create order, and cancellation shutdown.
   - Add a concrete test for `create_pr(..., draft=true)` resulting in `gh pr create --draft`.
   - Add end-to-end validate coverage for draft PR creation followed by ready-mark or close-on-no-diff path against the built binary/mocks.

---
