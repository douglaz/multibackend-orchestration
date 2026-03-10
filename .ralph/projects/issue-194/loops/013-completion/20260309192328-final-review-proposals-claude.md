---
artifact: final-review-proposals
loop: 13
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T19:23:28Z
---

That's in the `.ralph` directory and excluded from the diff scope. Not a concern for this review.

Now let me assemble my final review. The implementation is thorough, correct, and well-tested. I found one issue:

1. **Stray impl-notes file at repo root** - `20260309T190541Z-impl-notes.md` is committed and tracked. It's an implementation artifact from loop 12 that should have been cleaned up. Ironically, the same commit that added ISO-basic timestamp detection for stray cleanup left this file behind.

Let me also double-check one subtle concern I noticed — on line 601-602 of orchestrator.rs, the `get_or_create_for_role` for the planner is called BEFORE `drain_amendment_queue`. If that call fails, no amendments were drained, so no rollback is needed. But that means the error path doesn't need the `rollback_drained_amendments` wrapper, which is correct since it's not applied there.

Let me also verify that there isn't a TOCTOU issue with the completion guard — the planning-phase guard checks `pending_amendment_count` and the late guard also checks it. Between the planning guard passing (count=0) and the late guard checking, new amendments can arrive, which is the intended behavior (the late guard catches those). Good.

One more concern: the `format_external_amendments_for_prompt` function's behavior when `body` contains only whitespace — the validate check on `body.trim().is_empty()` prevents empty-after-trim bodies from being enqueued, but a body like "   " (all spaces) would also be rejected. This matches the spec requirement that `body` is required and non-empty.

# Final Review: AMENDMENTS

## Amendment: STRAY-IMPL-NOTES-001

### Problem
The file `20260309T190541Z-impl-notes.md` at the repository root is a committed stray implementation artifact from loop 12 (`commit 6885d06`). It contains implementation notes from the loop-12 amendment round and should have been cleaned up by `remove_stray_impl_artifacts()` during the implementing→reviewing transition. The ISO-basic timestamp pattern (`YYYYMMDDTHHMMSSZ-`) detection added in `src/git/commit.rs` was specifically designed to catch files like this, yet the file persisted because it was committed before that detection logic existed and no subsequent cleanup pass ran against already-tracked files.

This file is not part of the project's source code — it is an ephemeral artifact that should not ship with the branch.

### Proposed Change
`[P3]` Remove the file `20260309T190541Z-impl-notes.md` from the repository via `git rm`.

### Affected Files
- `20260309T190541Z-impl-notes.md` - delete from repository

---

## Summary

The implementation is **correct, safe, and well-structured** across all major concerns. Specific verification:

**Data Model** (`src/project/amendments.rs`): `AmendmentRequest`, `AmendmentPriority` (default P2 via `#[default]`), `AmendmentSource` (kebab-case serde) — all match spec. Validation rejects empty `id`/`body`.

**Atomic Handoff** (`enqueue_amendment`): Uses `create_new(true)` for temp file uniqueness, `hard_link` for collision-safe publish (no silent overwrite), `sync_all` before rename. Suffix loop handles collisions correctly. Temp and final files are in the same directory (same filesystem), so hard_link always works.

**Crash-Safe Drain** (`drain_amendment_queue`): Claims `.json` → `.inflight` via hard_link, reads, deletes on success. `.inflight` sorts before `.json` lexicographically, ensuring the `completed_inflight_stems` dedup correctly handles the crash-recovery case where both exist. `InflightReadOutcome` properly separates I/O failures (fatal → rollback) from content failures (quarantine → continue).

**Mid-Drain Rollback** (`rollback_mid_drain`): Re-enqueues all previously-drained items on fatal error. The `drained_for_rollback` clone in the orchestrator captures all items *before* the unify_final_review filter, ensuring final-review-sourced items are also restored.

**Completion Guards**: Planning-phase guard via `pending_amendment_count` (read-only, no drain). Late guard at the final success return path in the completing phase catches amendments arriving during completing/final-review. Neither guard mutates queue state.

**Quick-Dev Integration**: Drain-then-rollback pattern correctly splits `persist_destination_and_checkpoint` into `persist_quick_dev_state` + `save_state_to_disk` (rollback-eligible) + `checkpoint_if_enabled` (post-durable, no rollback). This prevents duplicate replay after durable persistence.

**Unify Final-Review**: Config merge precedence (project → global) is correct. Mirroring enqueues accepted amendments with `source=FinalReview`. Planning-phase dedupe filters these out of `external_amendments` text. Config CRUD (get/set at both scopes) is wired correctly.

**CLI** (`src/cli/amend.rs`): Priority validation, `@path` body expansion, default ID generation, project existence check — all correct.

**Tests**: 32+ unit tests in amendments.rs, 3 integration tests in `tests/amend_cli.rs`, 20 conformance tests in `src/validate/tests_amendments.rs` covering all spec acceptance criteria. Mock scripts verify prompt content injection (grep for amendment IDs/bodies in prompt text). The `quick_dev_checkpoint_failure_no_rollback_after_durable_success` test exercises the split-persist invariant by corrupting the git index.
