---
artifact: final-review-proposals
loop: 10
project: task-analysis-this-is-a-comprehensive-gi
backend: codex
role: final_reviewer
created_at: 2026-02-23T02:11:49Z
---

# Final Review: AMENDMENTS

## Amendment: FR-MODEL-RESOLUTION-001

### Problem
Final-review backends are not role-resolved before execution, so bare specs (for example `gemini`) do not pick up role models like `final_reviewer`/`arbiter`.

Evidence:
- Reviewers/arbiter are canonicalized from config without role resolution in `src/workflow/orchestrator.rs:3273`, `src/workflow/orchestrator.rs:3282`, `src/workflow/orchestrator.rs:3728`.
- Execution uses `get_or_create_for_role` with those bare specs in `src/workflow/orchestrator.rs:3310` and `src/workflow/orchestrator.rs:3572`.
- `get_or_create_for_role` does not inject role models (`src/backend/mod.rs:872`, `src/backend/mod.rs:1186`) even though `resolve_backend_for_role` exists (`src/backend/mod.rs:930`).
- Gemini role defaults are explicitly configured for these roles (`src/config/global.rs:768`, `src/config/global.rs:769`) but are skipped in this path.

### Proposed Change
Resolve reviewer and arbiter specs through role-model resolution before availability checks, snapshotting, artifact naming, and invocation:
- reviewers with role `final_reviewer`
- arbiter with role `arbiter`

Then keep status/progress lookup aligned with the same resolved spec set.

### Affected Files
- `src/workflow/orchestrator.rs` - resolve final-review reviewers/arbiter with role models before use.
- `src/cli/status.rs` - use the same resolved reviewer specs when locating final-review artifacts.
- `tests/orchestrator.rs` - add regression coverage for bare final-review specs using role-model defaults.

## Amendment: FR-COMPLETER-OVERRIDE-002

### Problem
`--completer-backend` / `workflow.completer_backend` is wired through config/CLI but is effectively ignored by the completion panel orchestration.

Evidence:
- CLI still exposes the flag in `src/cli/mod.rs:125`.
- Override is passed into effective config/role overrides in `src/workflow/orchestrator.rs:185`, `src/workflow/orchestrator.rs:199`.
- Registry still computes override-based completion assignment in `src/backend/mod.rs:1008`.
- Completion path discards the computed completer and always uses `completion_backends` (`src/workflow/orchestrator.rs:636`, `src/workflow/orchestrator.rs:642`), only keeping planner from `base_backends`.

### Proposed Change
Restore explicit precedence instead of silent no-op:
- If `completer_backend` is set, honor it (single-completer panel), or
- Explicitly reject it with a clear validation/deprecation error.

Do not silently accept and ignore the setting.

### Affected Files
- `src/workflow/orchestrator.rs` - apply or reject `completer_backend` deterministically.
- `src/config/mod.rs` - add validation for override/panel conflicts if needed.
- `src/validate/tests_completion_panel.rs` - add conformance test proving override behavior (or explicit rejection).
