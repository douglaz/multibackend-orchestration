---
artifact: acceptance-fail
loop: 2
project: when-a-daemon-task-fails-e-g-backend-tim
backend: codex(gpt-5.3-codex-xhigh)
role: qa
created_at: 2026-02-14T21:55:27Z
---

# QA: FAIL
## Failures
1. Fresh-dispatch behavior regressed: tasks with `project_id = null` can still take the resume path.  
Evidence in code: `src/daemon/runtime.rs:417`, `src/daemon/runtime.rs:424`, `src/daemon/runtime.rs:433` computes `effective_project_id` from discovery and dispatches `spawn_ralph_run(...)` when that discovered value is `Some(...)`, even though `task.project_id` is `None`.
2. Manual repro (2026-02-14) confirmed the regression: with exactly one project under `.ralph/projects/` and a pending task with `"project_id": null`, daemon spawned:
`run`, `--project`, `existing-project`, `--until-complete`  
instead of fresh-dispatch (`ralph auto --idea ...`). This violates required behavior #3 and acceptance criterion #1.
3. Required automation passed (`nix develop -c cargo check`, `nix develop -c cargo test`, `nix build -L`, `./result/bin/ralph validate --bin ./result/bin/ralph --filter daemon` with 35/35 passing), but project-wide acceptance is still not satisfied because criterion #1 fails.

## Suggested Fixes
1. Enforce the dispatch gate exactly as specified: in `dispatch_task()`, only call `spawn_ralph_run(...)` when `task.project_id.is_some()`. Otherwise always use the existing fresh-dispatch path.
2. Keep project discovery for persistence/recovery only (CAS activation + `complete_task()` before `cleanup_worktree()`), or hydrate `task.project_id` in a retry-only path before dispatch so fresh tasks are unaffected.
3. Add a daemon conformance test in `src/validate/tests_daemon.rs` for: “pending task with `project_id: null` + exactly one workspace project must still dispatch via `ralph auto --idea`.”
