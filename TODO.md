# TODO

## Bug: Rollback leaves untracked files, blocking `--until-complete`

**Severity:** High — causes `run --until-complete` to abort after any rollback

**Observed:** Loops 14 and 18 of the ralph-rewrite project both hit this.

**Description:**
When an implementer creates **new** files (not just edits to existing ones) during a loop, and that loop later gets rolled back due to exceeding `max_review_iterations`, the rollback resets git to the previous commit and removes the loop entry from state.json — but **untracked files created by the implementer remain on disk**. Ralph's dirty-tree check then prevents any subsequent loop from starting, causing `--until-complete` to fail with:

```
error: cannot start a new loop with uncommitted changes outside `.ralph/`.
```

**Root cause:**
`ralph rollback` does `git reset/checkout` to the prior loop's commit, which only affects tracked files. Newly created (untracked) files are invisible to `git reset` and stay on disk.

**Reproduction:**
1. Run a loop where the implementer creates new source files (e.g. new modules)
2. Have the reviewer never approve (or set `max_review_iterations` low)
3. Loop hits the limit and rolls back
4. Ralph tries to start the next loop → dirty tree error

**Workaround:**
After a rollback failure: `git stash` (or `git clean -fd` for untracked only), fix state.json to mark the last completed loop correctly, then restart.

**Fix options:**
- Run `git clean -fd` (excluding `.ralph/`) as part of the rollback procedure to remove untracked files
- Or track which files the implementer created and explicitly remove them on rollback
- Should also consider: the dirty-tree check could exclude files that would be cleaned by the rollback itself

## Bug: Reviewer sees incomplete diff — untracked files invisible during review

**Severity:** High — root cause of non-convergent review cycles on loops that create new files

**Observed:** Loops 14 and 18 of the ralph-rewrite project. The reviewer correctly flags "build fails, modules missing" but the implementer can never fix it because the problem is staging, not code.

**Description:**
When the implementer creates **new** files (e.g. `src/cli/prd.rs`, `src/prd/pipeline.rs`), those files are untracked. The reviewer prompt is built using `git diff HEAD` (`src/workflow/orchestrator.rs:599`), which only shows changes to **tracked** files. So the reviewer sees `mod prd;` added to `src/cli/mod.rs` but the actual `prd.rs` file is absent from the diff. The reviewer flags a broken build, the implementer re-writes the files (which are already correct on disk), and the cycle repeats until `max_review_iterations`.

**Root cause:**
Ralph never runs `git add` between the implement and review phases. The staging only happens at commit time (`git add -A` in `src/git/commit.rs:119`), which is after review has already approved/rejected.

**Key code path:**
1. `src/backend/mod.rs:149-196` — backend subprocess creates files on disk
2. `src/workflow/orchestrator.rs:448` — transitions to review phase (no `git add`)
3. `src/workflow/orchestrator.rs:599` — `git diff HEAD` misses untracked files
4. `src/git/commit.rs:119` — `git add -A` happens here, but too late

**Fix:**
Add `git add -A` (or `git add -N` for intent-to-add) after the implement phase completes, before transitioning to review. This ensures `git diff HEAD` includes newly created files and the reviewer sees the full picture. The `git add -A` in the commit phase can remain as-is.
