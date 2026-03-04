---
artifact: quick-dev-codex-review
loop: 15
project: issue-156
backend: codex
role: reviewer
created_at: 2026-03-04T16:43:51Z
---

# Review: CHANGES REQUESTED
1. **Critical: quick-dev cleanup does not remove common untracked stray files when commits are skipped/disabled.**  
   In [`checkpoint_if_enabled`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-156/src/workflow/quick_dev_orchestrator.rs:1141), cleanup is invoked before any `git add -A`.  
   In [`remove_stray_impl_artifacts`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-156/src/git/commit.rs:297), fallback filesystem deletion only runs when `git rm` errors ([`commit.rs:314`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-156/src/git/commit.rs:314)-[`315`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-156/src/git/commit.rs:315)). For untracked files, `git rm --ignore-unmatch` exits 0 and leaves the file, so cleanup silently fails.  
   This fails the acceptance criteria for quick-dev transition coverage, pre-review removal, and untracked handling.  
   Repro: `nix develop -c cargo run -- validate --bin target/debug/ralph --filter stray_cleanup:: --verbose` currently fails `stray_cleanup::quick_dev_cleanup`, `stray_cleanup::user_files_preserved`, and `stray_cleanup::multi_iteration_cleanup`.

2. **Low: tracked-file coverage is still weak in unit tests.**  
   [`remove_stray_impl_artifacts_cleans_tracked_and_untracked`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-156/src/git/commit.rs:862) labels files as tracked but does not actually commit stray artifacts first, so true tracked-from-HEAD behavior is not directly verified.

Concrete fixes:
1. Make `remove_stray_impl_artifacts` delete a matched file if it still exists after `git rm` (not only on `Err`), and then ensure index cleanup with `git rm --cached --ignore-unmatch -- <name>` when needed.
2. Keep quick-dev pre-review cleanup call, then rerun `ralph validate --filter stray_cleanup::` and confirm all 5 tests pass.
3. Strengthen the unit test by committing at least one canonical stray file first, then verifying cleanup removes it from both working tree and index.
