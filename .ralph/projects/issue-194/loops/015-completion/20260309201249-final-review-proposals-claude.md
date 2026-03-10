---
artifact: final-review-proposals
loop: 15
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T20:12:49Z
---

I've completed a thorough review of all 14 changed files, tracing through the data model, CLI, queue operations, orchestrator integration, config wiring, and all test suites. Let me summarize my findings.

# Final Review: NO AMENDMENTS

## Summary

The external amendment queue implementation is correct, safe, and complete. I verified the following across the source code:

**Data Model** (`src/project/amendments.rs:21-97`): `AmendmentRequest`, `AmendmentPriority` (Default=P2), and `AmendmentSource` are correctly defined with serde defaults. Validation rejects empty `id` and `body`.

**Atomic Enqueue** (`src/project/amendments.rs:111-169`): Uses `write_payload_to_temp_file` (O_CREAT|O_EXCL via `create_new`) then `claim_file_without_overwrite` (hard_link for no-overwrite semantics + remove source). Correctly skips stems occupied by `.inflight` siblings. Suffix collision is resolved by incrementing.

**Crash-safe Drain** (`src/project/amendments.rs:171-326`): Lists `*.json` and `*.inflight` sorted lexicographically. Claims `.json` -> `.inflight` before reading. Handles same-stem dedup (content comparison). Quarantines malformed files while continuing to drain valid ones. I/O read failures are fatal with mid-drain rollback (`rollback_mid_drain` re-enqueues already-drained items). `NotFound` on rename races are correctly skipped.

**Orchestrator Integration** (`src/workflow/orchestrator.rs:604-635`): Drains at start of Planning phase. Filters out `FinalReview`-source amendments when `unify_final_review` is enabled. Formats and injects via `external_amendments` template variable with `append_section_if_missing` fallback. All error paths wrapped in `rollback_drained_amendments`.

**Completion Guard** (`src/workflow/orchestrator.rs:749-758`): Checks `pending_amendment_count` before honoring `CompletionRequest`. Late guard at line 2832 catches amendments arriving during completing/final-review phases.

**Quick-Dev Integration** (`src/workflow/quick_dev_orchestrator.rs:347-409`): Drains after pre-commit feedback injection. Appends `## External Amendments` section. All error paths wrapped in rollback. Rollback stops after durable state persistence (line 411 comment and `checkpoint_if_enabled` outside rollback scope).

**Final-Review Unification** (`src/workflow/orchestrator.rs:4390-4407`): When `unify_final_review` is true, accepted amendments are enqueued as `AmendmentRequest` with `source=FinalReview`, `source_detail=reviewer_backend`, `priority=P2`. During planning, these are excluded from `external_amendments_text` to avoid duplication.

**Config** (`src/config/global.rs:27-35`, `src/config/mod.rs:462-465`): `AmendmentsConfig.unify_final_review` defaults to `false`. Project-level `Option<bool>` overrides global, matching documented merge precedence. Config get/set wired in `src/cli/config.rs`.

**CLI** (`src/cli/amend.rs`, `src/cli/mod.rs:54-67`): `ralph amend` with `--project`, `--body` (required, supports `@path`), `--priority` (default P2), `--id` (default `EXT-<timestamp>`). Validates priority before enqueue. Checks project existence before creating queue directory.

**Tests**: 25 unit tests in `amendments.rs` (enqueue naming, drain ordering, inflight recovery, same-stem dedup, malformed quarantine, mid-drain rollback, validation failure quarantine, field preservation). 7 CLI unit tests. 3 integration tests in `tests/amend_cli.rs`. 18 conformance tests in `src/validate/tests_amendments.rs` covering CLI behavior, planner/quick-dev injection, completion guard, late guard, unify config, dedupe, mirroring, failure rollback, checkpoint-failure-no-rollback, and malformed-queue orchestration.

**No stray files**: The only untracked file (`.ralph/projects/issue-194/loops/015-completion/final-review-config.json`) is inside the `.ralph` orchestration directory, which is excluded from the diff scope.

The `src/git/commit.rs` changes (ISO-basic timestamp handling for stray artifact cleanup) are a legitimate bugfix for a related artifact-naming edge case and are well-tested.
