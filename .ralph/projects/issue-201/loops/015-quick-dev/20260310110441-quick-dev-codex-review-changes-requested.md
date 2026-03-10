---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T11:04:41Z
---

# Review: CHANGES REQUESTED

1. High: resume marker is cleared on `ralph:ready` recovery failures, which can break restart-drift recovery semantics.
[ `src/daemon/runtime.rs:2717` ](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2717)  
[ `src/daemon/runtime.rs:2770` ](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2770)

- `from_label == "ralph:ready"` means the marker was pre-existing recovery state, not a marker created in this cycle.
- Current code clears it on label-swap failure and on rollback-success after dispatch failure, which can drop the recovery signal.

Actionable fix:
- Track whether marker was created in this cycle (`from_label == "ralph:completed"`).
- Only clear marker in those error paths when it was created in this cycle.
- Keep marker for `ralph:ready` recovery attempts so retries remain possible.
- Add a validate case where `ready + marker` hits a transient swap/dispatch failure and ensure marker persists.

2. Medium: staged-amendment presence check is too broad and treats temp files as real staged work.
[ `src/daemon/pr_review.rs:245` ](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:245)

- `has_staged_amendments()` returns true for any directory entry, including `.tmp`.
- This can trigger no-op PR-review resume attempts when only temp/crash artifacts exist.

Actionable fix:
- Make `has_staged_amendments()` match drain semantics: only count regular `.json` files.
- Add a unit test: staging dir with only `.tmp` returns `false`.

Implementation is otherwise aligned with most spec goals (whitelist plumbing, multi-endpoint polling with `--paginate`, dedup persistence, staged drain flow, and quick-dev/regular resume wiring).
