---
artifact: final-review-proposals
loop: 9
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T17:49:46Z
---

Confirmed. The stray files were committed during Loop 8. Now let me produce the final review.

# Final Review: AMENDMENTS

## Summary

The implementation is well-structured and largely correct. The amendment data model, queue functions, CLI command, orchestrator integrations (standard and quick-dev), config plumbing, crash-safe drain semantics, rollback-on-failure logic, and test coverage are all solid.

**Verified files:**
- `src/project/amendments.rs` — data model, enqueue (atomic temp-then-rename via hard_link), drain (claim via inflight, dedup on same-stem, quarantine malformed), rollback, pending count, formatter. All correct. Crash-safe semantics hold: inflight recovery, dedup between `.json`+`.inflight` pairs, temp files excluded.
- `src/cli/amend.rs` + `src/cli/mod.rs` — CLI parsing, `@path` body loading, priority validation, auto-generated ID. Correct.
- `src/config/global.rs`, `src/config/project.rs`, `src/config/mod.rs` — `AmendmentsConfig` with `unify_final_review`, proper global/project merge precedence. Config get/set both wired. Correct.
- `src/workflow/orchestrator.rs` — Drain at planning boundary, `drained_for_rollback` captures full set before unify-filter, all error paths between drain and durable state commit (`register_feature_loop`/`register_completion_attempt`) wrapped with `rollback_drained_amendments`, pending guard blocks CompletionRequest, late guard catches post-planning arrivals. Correct. Both `register_feature_loop` and `register_completion_attempt` set status to `InProgress` (confirmed in `src/project/state.rs:349,384`), so the late guard does not create a stuck state.
- `src/workflow/quick_dev_orchestrator.rs` — Drain after pre-commit feedback, rollback on all error paths before `save_state_to_disk`, no rollback after durable success. Split between `persist_quick_dev_state` + `save_state_to_disk` + `checkpoint_if_enabled` is correctly factored. Correct.
- `src/validate/tests_amendments.rs` — 15 conformance tests covering enqueue, default priority, invalid priority rejection, `@file` body, prompt injection (standard + quick-dev), completion guard (planning + late), rollback preservation (standard + quick-dev), durable-success no-rollback invariant, unify config. Comprehensive.
- `tests/amend_cli.rs` — 3 integration tests (enqueue+drain, multi-drain order, nonexistent project rejection). Correct.
- Unit tests in `src/project/amendments.rs` — 18 tests covering naming, dedup, inflight recovery, quarantine, ordering, temp file exclusion, roundtrip, re-enqueue. Thorough.

One issue found:

## Amendment: AMQ-001

### Problem
Two implementation artifact files from Loop 8 were committed to the branch root:
- `20260309T165947Z-impl-notes.md`
- `20260309T165947Z-impl-response-001.md`

These are workflow-generated response files (committed in `b112058` and `6a606dc`) and do not belong in the source tree. They would be included in any merge to master.

### Proposed Change
`[P2]` Remove both files and commit the deletion.

### Affected Files
- `20260309T165947Z-impl-notes.md` — delete
- `20260309T165947Z-impl-response-001.md` — delete

---
