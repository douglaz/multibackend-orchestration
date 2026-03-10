---
artifact: termination-request
loop: 11
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T18:28:23Z
---

# Project Completion Request

## Rationale
All 10 acceptance criteria from the master prompt are fully implemented, tested, and reviewed across 7 feature loops (1–4, 6, 8, 10). Three prior completion attempts (loops 5, 7, 9) were deferred to address final-review amendments; all such amendments have now been resolved.

## Summary of Work
- **Loop 1**: Typed amendment data model (`AmendmentRequest`, `AmendmentPriority`, `AmendmentSource`) with serde defaults; `enqueue_amendment`, `drain_amendment_queue`, `pending_amendment_count`, `format_external_amendments_for_prompt` in `src/project/amendments.rs`; unit tests (22 passing).
- **Loop 2**: `ralph amend` CLI command in `src/cli/amend.rs` with `--project`, `--body` (including `@path`), `--priority`, `--id`; atomic temp-then-rename enqueue; integration tests in `tests/amend_cli.rs` (3 passing).
- **Loop 3**: Standard orchestrator integration — drain at `Phase::Planning`, inject `external_amendments` into planner prompt with fallback `## External Amendments` section, completion guard before `PlannerDecision::CompletionRequest`.
- **Loop 4**: Final-review unification opt-in (`amendments.unify_final_review` config, default `false`); accepted final-review amendments mirrored to queue with `source = FinalReview`; planning-time deduplication; global/project config merge.
- **Loop 6**: Late-stage completion queue guard before final completed return path (FR-194-001); exact drain-order sequence assertion in CLI tests (FR-194-002); conformance test coverage for late-phase amendment blocking.
- **Loop 8**: At-least-once amendment preservation on phase failure (A-194-REVIEW-001) — drained amendments re-enqueued on downstream errors in both standard and quick-dev orchestrators; regression tests for drain+failure persistence.
- **Loop 10**: Lossless mid-drain recovery with internal rollback on fatal IO errors (AMEND-QUEUE-LOSS-001); tightened `quick_dev_checkpoint_failure_no_rollback_after_durable_success` test assertions (AMEND-TEST-SEMANTICS-002); 19 conformance tests passing.

Quick-dev integration drains at `PlanAndImplement` phase and appends `## External Amendments` to implementer prompt. Malformed files are quarantined to `.quarantine/` with warnings. Crash-safe `.inflight` recovery handles stale files from prior interrupted drains.

## Remaining Items
- `[P2]` Remove workflow artifact files from branch root: `20260309T165947Z-impl-notes.md` and `20260309T165947Z-impl-response-001.md` (AMQ-001). Non-blocking cleanup; does not affect functionality or test coverage.

---
