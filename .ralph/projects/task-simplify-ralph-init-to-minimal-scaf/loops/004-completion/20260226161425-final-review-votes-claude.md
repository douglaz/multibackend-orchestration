---
artifact: final-review-votes
loop: 4
project: task-simplify-ralph-init-to-minimal-scaf
backend: claude
role: final_reviewer
created_at: 2026-02-26T16:14:25Z
---

I've now thoroughly verified all the code paths. The bug chain is clear and confirmed. Let me write my vote.

# Vote Results

## Amendment: FR-20260226-01

### Vote
ACCEPT

### Rationale
I independently verified every link in the bug chain against the source code and confirm the amendment is correct:

1. **`key_becomes_none` removes the key** (line 1227-1235): When `raw_value == "none"` for `workflow.planner_max_prior_loops`, `key_becomes_none` returns `true`, causing `remove_document_key` to strip the key from the TOML file entirely (line 1216-1217).

2. **Missing key falls back to `Some(10)`** (line 914-916): `default_planner_max_prior_loops()` returns `Some(10)`, which is used by serde when the key is absent from TOML (via `#[serde(default = "default_planner_max_prior_loops")]` at line 379).

3. **CLI reload completes the lossy round-trip** (line 309-312): `execute_set` calls `save_global_value_sparse` (which removes the key) then immediately reloads from disk via `GlobalConfig::load`, so the in-memory state snaps back to `Some(10)`.

4. **The table-driven test masks this**: At lines 3694-3699, the test applies `save` + `load` normalization to the *expected* config on every iteration. When `("workflow.planner_max_prior_loops", "none")` is applied, `expected` is set to `None` in memory, then saved (which also loses the key), then reloaded (which also defaults to `Some(10)`). So `expected` matches `actual` — both are wrong. The test passes because the lossy round-trip is applied symmetrically to both sides.

5. **The fix is well-scoped**: Custom serde or an explicit sentinel value in the TOML (e.g., `planner_max_prior_loops = "none"`) is the correct approach to distinguish "unlimited" from "absent/default." The proposed regression tests (unit sparse round-trip + CLI conformance) directly target the specific failure mode.

This is a genuine correctness bug that silently discards user configuration. The fix scope is narrow, the affected files are correctly identified, and the testing plan addresses both the immediate bug and the structural test weakness that hid it.
