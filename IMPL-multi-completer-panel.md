# Implementation Notes: Multi-Completer Completion Panel (Phase 2)

## Summary

Replaced the single-completer completion flow with a panelized multi-completer system. Multiple backends now independently assess project completeness, and a configurable consensus algorithm determines the final verdict.

## Changes by File

### src/config/global.rs
- Added 3 new `WorkflowConfig` fields:
  - `completion_backends: Vec<String>` (default: `["claude", "codex", "?gemini"]`)
  - `completion_min_completers: u32` (default: `2`)
  - `completion_consensus_threshold: f64` (default: `1.0`)
- Added corresponding default functions and `Default` impl entries

### src/config/mod.rs
- Added 3 fields to `EffectiveWorkflowConfig`
- Added merge logic in `resolve_effective_config` (project override → global fallback)
- Added `validate_completion_panel_config()` with checks for:
  - Non-empty backends list
  - `min_completers >= 1`
  - Threshold bounds `(0.0, 1.0]`
  - Canonical deduplication of backend specs
  - Filename collision prevention via `completion_verdict_filename()`
- Refactored `normalize_backend_specs` into `normalize_backend_specs_labeled` for reuse

### src/config/project.rs
- Added 3 optional fields to `ProjectWorkflowOverrides`

### src/project/state.rs
- Changed `CompletionLoopBackends` from `{ planner, completer }` to `{ planner, completers: Vec<String> }`
- Added `CompletionLoopBackends::new()` constructor
- Implemented custom `Deserialize` that promotes legacy `completer` field to `completers` vec automatically

### src/project/artifacts.rs
- Added `CompleterVerdictBackend { backend: String }` variant to `ArtifactKind`
- `file_name()` generates `completer-verdict-{slugified_backend}.md`
- `base_type()` returns `"completer-verdict"`
- Fixed `slugify_backend` to trim trailing dashes from parenthesized specs

### src/backend/mod.rs
- Updated `assign_completion_backends` to use `CompletionLoopBackends::new(planner, vec![completer])`
- Added `resolve_completion_panel()` async method:
  - Iterates `completion_backends` config list
  - Handles `?optional` backends: unavailable → skip (error or health check failure)
  - Required backends: unavailable → error
  - Validates `effective.len() >= min_completers`
  - Returns resolved backend spec list

### src/project/lifecycle.rs
- Rewrote `reconstruct_completion_attempt()` to handle both:
  - Legacy single verdict (`completer-verdict.md`) → maps to single completer
  - New per-backend verdicts (`completer-verdict-{backend}.md`) → maps to panel layout
- Uses `CompletionLoopBackends::new()` instead of struct literal

### src/workflow/orchestrator.rs
- Registration path (`PlannerDecision::CompletionRequest`):
  - Calls `resolve_completion_panel()` to get effective completers from config
  - Falls back to `assign_completion_backends` single completer if panel resolution fails
  - Stores full completers list in `CompletionLoopBackends`
- Execution path (`Phase::Completing`):
  - Iterates over `completion.backends.completers`
  - Invokes each completer backend independently
  - Writes per-backend verdict artifacts (`CompleterVerdictBackend` for multi, `CompleterVerdict` for single)
  - Computes consensus: `complete_votes >= min_completers && complete_votes/total >= threshold`
  - Runs acceptance QA exactly once after panel verdict

### src/cli/project.rs, src/cli/status.rs
- Updated display to show `completers=[...]` instead of `completer=...`

### src/workspace/mod.rs, src/workspace/summary.rs
- Updated test helpers to use `CompletionLoopBackends::new()`

### tests/state.rs, tests/backend.rs, tests/orchestrator.rs
- Updated all struct literals to use `CompletionLoopBackends::new()`
- Updated `.completer` field accesses to `.completers[0]`
- Updated verdict artifact assertions for panel naming
- Added `completion_backends` config to all test setup functions

### src/validate/tests_completion_panel.rs (NEW)
- `two_completer_consensus_complete`: 2 completers both COMPLETE → consensus
- `single_completer_backward_compat`: single completer → legacy behavior
- `panel_continue_verdict`: split votes → CONTINUE verdict
- `per_backend_verdict_artifacts`: verifies per-backend artifact files exist

### src/validate/mod.rs
- Registered `tests_completion_panel` module

## Consensus Math

```
consensus_reached = complete_votes >= min_completers
    AND total_completers > 0
    AND (complete_votes / total_completers) >= consensus_threshold
```

Both thresholds are inclusive (>=).

## Backward Compatibility

- Legacy `CompletionLoopBackends` JSON with `completer` field auto-promotes to `completers` vec via custom `Deserialize`
- Legacy `completer-verdict.md` artifacts are recognized during reconstruction
- Single-completer panels use legacy `ArtifactKind::CompleterVerdict` artifact name
- Optional `?backend` syntax in `completion_backends` allows graceful degradation

## Build Status

- `cargo check`: PASS
- `cargo test`: 925+ tests PASS (0 failures)
- `nix build`: pre-existing failure in `daemon::runtime_artifact_comments_posted` (unrelated to this change)
