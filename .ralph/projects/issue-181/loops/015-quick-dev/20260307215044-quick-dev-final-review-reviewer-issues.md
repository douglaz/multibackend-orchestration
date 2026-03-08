---
artifact: quick-dev-final-review
loop: 15
project: issue-181
backend: codex
role: reviewer
created_at: 2026-03-07T21:50:44Z
---

# Final Review: AMENDMENTS

## Amendment: A1 [P1] Preserve Existing Expected Branch During Worktree Branch Correction

### Problem
In [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs):270, mismatch correction now always executes `git checkout --force -B <expected_branch>`.  
`-B` rewrites the expected branch ref to the current HEAD, so if the expected branch already exists (with local/unpushed commits), its tip is overwritten during reuse. This is a data-loss risk in mismatch scenarios.

Test coverage does not catch this regression: current new coverage focuses on missing-branch migration (e.g. [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs):697), not preserving an existing expected branch tip.

### Proposed Change
Change `verify_worktree_branch` behavior to:
1. Check whether `refs/heads/<expected_branch>` exists.
2. If it exists, run `git checkout --force <expected_branch>` (no ref rewrite).
3. Only if it does not exist, run `git checkout --force -B <expected_branch>` (migration fallback).

Add a regression test that:
1. Creates expected branch with a unique commit,
2. Switches worktree to a different branch,
3. Reuses worktree,
4. Asserts expected branch SHA is unchanged and HEAD is restored to expected branch.

### Affected Files
- [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs) - branch correction logic in `verify_worktree_branch`.
- [`src/daemon/worktree.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-181/src/daemon/worktree.rs) - unit test to verify no clobber when expected branch already exists.

---
