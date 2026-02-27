---
artifact: final-review-proposals
loop: 3
project: task-implement-parallel-prd-issue-proces
backend: claude
role: final_reviewer
created_at: 2026-02-25T15:42:11Z
---

Now I have a thorough understanding of the changes. Let me compile my final review. The core implementation is sound, but I found two issues:

1. **Integration test `concurrent_advancement_slow_and_fast` lacks a watchdog timeout** — it can hang indefinitely if a regression causes sequential processing (FIFO deadlock). The conformance version has a watchdog, but the integration test does not.

2. **Out-of-scope legacy symlink removal** — Commit `7cdda5b` removes legacy template symlinks from `src/cli/init.rs`, `tests/init_command.rs`, and `src/validate/tests_init.rs`. This is unrelated to the parallel PRD work and could be a breaking change for users relying on those symlinks.

The core concurrency changes are well-implemented:
- `poll_and_advance_prd` correctly deduplicates, refreshes once, then uses `thread::scope` with a bounded work queue
- Each worker has its own `bot_login_cache` (per-thread isolation)
- `catch_unwind` properly catches panics and records them
- `CliBackend.cwd` correctly uses `Command::current_dir()` instead of process-global cwd
- `refresh_repo_clone` uses `Command::current_dir()` properly
- State files are per-issue so no concurrent write conflicts
- All new integration tests use `#[serial]` for env var safety
- Conformance tests use `ENV_MUTEX` for thread safety

# Final Review: AMENDMENTS

## Amendment: SLOW-FAST-WATCHDOG

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

## Amendment: OUT-OF-SCOPE-LEGACY-REMOVAL

### Problem
Commit `7cdda5b` removes legacy template symlink support (`LEGACY_LINKS`, `CreateLegacyLink` variant, and associated symlink creation logic) from `src/cli/init.rs`, along with corresponding test assertions in `tests/init_command.rs` and `src/validate/tests_init.rs`. This is completely unrelated to the parallel PRD issue processing task described in the master prompt.

This removal is a **breaking change** for users whose workflows depend on the legacy symlink paths (`planner.md`, `implementer.md`, `reviewer.md`, `completer.md`). It should be in a separate branch/PR with its own review and changelog entry, not bundled into the parallel PRD work.

### Proposed Change
Revert the legacy symlink removal changes. This can be done by restoring the removed code from the base commit. Specifically:
- Restore `LEGACY_LINKS` constant, `CreateLegacyLink` enum variant, and symlink creation logic in `src/cli/init.rs`
- Restore legacy symlink test assertions in `tests/init_command.rs`
- Restore `dry-run: create-legacy-link` lines in `src/validate/tests_init.rs`

If the legacy symlink removal is intentional, it should be done in a separate PR with proper documentation.

### Affected Files
- `src/cli/init.rs` - Revert removal of `LEGACY_LINKS`, `CreateLegacyLink`, and symlink logic
- `tests/init_command.rs` - Revert removal of legacy symlink assertions
- `src/validate/tests_init.rs` - Revert removal of `create-legacy-link` dry-run lines

---
