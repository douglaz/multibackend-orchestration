---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: claude
role: implementer
created_at: 2026-03-08T17:25:38Z
---

All clean. `build_ralph_commit_message` is an unrelated git commit formatting function. The only `ChildHandle` reference is a doc comment in `mod.rs` explaining the replacement. No `spawn_ralph_*` or `build_ralph_*` (process spawn) functions remain.

Now I have enough data for a comprehensive assessment. Let me compile my findings.

# Final Review: AMENDMENTS

## Summary

The implementation is **well-executed overall**. The core migration from subprocess dispatch to in-process tokio tasks is correct and well-architected. CWD safety, environment sanitization, cancellation threading, per-task log isolation, and lifecycle management are all properly implemented. The `KillOnDrop` guard, `derive_terminal_label`, `drain_all_children`, and `collect_children` are sound. The CwdGuard has been removed, `Workspace::discover()` is not used in daemon paths, and `RALPH_MAX_BACKEND_RETRIES` env var has been replaced with a config field.

However, I found the following issues:

---

## Amendment: 1

### Problem

**[P2]** In `src/daemon/tasks.rs`, `run_run_task` (lines 274-300) and `run_quick_dev_run_task` (lines 451-473) do not call `BackendRegistry::set_cwd()` explicitly. They rely on the orchestrators (`Orchestrator::run()` at `orchestrator.rs:242` and `QuickDevOrchestrator::run()` at `quick_dev_orchestrator.rs:126`) calling `registry.set_cwd(self.workspace.root.parent())` internally during `run()`.

This is actually **correct behavior** because the orchestrators always set CWD themselves. However, the `run_auto_task` (line 156) and `run_quick_dev_auto_task` (line 363) functions create a separate `BackendRegistry` for the PRD phase and call `set_cwd()` on it — this registry is distinct from the one the orchestrator creates later. So there is no actual bug here, just an inconsistency in the pattern that could be misleading.

**Not filing as an amendment** — this is a non-issue on closer inspection.

---

## Amendment: A1

### Problem

**[P2]** In `src/daemon/tasks.rs:553-556`, the `open_log_file_append` function opens the file in append mode, then seeks to `End(-1)` to check the trailing newline:

```rust
let ends_with_newline = file.seek(SeekFrom::End(-1)).and_then(|_| {
    let mut last = [0_u8; 1];
    file.read_exact(&mut last).map(|_| last[0] == b'\n')
});
```

The file is opened with both `read(true)` and `append(true)`. In append mode, writes always go to the end regardless of seek position, so the seek+read for inspection is safe. However, if the file is exactly 0 bytes, `SeekFrom::End(-1)` will fail with an invalid seek error. This path is guarded by the `has_content` check at line 548, which requires `len > 0`, so the seek is only reached for non-empty files. **No bug here** — properly guarded.

**Not filing as an amendment.**

---

## Amendment: A2

### Problem

**[P1]** In `src/backend/mod.rs`, the default trait implementation of `execute_with_cancel` (lines 106-116) uses `tokio::select!` to race `execute_with_log` against `cancel.cancelled()`. The doc comment (lines 99-105) correctly warns that dropping the future does NOT kill spawned child processes. `CliBackend` properly overrides this method (line 978-985) to pass the token directly to `execute_streaming` which has the `KillOnDrop` guard.

However, the `TmuxBackend` (in `tmux_backend.rs:469`) does **not** override `execute_with_cancel`. It inherits the default trait implementation that drops `execute_with_log` on cancellation. The `TmuxBackend::execute_raw()` method (line 169) writes prompt files, runs tmux commands, and polls for completion files. When cancelled, this future is dropped mid-poll. The tmux window and backend process continue running inside the tmux session unmanaged. The prompt/output temp files may be left behind.

This is a **moderate concern** for daemon use: cancelled tmux-backed tasks leave orphaned tmux windows running backend processes (claude/codex) that consume compute resources indefinitely.

### Proposed Change

Override `execute_with_cancel` in `TmuxBackend` to kill the tmux window on cancellation. After the `select!` cancel branch wins, use `tmux kill-window` to terminate the session window. Alternatively, add a `KillOnDrop`-style guard that kills the tmux window if the future is dropped.

### Affected Files
- `src/backend/tmux_backend.rs` - Add `execute_with_cancel` override that performs tmux window cleanup on cancellation

---

## Amendment: A3

### Problem

**[P2]** In `src/daemon/tasks.rs`, the `spawn_inprocess_task` function (lines 506-524) creates a `tracing_subscriber::fmt::Subscriber` with `Mutex::new(file)` as the writer. When the task completes and the `Dispatch` is dropped, the `Subscriber` is dropped, which drops the `Mutex<File>`. However, `tracing_subscriber::fmt` does not guarantee a flush before dropping the writer. This means the last few log lines could be lost if they're buffered in the `fmt` layer's internal writer.

In practice, `std::fs::File` is unbuffered (each `write()` call goes directly to the OS), so `fmt::Subscriber` writing through `Mutex<File>` should not lose data. However, this relies on an implementation detail of `fmt::Subscriber` — it might internally buffer.

### Proposed Change

Wrap the file in a `std::io::BufWriter` with explicit flush on drop, or use `tracing_subscriber::fmt::writer::MakeWriterExt` to ensure the writer is flushed. Alternatively, add a comment documenting that `std::fs::File` is unbuffered so this is safe.

### Affected Files
- `src/daemon/tasks.rs` - Add a doc comment or explicit flush mechanism in `spawn_inprocess_task`

---

## Amendment: A4

### Problem

**[P3]** In `src/daemon/tasks.rs`, `run_run_task` and `run_quick_dev_run_task` do not perform backend health checks before starting orchestration. In contrast, `run_auto_task` (lines 163-164) and `run_quick_dev_auto_task` (lines 370-371) call `health_check()` on backends upfront for fail-fast behavior.

For `run`/`quick-dev-run` variants, the orchestrator's `run()` method does its own health checks during backend preloading (e.g., `orchestrator.rs:247`), so this isn't a correctness bug. The orchestrators handle the health check internally. This is a non-issue on closer inspection.

**Not filing as an amendment.**

---

## Amendment: A5

### Problem

**[P2]** In `src/cli/auto.rs`, the refactored `execute` function (line 176) creates `AutoTaskParams` with:
```rust
spec_writer: if args.spec_writer.trim().is_empty() { None } else { Some(args.spec_writer) },
```

But in `src/daemon/tasks.rs:135-146`, `run_auto_task` handles `None` by falling back to `workspace.config.workspace.daemon_prd_writer_backend`:
```rust
let writer_spec = params
    .spec_writer
    .unwrap_or_else(|| workspace.config.workspace.daemon_prd_writer_backend.clone());
```

This is correct behavior — `None` means "use workspace default". The CLI correctly maps empty string to `None`. No issue.

**Not filing as an amendment.**

---

## Amendment: A6

### Problem

**[P3]** Several `eprintln!` calls remain in `src/git/branch.rs` and `src/workspace/mod.rs` that have been converted to `tracing::warn!`/`tracing::info!`. The diff shows these were intentionally converted. However, there are still `eprintln!` calls throughout `src/daemon/runtime.rs` (e.g., lines 1526, 1532, 1546, 1689, 1810, 1820, 1824, etc.) for daemon status messages. These go to the daemon process stderr, which is appropriate since they are daemon-level diagnostics (not per-task log messages). The per-task functions in `tasks.rs` correctly use `tracing` events. This is fine.

**Not filing as an amendment.**

---

Remaining assessment:

- **CWD safety**: Verified. No `set_current_dir()` in library paths. `current_dir()` only at CLI boundaries.
- **Environment sanitization**: Verified. `SANITIZED_ENV_VARS` applied in both `CliBackend::execute_streaming` and `TmuxBackend::build_command_line`.
- **Per-task logging**: Verified with `WithSubscriber` pattern. Test at `tasks.rs:630` validates isolation.
- **Cancellation**: Properly threaded through all 4 dispatch variants, checked between phases, and in backend execution via `tokio::select!`.
- **Task lifecycle**: `collect_children` → `derive_terminal_label` → `complete_task` chain is correct. `drain_all_children` has proper cooperative → forced abort escalation.
- **Tests**: Comprehensive coverage of new functionality. `derive_terminal_label` unit tests, drain timeout tests, log isolation tests, cancellation tests all present.

The only real amendment warranting attention is **A2** (TmuxBackend cancellation leaving orphaned tmux windows).
