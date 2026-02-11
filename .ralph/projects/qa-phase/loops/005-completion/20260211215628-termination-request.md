---
artifact: termination-request
loop: 5
project: qa-phase
backend: claude(opus)
role: planner
created_at: 2026-02-11T21:56:28Z
---

# Project Completion Request

## Rationale
All four implementation phases from the master plan have been completed across four feature loops:

1. **Phase 1 - Schema and plumbing** (Loop 1: "QA Schema and Config Plumbing"): Added `Phase::QA`, QA artifact kinds, QA fields on state/config/backend structs, CLI `--qa-backend` flag, config key plumbing, `QaIterationLimitExceeded` error, and backward compatibility with existing state/config files.

2. **Phase 2 - QA parser/template + orchestrator loop gate** (Loop 2: "QA Runtime Gate and Feedback Loop"): Added `QaDecision` parser with pass/fail contract validation, `default_qa_template()`, `templates/qa.md` init wiring, `Phase::QA` orchestrator arm with feedback loop, implementer QA-response handling, iteration-limit enforcement, parse-retry support, and end-to-end orchestrator tests.

3. **Phase 3 - Acceptance gate** (Loop 3: "QA Acceptance Gate for Completion Phase"): Added acceptance-time QA execution in `Phase::Completing`, `AcceptancePass`/`AcceptanceFail` artifact persistence, forced `Continue` on acceptance failure routing back to planning, and planner-context injection for acceptance-failure details.

4. **Phase 4 - UX and documentation** (Loop 4: "QA UX Reporting and Documentation Finalization"): Added QA result summaries in `ralph status`, QA metadata in `ralph history --verbose`, `PLAN.md` documentation of QA workflow/config surface, and integration tests for status/history QA rendering.

All five Definition of Done criteria are satisfied:
1. QA can be enabled per project and globally.
2. QA executes between implementation and review, with retry loop on failures.
3. QA acceptance gate can veto completer `COMPLETE` and route back to planning.
4. All existing non-QA behavior remains unchanged when QA is disabled.
5. Added tests cover parser, config precedence, backend assignment, orchestrator transitions, and compatibility.

## Summary of Work
- **Loop 1** (committed `afdff6e`): Foundation data model — `Phase::QA`, `QaExchange`, QA artifact kinds, config/backend/CLI plumbing, error variant, serde backward compatibility.
- **Loop 2** (committed `0521c9c`): Runtime QA gate — parser with strict H1/H2 contract, QA prompt template, orchestrator QA phase with pass/fail/feedback-loop transitions, iteration limit with rollback, 5 orchestrator integration tests.
- **Loop 3** (committed `be3f9f6`): Completion acceptance gate — acceptance QA execution, artifact persistence, forced-Continue on failure, planner context injection, 3 orchestrator integration tests.
- **Loop 4** (committed `f530ae9`): UX polish — `ralph status` QA summaries, `ralph history --verbose` QA metadata, `PLAN.md` documentation, status/history integration tests.

## Remaining Items
- None — all requirements from the master plan are satisfied.
