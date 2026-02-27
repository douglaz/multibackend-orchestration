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


## Round 4

### Amendment: FR-001-REMOVE-ORPHAN-ROOT-ARTIFACTS

### Problem
Two non-source artifact files were committed at the repository root: `20260225T012624Z-impl-response-001.md:1` and `8-impl-notes.md:1`. These are loop/process artifacts, not product code or test assets, and they create repository noise. This also conflicts with the project’s prior cleanup intent for orphan root artifacts.

### Proposed Change
Delete both artifact files from the branch and keep implementation-loop notes/responses only under `.ralph/.../loops/...` artifact paths.

### Affected Files
- `20260225T012624Z-impl-response-001.md` - remove orphan implementation-response artifact from repo root
- `8-impl-notes.md` - remove orphan implementation-notes artifact from repo root

---

### Reviewer
codex

### Amendment: ORPHAN-001

### Problem
A workflow artifact file `20260225T012624Z-impl-response-001.md` is committed at the repository root. This is a loop 8 implementation response document that was accidentally committed outside the `.ralph/` artifacts directory. It is tracked by git (committed in `bf9d023`).

### Proposed Change
Remove the orphan file from the repository root and commit the deletion.

### Affected Files
- `20260225T012624Z-impl-response-001.md` - delete from repo root

### Reviewer
claude


## Round 1

### Amendment: FR-PRD-001

### Problem
Concurrent workers are enabled in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:421), but all issue backends run in the same clone directory (`config.repo_clone_path()`) via shared `cwd` wiring in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1081), [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1335), [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1479), and [`backend/mod.rs`](/tmp/final-review-test/src/backend/mod.rs:478).  
Default backend configs are write-capable (`Edit/Write` tools and bypass flags) in [`config/global.rs`](/tmp/final-review-test/src/config/global.rs:701) and [`config/global.rs`](/tmp/final-review-test/src/config/global.rs:730).  
Result: concurrent issues share one mutable filesystem workspace, which can cause cross-issue interference and nondeterministic outcomes.

### Proposed Change
Use per-issue isolated working directories for backend execution in a tick (e.g., per-issue worktree/snapshot), and pass that issue-specific path as `cwd`. Keep clone refresh once per tick, but do not run multiple issue backends in the same directory.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs) - create/use per-issue work dirs and thread them through PRD generation calls.
- [`src/backend/mod.rs`](/tmp/final-review-test/src/backend/mod.rs) - keep current `cwd` support; ensure callers provide isolated paths.

### Reviewer
codex

### Amendment: FR-PRD-002

### Problem
Panics are caught and logged in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:440) and [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:459), then the tick returns `Ok` in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:484).  
That bypasses durable failure accounting/state transitions implemented in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1169) and [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1566).  
A repeatable panic can therefore retry forever without persisting failure state.

### Proposed Change
Route panic outcomes through the same persisted error path as normal failures: increment `error_count`, set `last_error`, save state, and transition to `Failed` at threshold. Best approach: convert panic to `Err` within a state-aware wrapper so `finish_transition` handles it uniformly.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs) - unify panic/error persistence and failure-threshold behavior.
- [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs) - add regression asserting repeated panic reaches durable `Failed`.
- [`src/validate/tests_interactive_prd.rs`](/tmp/final-review-test/src/validate/tests_interactive_prd.rs) - add conformance assertion for panic persistence.

### Reviewer
codex

### Amendment: FR-PRD-003

### Problem
Deadlock-prone FIFO tests run `poll_and_advance_prd` without a watchdog timeout in [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs:3241), [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs:3502), and [`src/validate/tests_interactive_prd.rs`](/tmp/final-review-test/src/validate/tests_interactive_prd.rs:3839).  
The validate runner has no per-test timeout in [`runner.rs`](/tmp/final-review-test/src/validate/runner.rs:117).  
If concurrency regresses, these tests can hang the suite indefinitely.

### Proposed Change
Wrap those calls with a watchdog (`thread::spawn` + `recv_timeout`) and fail explicitly on timeout. Also restore env vars via RAII so timeout/panic paths do not leak mutated `PATH`.

### Affected Files
- [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs) - add watchdogs to FIFO/barrier tests.
- [`src/validate/tests_interactive_prd.rs`](/tmp/final-review-test/src/validate/tests_interactive_prd.rs) - add watchdog to bounded-worker test and RAII env restoration helper.

---

### Reviewer
codex

### Amendment: SLOW-FAST-WATCHDOG

### Problem
The integration test `concurrent_advancement_slow_and_fast()` in `tests/daemon_interactive_prd.rs:3071` calls `poll_and_advance_prd(&config)` directly on the test thread without a watchdog timeout. If a regression causes sequential processing, issue #80's mock `gh` script blocks on a FIFO `read` that will never complete (issue #90 never starts to write to it), causing the entire `cargo test` run to hang indefinitely.

The conformance version of this test (`concurrent_advancement_slow_fast` in `src/validate/tests_interactive_prd.rs:4121-4134`) correctly uses a `std::sync::mpsc::channel` + `recv_timeout(30s)` watchdog pattern. The integration test should use the same pattern.

### Proposed Change
Wrap the `poll_and_advance_prd` call in `concurrent_advancement_slow_and_fast()` with the same watchdog timeout pattern used by the conformance test: spawn a thread, send the result via a channel, and `recv_timeout` with a 30-second deadline.

Replace:
```rust
let old_path = std::env::var("PATH").unwrap_or_default();
unsafe { std::env::set_var("PATH", &path_env) };
let result = poll_and_advance_prd(&config);
unsafe { std::env::set_var("PATH", &old_path) };
```

With:
```rust
let old_path = std::env::var("PATH").unwrap_or_default();
unsafe { std::env::set_var("PATH", &path_env) };

let watchdog_timeout = std::time::Duration::from_secs(30);
let (tx, rx) = std::sync::mpsc::channel();
let config_clone = config.clone();
let handle = std::thread::spawn(move || {
    let r = poll_and_advance_prd(&config_clone);
    let _ = tx.send(r);
});
let result = rx.recv_timeout(watchdog_timeout)
    .expect("slow/fast integration test timed out — possible FIFO deadlock regression");
let _ = handle.join();

unsafe { std::env::set_var("PATH", &old_path) };
```

### Affected Files
- `tests/daemon_interactive_prd.rs` - Add watchdog timeout to `concurrent_advancement_slow_and_fast()` (lines ~3239-3242)

---

### Reviewer
claude

