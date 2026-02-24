---
artifact: termination-request
loop: 4
project: add-a-final-review-gate-after-acceptance
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-02-20T20:32:47Z
---

# Project Completion Request

## Rationale
All requirements in `prompt.md` are already satisfied by completed Loops 1-3 and implemented in the current codebase. The FinalReview phase, config/override/effective-resolution rules, role/template extensions, fail-closed parsers, orchestrator transitions and resumable flow, restart-count derivation from checkpoints, force-complete behavior, planner amendment injection, CLI status/history output, and validate coverage are all present. Targeted integration and validate tests for final review pass.

## Summary of Work
Implemented the complete FinalReview gate after acceptance QA, including:
- New `FinalReview` phase lifecycle/state handling and labels.
- Full final-review workflow config + validation semantics.
- `final_reviewer`/`arbiter` backend role model and timeout support.
- Final-review templates and parser contracts (fail-closed).
- Artifact-resumable FinalReview orchestration with consensus, arbiter dispute resolution, restart flow, config-mismatch invalidation, and restart-cap force completion.
- Planner prompt amendment-context injection.
- CLI final-review progress reporting.
- Integration tests and registered validate conformance tests for final-review flows.

## Remaining Items
None
