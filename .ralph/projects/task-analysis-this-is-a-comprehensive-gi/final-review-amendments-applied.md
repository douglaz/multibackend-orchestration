# Final Review Amendments Applied

## Round 1

### Amendment: A1

### Problem
Two stray implementation artifact files were committed to the repository root and are not project deliverables.

### Proposed Change
Remove the stray files from the repository history tip by deleting them in a follow-up commit:
`git rm 20260222T223018Z-impl-response-III.md IMPL-multi-completer-panel.md`

### Affected Files
`20260222T223018Z-impl-response-III.md`  
`IMPL-multi-completer-panel.md`

### Reviewer
claude

### Amendment: FR-20260222-PR-ALIAS-PRECEDENCE

### Problem
`resolve_effective_config` treats global `prompt_review_backends` as "explicit" only when its value differs from defaults (`src/config/mod.rs:182`, `src/config/mod.rs:183`, `src/config/mod.rs:189`).  
That is value-based, not presence-based, and breaks the alias contract:

1. If `prompt_review_backends` is explicitly set to the default value, it is treated as unset and the singular alias path is used (`src/config/mod.rs:192`, `src/config/mod.rs:194`), violating "if `prompt_review_backends` is set, use it."
2. Project singular alias overrides can be ignored when global plural is non-default, because project singular is only consulted in the fallback branch.

Defaults that trigger this ambiguity are defined at `src/config/global.rs:979` and `src/config/global.rs:983`.

### Proposed Change
Use explicit key presence (not value inequality) for global plural alias resolution, and apply precedence as:

1. project `prompt_review_backends` (if set)
2. else project `prompt_review_backend` (if set)
3. else global `prompt_review_backends` (if explicitly set)
4. else synthesize from global `prompt_review_backend`

Add regression tests for:
1. explicit global plural equal-to-default still winning over singular
2. project singular override behavior when global plural is set

### Affected Files
- `src/config/mod.rs` - fix precedence logic and add coverage for alias precedence edge cases.
- `src/config/global.rs` - preserve/propagate explicit presence signal for `workflow.prompt_review_backends` at load time.

### Reviewer
codex


## Round 2

### Amendment: FR-PR-001

### Problem
Optional backend semantics are not applied correctly when the first prompt-review backend is unavailable.  
`src/workflow/orchestrator.rs:289` picks only the first configured backend as refiner, and `src/workflow/orchestrator.rs:312` to `src/workflow/orchestrator.rs:319` marks prompt review completed when that first backend is optional and unavailable.  
This bypasses the rest of `prompt_review_backends` instead of skipping the unavailable optional backend and continuing with remaining backends, which conflicts with panel-list optional behavior validated in `src/config/mod.rs:783`.

### Proposed Change
Resolve the prompt-review backend list first (optional skip, required fail), then run prompt review using:
1. First effective backend as refiner.
2. Remaining effective backends as serial validators.
3. Error if no effective backend remains after filtering.

Add a conformance test for `prompt_review_backends=["?gemini","claude"]` with gemini unavailable to ensure Claude is used as refiner (not full prompt-review skip).

### Affected Files
- `src/workflow/orchestrator.rs` - resolve effective prompt-review backend list before selecting refiner.
- `src/validate/tests_prompt_review_panel.rs` - add regression coverage for optional-first backend skip behavior.

### Reviewer
codex

### Amendment: FR-PR-002

### Problem
Prompt-review side effects happen before the `prompt-original.md` safety guard, creating false "completed" reconstruction states.  
`src/workflow/orchestrator.rs:367` writes `prompt-review.md` before checking whether `prompt-original.md` already exists (`src/workflow/orchestrator.rs:506`).  
If `prompt-original.md` pre-exists, run fails, but both files now exist; reconstruction then marks prompt review completed via `src/project/lifecycle.rs:962` and `src/project/lifecycle.rs:309`, even though prompt rewrite never succeeded.

### Proposed Change
Move the `prompt-original.md` existence guard to run before any prompt-review artifact writes or validator execution.  
Keep `prompt-review.md` emission only after guard passes and prompt update path is valid.  
Add regression coverage to verify existing `prompt-original.md` causes a clean failure without writing `prompt-review.md`.

### Affected Files
- `src/workflow/orchestrator.rs` - reorder guard and artifact write flow for prompt review.
- `src/validate/tests_prompt_review_panel.rs` (or prompt-review conformance module) - add failure-path regression test.

### Reviewer
codex


## Round 3

### Amendment: FR-DAEMON-REFINEMENT-GUARDRAIL-002

### Problem
Gemini is not fully guardrailed for daemon refinement when configured via project override.

Project-level override exists (`src/config/project.rs:95`) and can be set without guardrail rejection (`src/cli/config.rs:811`). The merged daemon config uses that override (`src/config/mod.rs:439`), but daemon startup validates only workspace-level refinement backend (`src/cli/daemon.rs:167` calling `src/config/mod.rs:552`). So project override can bypass the intended Gemini rejection for daemon refinement. Failure is deferred to runtime refine backend creation (`src/daemon/refine.rs:53`, `src/daemon/refine.rs:56`) as "unknown refinement backend," not explicit guardrail validation.

### Proposed Change
Validate the effective daemon refinement backend (post-merge) as a required non-Gemini surface:
- Add validation against `daemon_cfg.refinement_backend` after `resolve_daemon_config` in daemon startup.
- Centralize this in config validation (effective daemon config validator) so all call paths are consistent.
- Add tests for project override rejection (`daemon.refinement_backend = gemini(...)`) in daemon startup/config validation.

### Affected Files
- `src/cli/daemon.rs` - validate merged daemon refinement backend, not only workspace raw config.
- `src/config/mod.rs` - add effective daemon refinement validation helper and tests.
- `src/validate/tests_gemini_backend.rs` - add conformance case for project-level daemon refinement guardrail.

### Reviewer
codex

### Amendment: FR-PR-ALIAS-OPTIONAL-001

### Problem
`?backend` is currently accepted through the singular prompt-review alias surface, which violates the constraint that optional syntax is invalid on single-backend required surfaces.

The singular alias is accepted by config set (`src/cli/config.rs:671`, `src/cli/config.rs:990`), then promoted into the panel list (`src/config/mod.rs:186`). During validation, the spec is canonicalized before validation (`src/config/mod.rs:647`), and canonicalization strips the optional marker (`src/config/mod.rs:817`), so the optional-surface check in `validate_backend_spec` (`src/config/mod.rs:497`) is bypassed for this path.

I confirmed this behavior via CLI: setting `workflow.prompt_review_backend` to `?gemini` succeeds and resolves as `["?gemini"]`.

### Proposed Change
Reject optional syntax on the singular alias before alias synthesis/canonicalization. Concretely:
- Validate `workflow.prompt_review_backend` as a required surface (`ValidationSurface::Required`) when present.
- In list normalization, validate the raw spec first (to preserve `optional`), then canonicalize for dedupe/model normalization.
- Add tests that `workflow.prompt_review_backend = "?gemini"` fails for both global/project config paths.

### Affected Files
- `src/config/mod.rs` - enforce required-surface validation for singular alias; adjust normalization order.
- `src/cli/config.rs` - ensure singular-key set path rejects optional syntax consistently.
- `src/validate/tests_prompt_review_panel.rs` - add conformance coverage for singular-alias optional rejection.

### Reviewer
codex

