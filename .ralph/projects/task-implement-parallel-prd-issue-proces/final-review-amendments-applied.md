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


## Round 3

### Amendment: PRD-CONC-TEST-001

### Problem
The integration “slow vs fast” concurrency test is not a strict proof of no-blocking and uses sleep polling with a timeout fallback.

- In [tests/daemon_interactive_prd.rs:3065](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3065), `concurrent_advancement_slow_and_fast` allows issue `#80` to proceed after a 5s timeout even if `#90` never unblocks it.
- The loop at [tests/daemon_interactive_prd.rs:3119](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3119) (`sleep 0.1` polling) means a sequential implementation can still pass.
- Similar sleep-based overlap appears in bounded-concurrency checks at [tests/daemon_interactive_prd.rs:3294](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3294).

### Proposed Change
Replace sleep/polling with deterministic synchronization primitives (FIFO/barrier handshake) and remove timeout fallback that allows false positives. Assert explicit event ordering proving that the fast issue advanced before the slow issue was released.

### Affected Files
- `tests/daemon_interactive_prd.rs` - rewrite concurrency synchronization in `concurrent_advancement_slow_and_fast` (and optionally bounded test) to deterministic handshakes.

### Reviewer
codex

### Amendment: PRD-CONFORMANCE-002

### Problem
Conformance coverage is missing an explicit “slow issue does not block fast issue in same tick when `max_concurrent > 1`” case.

- Registered concurrency tests in [src/validate/tests_interactive_prd.rs:190](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:190) through [src/validate/tests_interactive_prd.rs:207](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:207) include dedup/error/panic/bounded/refresh, but no slow-vs-fast advancement test.
- That leaves a regression gap for the core no-head-of-line-blocking requirement.

### Proposed Change
Add a new validate test (for example `interactive_prd::concurrent_advancement_slow_fast`) using deterministic synchronization, and register it in `tests()`.

### Affected Files
- `src/validate/tests_interactive_prd.rs` - add and register explicit slow-vs-fast concurrent advancement conformance case.

### Reviewer
codex

