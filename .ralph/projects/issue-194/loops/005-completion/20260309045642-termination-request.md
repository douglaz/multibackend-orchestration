---
artifact: termination-request
loop: 5
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T04:56:42Z
---

# Project Completion Request

## Rationale
All acceptance criteria from the master prompt have been satisfied across the four completed loops. Every required file has been created or modified, all functions are implemented, and conformance tests cover the full feature surface.

## Summary of Work

- **Loop 1 — Amendment Data Model and Queue Functions:** Implemented `AmendmentRequest`, `AmendmentPriority`, `AmendmentSource` types with serde defaults in `src/project/amendments.rs`. Delivered `enqueue_amendment`, `drain_amendment_queue`, `pending_amendment_count`, and `format_external_amendments_for_prompt` with crash-safe two-phase drain (`.inflight` rename), quarantine on malformed JSON, and 17 unit tests.
- **Loop 2 — CLI Amendment Intake Command:** Added `ralph amend` in `src/cli/amend.rs` with `--project`, `--body` (including `@path` expansion), `--priority`, and `--id` flags. Integrated into `src/cli/mod.rs`. Added conformance tests in `src/validate/tests_amendments.rs`.
- **Loop 3 — Standard Orchestrator Amendment Integration:** Modified `src/workflow/orchestrator.rs` to drain the queue at `Phase::Planning`, inject `external_amendments` into `build_planner_prompt`, and block `CompletionRequest` when pending amendments exist. Integrated quick-dev orchestrator drain and prompt injection in `src/workflow/quick_dev_orchestrator.rs`. Added conformance tests for standard planner injection, quick-dev injection, and completion guard rejection.
- **Loop 4 — Final-Review Unification and Typed Amendments Config:** Added `AmendmentsConfig` with `unify_final_review: bool` (default `false`) to global and project config with merge precedence. When enabled, accepted final-review amendments are mirrored into the queue with `source = FinalReview`. Planning-phase deduplication excludes `source == FinalReview` items from the external amendments prompt block. Added conformance tests for default-off behavior, deduplication, and mirroring.

## Remaining Items
- None. All acceptance criteria are met and all required tests are in place.

---
