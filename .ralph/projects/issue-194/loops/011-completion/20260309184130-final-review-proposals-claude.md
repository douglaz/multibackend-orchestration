---
artifact: final-review-proposals
loop: 11
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T18:41:30Z
---

Good — `EffectiveAmendmentsConfig` is defined in `src/config/mod.rs` and used in the quick_dev test helper. The standard orchestrator tests use `resolve_effective_config` which already includes the amendments field.

Let me do one final check — does the `ProjectAmendmentsOverrides` use `deny_unknown_fields`? If so, there's no issue, but let me confirm:

Looking at `src/config/project.rs` diff:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectAmendmentsOverrides {
    pub unify_final_review: Option<bool>,
}
```

Good, `deny_unknown_fields` is set — typos in config keys won't be silently ignored.

I've now completed my thorough review. Let me compile my findings.

# Final Review: AMENDMENTS

## Amendment: STRAY-ROOT-FILES-001

### Problem
Two implementation artifact files from loop 8 remain tracked in the repository root:
- `20260309T165947Z-impl-notes.md` (line 1-17)
- `20260309T165947Z-impl-response-001.md` (line 1-21)

These are stray files that should have been cleaned up by `remove_stray_impl_artifacts` but were committed during the loop-8 implementing→reviewing transition. They contain internal implementation notes and are not part of the deliverable source code. `[P3]`

### Proposed Change
Delete both files and commit the removal.

### Affected Files
- `20260309T165947Z-impl-notes.md` - delete
- `20260309T165947Z-impl-response-001.md` - delete

---

## Summary

Beyond the stray files above, the implementation is correct, safe, and complete. Here is what I verified:

**Data Model** (`src/project/amendments.rs`): `AmendmentRequest`, `AmendmentPriority` (default P2), and `AmendmentSource` (kebab-case serde) are correctly defined with proper validation. Serde defaults work as specified.

**Queue Operations** (`src/project/amendments.rs`):
- `enqueue_amendment` uses atomic temp-then-hard_link handoff (lines 129-162), correctly avoiding rename overwrite on Unix. Suffix collision handling is sound.
- `drain_amendment_queue` (lines 164-258) implements crash-safe `.json`→`.inflight` claim, `.inflight` recovery, dedup for interrupted claims (stem tracking in `completed_inflight_stems`), quarantine for malformed files, and mid-drain rollback via `rollback_mid_drain`.
- `pending_amendment_count` correctly counts both `.json` and `.inflight` files while excluding `.tmp-*` staging files.
- Sort order is lexicographic by filename, which preserves timestamp ordering.

**CLI** (`src/cli/amend.rs`, `src/cli/mod.rs`): `ralph amend` is properly wired with `--project`, `--body` (including `@path`), `--priority` (default P2), and `--id` (default `EXT-<timestamp>`). Priority validation rejects non-P0/P1/P2/P3 values. Project existence is checked before enqueue.

**Standard Orchestrator** (`src/workflow/orchestrator.rs`):
- Drains queue at start of Planning phase (line 601). Filters out `FinalReview`-sourced items when `unify_final_review=true`.
- Injects `external_amendments` into `build_planner_prompt` with fallback `## External Amendments` section via `append_section_if_missing`.
- Completion guard checks `pending_amendment_count` before honoring `CompletionRequest` (line 749).
- Late guard checks again at line 2832 before the final success return.
- All failable operations between drain and durable state commit (`register_feature_loop`/`register_completion_attempt`) are wrapped with `rollback_drained_amendments`.

**Quick-Dev Orchestrator** (`src/workflow/quick_dev_orchestrator.rs`):
- Drains after pre-commit feedback in `PlanAndImplement` phase (line 344). Appends `## External Amendments` section if non-empty.
- Rollback wraps all operations up to `save_state_to_disk` (durable success boundary at line 408). Checkpoint failure after durable persistence correctly does NOT trigger rollback.

**Final-Review Unification** (`src/workflow/orchestrator.rs:4390-4431`): When `unify_final_review=true`, accepted final-review amendments are enqueued as `AmendmentRequest` with `source=FinalReview`, `priority=P2`, and `source_detail` set to reviewer backend. Enqueue failures are logged but don't fail orchestration.

**Config** (`src/config/global.rs`, `src/config/project.rs`, `src/config/mod.rs`): `amendments.unify_final_review` is properly defined at both global (default `false`) and project (optional override) scopes with correct merge precedence.

**Tests**: 21 unit tests in `amendments.rs` covering enqueue naming, deterministic drain ordering, post-drain cleanup, inflight recovery, dedup, malformed quarantine, temp file exclusion, missing queue, priority defaults, roundtrip, formatter, re-enqueue, and mid-drain rollback. 3 integration tests in `tests/amend_cli.rs`. 16 conformance tests in `src/validate/tests_amendments.rs` covering CLI behavior, planner/quick-dev injection, completion guard, late guard, config defaults, unify dedupe, unify mirroring, planning failure rollback, quick-dev failure rollback, and checkpoint-failure no-rollback.
