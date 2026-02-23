---
artifact: final-review-proposals
loop: 6
project: task-analysis-this-is-a-comprehensive-gi
backend: codex
role: final_reviewer
created_at: 2026-02-23T00:34:57Z
---

# Final Review: AMENDMENTS

## Amendment: FR-PR-001

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

## Amendment: FR-PR-002

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
