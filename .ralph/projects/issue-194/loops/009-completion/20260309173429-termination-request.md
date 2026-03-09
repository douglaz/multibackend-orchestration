---
artifact: termination-request
loop: 9
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T17:34:29Z
---

# Project Completion Request

## Rationale
All acceptance criteria from the master prompt are fully satisfied across 6 completed implementation loops. Every required function, integration point, configuration option, and test suite has been implemented and verified in the codebase.

## Summary of Work
- **Loop 1** – Amendment data model (`AmendmentRequest`, `AmendmentPriority`, `AmendmentSource`) with serde defaults, and all queue functions (`enqueue_amendment`, `drain_amendment_queue`, `pending_amendment_count`, `format_external_amendments_for_prompt`) with crash-safe `.inflight` semantics, quarantine, and atomic temp-then-rename handoff.
- **Loop 2** – `ralph amend` CLI command with `--project`, `--body` (including `@file`), `--priority`, and `--id` flags; input validation; active-project resolution; and CLI integration tests.
- **Loop 3** – Standard orchestrator drains the amendment queue at the start of `Phase::Planning`, formats amendments into the planner prompt via `external_amendments`, and uses `append_section_if_missing` as fallback.
- **Loop 4** – `amendments.unify_final_review` config (default `false`) in global and project configs with merge precedence; opt-in final-review mirroring to the amendment queue; planning-phase dedup filter excluding `FinalReview`-sourced items when enabled.
- **Loop 6** – Late-phase completion queue guard before the final completed return path; deterministic drain-order test enforcement with exact sequence assertions; conformance test for amendment arrival during completing/final-review windows.
- **Loop 8** – At-least-once amendment preservation via `rollback_drained_amendments` on phase failure in both standard and quick-dev orchestrators; no duplicate re-enqueue after durable state persistence; regression tests for all failure modes.

## Remaining Items
- None — all master-prompt acceptance criteria, final-review amendments (FR-194-001, FR-194-002, A-194-REVIEW-001), and required test suites are implemented.

---
