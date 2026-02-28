---
artifact: final-review-proposals
loop: 8
project: issue-93
backend: openrouter
role: final_reviewer
created_at: 2026-02-28T22:41:37Z
---

# Final Review: AMENDMENTS

## Amendment: RVW-001

### Problem
`unstage_non_commit_artifacts()` is implemented with:

- `git rm --cached -r --ignore-unmatch .ralph` (`src/git/commit.rs`, lines ~268–281),
- and is now called from `commit_feature_loop()`, `commit_and_push_phase_transition()`, and `stage_implementation_changes()` (lines ~123–126, ~216–219, ~258–265).

This is unsafe when `.ralph/**` files are already tracked (which is now true after `commit_and_push_initial_prompt()` starts committing `.ralph/projects/<id>/*`).

`git rm --cached` on tracked `.ralph` paths stages **deletions** from the index, not a harmless unstage. In practice this can silently remove tracked prompt/project files from later commits and leaves `.ralph/` as untracked (`?? .ralph/`). This is a correctness/data-loss regression, not just “git pollution prevention.”

The updated orchestrator tests now explicitly filter out `?? .ralph/` (`tests/orchestrator.rs`, lines ~510–513 and ~2669+), which masks this regression instead of catching it.

### Proposed Change
Replace `.ralph` handling in unstaging logic with a non-destructive unstage strategy:

- For orchestration paths (`.ralph`), use `git reset HEAD -- .ralph` (or `git restore --staged -- .ralph`) best-effort.
- Keep `git rm --cached --ignore-unmatch -- SPEC.md` (or equivalent) only for explicit generated artifact files that should be removed from index.
- Ensure this behavior works in unborn-HEAD repos (fall back gracefully if `HEAD` missing).

Also strengthen tests so this regression cannot hide:
- Assert no staged deletions of tracked `.ralph/projects/<id>/prompt.md|project.toml|config.toml` after implementation/phase-transition staging.
- Remove the blanket `?? .ralph/` ignore from orchestrator tests and replace it with targeted assertions.

### Affected Files
- `src/git/commit.rs` - make `.ralph` unstaging non-destructive; keep generated-artifact handling explicit.
- `tests/orchestrator.rs` - stop masking `.ralph` index/worktree pollution.
- (optionally) `src/validate/tests_pr_lifecycle.rs` - add a conformance assertion that tracked prompt inputs are not staged for deletion by later commits.

---

## Amendment: RVW-002

### Problem
`draft_pr_watcher()` can fail permanently/silently when configured base branch is absent locally (e.g. config says `master`, repo uses `main`):

- It calls `github::has_commits_ahead_of_base(&worktree_path, &base_branch)` each cycle (`src/daemon/runtime.rs`, lines ~250–263).
- `has_commits_ahead_of_base()` does a strict `git rev-list --count <base>..HEAD` with no fallback (`src/daemon/github.rs`, lines ~585–613).
- On failure, watcher logs and treats as `false`, then keeps polling forever (runtime lines ~256–261), so no draft PR is created “when work begins.”

This violates the intended draft-PR lifecycle behavior in common base-branch mismatch environments.

### Proposed Change
Harden ahead-of-base detection to mirror existing robust base-resolution patterns used elsewhere (`has_diff_with_base`):

- Resolve/verify base ref first (`origin/<base>` if exists, else detect remote default branch).
- Use the resolved ref for `rev-list --count`.
- If base cannot be resolved, return a typed error with actionable context (not silent false forever).
- Consider bailing watcher after repeated identical base-resolution failures (or at least exponential backoff + explicit terminal log) to avoid infinite noisy loops.

### Affected Files
- `src/daemon/github.rs` - make `has_commits_ahead_of_base` resilient to base-branch mismatch.
- `src/daemon/runtime.rs` - handle persistent ahead-check failures explicitly (not silently as “not ahead” forever).
- `src/validate/tests_pr_runtime.rs` - add a conformance case where configured base is missing but default branch exists, and draft PR is still created.

---

## Amendment: RVW-003

### Problem
A non-source artifact file was committed at repo root:

- `20260228T151926-impl-notes.md`

This appears to be loop/internal notes and is outside normal code/test scope. It is stray change noise and should not ship with product source changes.

### Proposed Change
Remove the file from the commit and, if needed, keep such notes under ignored orchestration paths only.

### Affected Files
- `20260228T151926-impl-notes.md` - delete from repository history for this feature branch.
