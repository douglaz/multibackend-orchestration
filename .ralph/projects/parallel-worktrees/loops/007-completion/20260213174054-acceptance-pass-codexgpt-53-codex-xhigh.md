---
artifact: acceptance-pass
loop: 7
project: parallel-worktrees
backend: codex(gpt-5.3-codex-xhigh)
role: qa
created_at: 2026-02-13T17:40:54Z
---

# QA: PASS
## Tests Run
- `nix develop -c cargo check` (pass)
- `nix develop -c cargo test` (pass; full unit + integration suite)
- `nix build -L` (pass; release build succeeded and in-build conformance ran)
- `./result/bin/ralph validate --bin ./result/bin/ralph` (pass; `98/98`)

## Verification Summary
- Reviewed full current diff against `origin/master` (`57 files changed`) and validated project-wide behavior, not just one loop.
- `index.json` registry removal is implemented: `src/workspace/index.rs` deleted, `Workspace` no longer carries/saves index, and `init` no longer creates it.
- Project discovery is now scan-based from `.ralph/projects/*/state.json`, with deterministic sorting and computed `ProjectSummary` (`src/workspace/mod.rs`, `src/workspace/summary.rs`).
- Active project is worktree-local via git dir resolution (`src/workspace/active.rs`), with one-time legacy migration from `index.json`, stale-ID handling, and invalid/empty file warnings.
- CLI and MCP surfaces were migrated off index lookups: `run/status/history/tail/rollback/project show/config*` and MCP `project_list/project_show/status/history/config_show` now resolve via scan/local-active APIs.
- Orchestrator and rollback no longer sync any shared index (`persist_state_and_index` removed; state-only persistence).
- `ProjectState` now includes `created_at` and uses atomic save (tempfile + rename), with legacy fallback logic implemented in summary derivation.
- Conformance additions for acceptance edge cases are present and passing (`no_index_json`, legacy migration, stale active project, corrupt active project).
