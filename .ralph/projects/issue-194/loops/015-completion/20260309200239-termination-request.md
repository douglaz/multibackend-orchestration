---
artifact: termination-request
loop: 15
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T20:02:39Z
---

All three Round-5 amendments have been addressed and all stray files are cleaned up. Given that all acceptance criteria are met, all 5 rounds of final review amendments have been resolved, and the Round-5 reviewer confirmed the implementation is "correct, safe, and well-structured across all major concerns," this project is complete.

# Project Completion Request

## Rationale
All acceptance criteria from the master prompt are satisfied, and all final-review amendments through Round 5 have been implemented and verified:

1. **Typed amendment model** — `AmendmentRequest`, `AmendmentPriority` (default P2), `AmendmentSource` with serde defaults.
2. **CLI `ralph amend`** — enqueues via atomic temp-then-hard_link with `@path` body, priority validation, default ID.
3. **Multiple pending files** — queue directory supports concurrent enqueue with suffix collision handling.
4. **Standard orchestrator** — drains at Planning boundary, injects `external_amendments`, filters final-review when unified.
5. **Quick-dev orchestrator** — drains at PlanAndImplement boundary, appends `## External Amendments` section.
6. **Crash-safe drain** — `.json`→`.inflight` claim, `.inflight` recovery, mid-drain rollback on fatal error.
7. **Malformed quarantine** — content errors (bad JSON, invalid UTF-8) quarantined; only true I/O errors are fatal.
8. **Completion guard** — both planning-phase and late-stage guards block completion when pending amendments exist.
9. **Unify final-review** — `amendments.unify_final_review` defaults `false`; opt-in enqueues accepted amendments with dedupe filtering.
10. **Conformance tests** — 20 conformance tests in `src/validate/tests_amendments.rs` covering CLI, injection, guards, rollback, and config.

## Summary of Work
- **Loop 1**: Core data model and queue functions (`src/project/amendments.rs`)
- **Loop 2**: CLI command (`src/cli/amend.rs`, `src/cli/mod.rs`)
- **Loop 3**: Standard and quick-dev orchestrator integration
- **Loop 4**: Final-review unification config and behavior
- **Loop 6**: Late-phase completion guard and drain-order test enforcement (FR-194-001, FR-194-002)
- **Loop 8**: At-least-once amendment preservation on phase failure (A-194-REVIEW-001)
- **Loop 10**: Lossless mid-drain recovery and failure-path test hardening (AMEND-QUEUE-LOSS-001, AMEND-TEST-SEMANTICS-002)
- **Loop 12**: Round-4 remediation — read vs parse error separation, stray artifact cleanup and detection hardening
- **Loop 14**: Round-5 hardening — stem-race dedup safety, non-UTF8 quarantine, remaining stray file cleanup

## Remaining Items
- None — all spec requirements and reviewer amendments have been addressed.

---
