---
artifact: acceptance-pass
loop: 6
project: parallel-worktrees
backend: codex(gpt-5.3-codex-xhigh)
role: qa
created_at: 2026-02-13T17:26:58Z
---

# QA: PASS
## Tests Run
- `git diff --name-status master...HEAD` and `git diff --name-status` (full project diff scope reviewed against base plus working tree state)
- `nix develop -c cargo check` (pass)
- `nix develop -c cargo test` (pass)
- `nix build -L` (pass; build-time test + validate hooks passed)
- `./result/bin/ralph validate --bin ./result/bin/ralph` (pass: `98 passed, 0 failed`)
- Manual acceptance smoke checks with built binary in temp git repo/workspace:
- `ralph init` does not create `.ralph/index.json`
- `project new/list/use` uses `.git/ralph-active-project` and list ordering is deterministic
- one-time migration from legacy `.ralph/index.json` seeds local active project and logs migration
- stale active project returns exit code `2` with remediation hint
- corrupt active-project file returns exit code `2` with warning

## Verification Summary
- Project-wide migration off centralized index is implemented and consistent across loops:
- `src/workspace/index.rs` removed; project metadata is now scan-derived via `src/workspace/mod.rs` + `src/workspace/summary.rs`
- worktree-local active project resolution/writes implemented in `src/workspace/active.rs`
- CLI and MCP surfaces switched from index lookups to scan/existence + local active resolution (`src/cli/*`, `src/mcp/handlers.rs`)
- orchestrator and rollback no longer sync shared index (`src/workflow/orchestrator.rs`, `src/cli/rollback.rs`)
- `ProjectState` now includes `created_at` and atomic save (`src/project/state.rs`)
- legacy compatibility/migration behavior is present (ignore legacy index, optional one-time active-project migration)
- validate conformance coverage for the new behavior is present and passing (`src/validate/tests_init.rs`, `src/validate/tests_project.rs`)

Project acceptance criteria are satisfied across the combined feature loops and current full diff.
