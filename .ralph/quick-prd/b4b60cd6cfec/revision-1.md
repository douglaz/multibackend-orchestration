Here's how each of the 12 review issues was addressed:

| # | Issue | Resolution |
|---|---|---|
| 1 | **Subprocess vs in-process contradiction** | Replaced `tokio::spawn` + `Orchestrator::run()` with real child processes: `std::process::Command` spawning `ralph auto` in a dedicated git worktree. Child has a real OS PID/PGID. |
| 2 | **Tokio tasks don't have PIDs** | `child_pid: Option<u32>` and `child_pgid: Option<u32>` now refer to actual OS PID/PGID from `setsid`-isolated child processes. Kill uses `killpg()`. |
| 3 | **Multi-repo infeasible in single worktree** | Each task runs in its own `git worktree` under `.ralph/daemon/worktrees/<task-id>/`. Out of Scope explicitly notes repos must be developable from the same git checkout. |
| 4 | **Concurrent tasks conflict in single working tree** | Git worktree per task eliminates conflicts. `max_concurrent > 1` is now safe. Worktree lifecycle (create/remove/cleanup) is fully specified. |
| 5 | **Cross-repo task ID collisions** | Task ID is `<owner>-<repo>-<number>` (e.g., `acme-widgets-42`). CLI abort accepts full ID or bare number with ambiguity detection. |
| 6 | **Paused state/label inconsistency** | Eliminated `ralph:paused` entirely. On shutdown, tasks become `pending` in `tasks.json` but keep `ralph:in-progress` on GitHub. Daemon re-adopts on restart. Explicitly out of scope. |
| 7 | **Polling correctness (pagination, label semantics)** | Added `--limit 100` with overflow warning. Specified AND semantics for labels (`--label L1 --label L2`). Filter out issues with existing `ralph:*` labels. |
| 8 | **Idempotency / duplicate side effects on restart** | All comments include machine-readable HTML markers (`<!-- ralph:task:<id>:<phase> -->`). Before posting, existing comments are checked. PR creation checks for existing PR via `gh pr list --head`. |
| 9 | **State consistency / concurrent writers** | `TaskStore` uses file locking via `fs2`: exclusive for writes, shared for reads. Lock held only during read-modify-write cycle. `status` and `abort` acquire their own locks independently. |
| 10 | **Abort semantics incomplete** | Fully specified: stale PID → skip kill, proceed with state/label update. Daemon-not-running → works directly against `tasks.json`. Terminal state → error. `SIGTERM` → 10s timeout → `SIGKILL`. Label cleanup (remove `in-progress`, add `aborted`). Bare-number resolution with ambiguity error. |
| 11 | **PR creation path incomplete** | Checks: no-diff → skip PR with note. Existing PR → reuse URL. Creation failure → log warning, still mark completed. Branch specified via `--head`. |
| 12 | **Missing conformance tests** | Added `src/validate/tests_daemon.rs` with registration in `register_tests()`. Eight specific conformance tests covering CLI parsing, config, start validation, status, and abort. |