---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T19:28:03Z
---

# Final Review: AMENDMENTS

## Amendment: STALE-ENV-1

### Problem
`RALPH_DAEMON_BIN` environment variable is set in ~28 test cases in `src/validate/tests_interactive_prd.rs` and ~18 test cases in `tests/daemon_interactive_prd.rs`, but it is never read by any production code. The daemon no longer resolves a binary path for subprocess dispatch — it runs orchestration in-process via `spawn_inprocess_task()`. The associated mock ralph scripts (`daemon_mock_ralph_script()` in `src/validate/mock_scripts.rs`) are written to disk but never executed. This is dead test setup code that misleads future contributors into thinking subprocess dispatch still exists.

### Proposed Change `[P2]`
Remove all `("RALPH_DAEMON_BIN", &ralph_path)` entries from `daemon_env()` and `ralph_env()` call sites in `src/validate/tests_interactive_prd.rs` (28 sites, e.g. lines 584, 668, 908, 1086, 1214, 1329, 1550, 1676, 1777, 1876, 1974, 2078, 2176, 2290, 2409, 2492, 2596, 2702, 2820, 2925, 3059, 3177, 3212, 3429, 3583, 3725, 3942) and `tests/daemon_interactive_prd.rs` (18 sites, e.g. lines 407, 629, 819, 991, 1159, 1417, 1442, 1468, 1598, 1721, 1872, 1998, 2170, 2308, 2345, 2466, 2648, 2799). Also remove `write_daemon_mock_ralph()` calls and the `daemon_mock_ralph_script()` function from `src/validate/mock_scripts.rs` if no remaining callers exist.

### Affected Files
- `src/validate/tests_interactive_prd.rs` - Remove ~28 `RALPH_DAEMON_BIN` env var entries
- `tests/daemon_interactive_prd.rs` - Remove ~18 `RALPH_DAEMON_BIN` env var entries
- `src/validate/mock_scripts.rs` - Remove `daemon_mock_ralph_script()` if unused after cleanup

---

## Amendment: ORDERING-1

### Problem
In `src/daemon/runtime.rs:2085`, `aborted_externally` is loaded with `Ordering::Relaxed` while all other 3 access sites use `Ordering::SeqCst` (store at line 1988, loads at lines 1808 and 2384). While this is technically safe in the current code (both the store and this load execute on the same async task's sequential main loop, so task-local sequencing provides the happens-before), it's a consistency hazard: a future refactor that moves `drain_all_children` to a different task or calls it from a concurrent context would silently introduce a data race. The inconsistency also makes code review harder — every reader must verify the single-task invariant to confirm correctness.

### Proposed Change `[P3]`
Change `Ordering::Relaxed` to `Ordering::SeqCst` at line 2085 to match all other access sites.

### Affected Files
- `src/daemon/runtime.rs` - Line 2085: change `Ordering::Relaxed` to `Ordering::SeqCst`

---
