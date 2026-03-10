---
artifact: completer-verdict
loop: 13
project: issue-194
backend: claude(opus)
role: completer
created_at: 2026-03-09T19:13:59Z
---

All 29 unit tests pass. Now I have all the information needed to render a verdict. Let me cross-check each acceptance criterion:

# Verdict: COMPLETE

The project satisfies all requirements:

- **Typed amendment model + serde defaults**: `AmendmentRequest` struct with `#[serde(default)]` on `priority` (defaults to `P2`) and `source_detail`. `AmendmentPriority` enum (P0-P3, Default=P2), `AmendmentSource` enum (Cli, FinalReview, File). Validation enforces non-empty `id` and `body`. `created_at` uses `DateTime<Utc>` (RFC3339). All in `src/project/amendments.rs`.

- **`ralph amend` enqueues valid JSON via temp-then-rename**: CLI in `src/cli/amend.rs` with `--project` (optional), `--body` (required, supports `@path`), `--priority` (optional, default P2), `--id` (optional, default `EXT-<timestamp>`). Enqueue uses `.tmp-<uuid>.json` → `hard_link` to `<timestamp>-<sanitized_id>.json` → delete temp. Registered in `src/cli/mod.rs`.

- **Queue supports multiple pending files**: Filename collision handled by appending `-<n>` suffix. Lexicographic drain ordering ensures deterministic processing.

- **Standard orchestrator drains at planning boundary and injects `external_amendments`**: `drain_amendment_queue` called at start of Planning phase. Filtered through `format_external_amendments_for_prompt` and passed to `build_planner_prompt`. Template variable `external_amendments` supported with fallback `## External Amendments` section.

- **Quick-dev drains at plan/implement boundary and injects amendments**: Drains after pre-commit feedback injection in `PlanAndImplement` phase. Appends `## External Amendments` section using shared formatter.

- **Drain supports `.inflight` recovery and crash-safe semantics**: Lists both `*.json` and `*.inflight`. Renames `.json` → `.inflight` before reading. Already-existing `.inflight` files are processed directly. `NotFound` on rename (race) is skipped. `.inflight` deleted only after successful parse.

- **Malformed files are quarantined with warnings; orchestration continues**: Parse failures and validation failures moved to `.quarantine/` dir with unique names. Warning logged. Drain continues processing remaining files. Read I/O errors trigger mid-drain rollback (fatal), but content parse errors do not.

- **Completion request is blocked while pending amendments exist**: Two guards: (1) before honoring `PlannerDecision::CompletionRequest`, checks `pending_amendment_count` and returns `RalphError::Orchestration` with count; (2) late-stage guard before final success return blocks if amendments arrived during completing/final-review phases.

- **`amendments.unify_final_review` default is `false`; opt-in path works with dedupe**: Config defined in `global.rs` (`AmendmentsConfig.unify_final_review: bool`, default false), project override in `project.rs` (`Option<bool>`), merged in `EffectiveAmendmentsConfig`. When true, final-review source amendments are filtered out of `external_amendments` prompt text to avoid duplication.

- **Validate conformance tests cover new CLI and orchestration behavior**: `src/validate/tests_amendments.rs` registered in `src/validate/mod.rs` with 19+ conformance tests covering: CLI enqueue/defaults/validation/error cases, standard planner drain+injection, quick-dev drain+injection, completion guard rejection, late guard blocking, planning failure amendment preservation, quick-dev failure preservation, durable success no-rollback, unify config default, unify dedup, and unify mirroring.

- **All unit tests pass**: 24 unit tests in `amendments.rs` + 5 in `amend.rs` = 29 total, all passing. Tests cover: enqueue naming/atomic handoff, deterministic drain ordering, post-drain file removal, `.inflight` recovery, malformed JSON quarantine, missing queue behavior, priority defaults, serialization roundtrip, re-enqueue, mid-drain rollback.

- **At-least-once amendment preservation on phase failure**: `rollback_drained_amendments` re-enqueues drained items on any error before durable state persistence, in both standard and quick-dev orchestrators.

- **Lossless mid-drain recovery**: Fatal I/O errors during drain trigger `rollback_mid_drain`, re-enqueueing all already-drained items before returning the error.

---
