# Final Review Amendments Applied

## Round 1

### Amendment: FR-194-001

### Problem
Completion is only guarded against pending queue items at planner decision time ([`src/workflow/orchestrator.rs:739`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:739)).  
The run can still return completed without a second queue check ([`src/workflow/orchestrator.rs:2797`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:2797)).  
If an amendment arrives during completing/final-review windows, this run may still report success as completed while leaving pending amendments unprocessed.

### Proposed Change
Add a final pending-queue check immediately before the completed return path. If `pending_amendment_count > 0`, do not finalize completion in that run (either error out with count or transition back to planning). Add a conformance test that enqueues during late phases and verifies completion is blocked.

### Affected Files
- [`src/workflow/orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs) - add late-stage queue guard before final completed return.
- [`src/validate/tests_amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs) - add coverage for amendment arrival after planner completion request.

### Reviewer
codex

### Amendment: FR-194-002

### Problem
`amend_cli_multiple_amendments_drain_in_order` claims order verification but only checks membership with `contains` ([`tests/amend_cli.rs:191`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs:191)-[`tests/amend_cli.rs:221`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs:221)).  
This test passes even if drain ordering regresses.

### Proposed Change
Assert the exact drained ID sequence (or rename the test to remove the order claim). Prefer exact sequence to preserve intended contract.

### Affected Files
- [`tests/amend_cli.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/tests/amend_cli.rs) - make assertion match test intent.

---

### Reviewer
codex


## Round 2

### Amendment: A-194-REVIEW-001

### Problem
Queued amendments can be silently lost on phase failure.

In standard orchestration, amendments are drained and deleted before planner execution ([`src/workflow/orchestrator.rs#L603`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs#L603)), then the run can fail later during prompt build/backend execution ([`src/workflow/orchestrator.rs#L623`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs#L623), [`src/workflow/orchestrator.rs#L660`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs#L660)) with no requeue path.

Quick-dev has the same pattern: drain first ([`src/workflow/quick_dev_orchestrator.rs#L345`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs#L345)), then backend call can fail ([`src/workflow/quick_dev_orchestrator.rs#L363`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs#L363)) and drained items are gone.

This violates safety for external amendment intake under transient backend/template failures.

### Proposed Change
Make drain handling at-least-once for phase failures.

1. Keep drained amendments in memory for the active phase.
2. If the phase errors before a durable success transition, re-enqueue the drained amendments with original fields (`id`, `body`, `priority`, `source`, `source_detail`, `created_at`).
3. Add regression tests that intentionally fail immediately after drain and assert queue contents are preserved for retry.

### Affected Files
- [`src/workflow/orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs) - protect planning-drained amendments from loss on downstream errors.
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs) - same protection for quick-dev `PlanAndImplement`.
- [`src/validate/tests_amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs) - conformance coverage for drain+failure persistence.

---

### Reviewer
codex


## Round 3

### Amendment: AMEND-QUEUE-LOSS-001

### Problem
`drain_amendment_queue_with_hook` can delete already-processed queue items and still return `Err` on a later file operation, which creates a loss path for drained amendments.  
Key points:
- It processes files incrementally and deletes each parsed inflight file ([src/project/amendments.rs:239](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs:239)).
- Any later `?`-propagated IO error aborts the whole drain ([src/project/amendments.rs:168](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs:168)).
- Callers treat drain failure as fatal and cannot rollback because they never receive the partial drained vector ([src/workflow/orchestrator.rs:604](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/orchestrator.rs:604), [src/workflow/quick_dev_orchestrator.rs:347](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/workflow/quick_dev_orchestrator.rs:347)).

### Proposed Change
Make drain failure non-lossy:
1. On fatal mid-drain error, best-effort re-enqueue already drained items before returning `Err`.
2. Add a unit test that injects a mid-drain failure and asserts no amendment disappears.

### Affected Files
- `src/project/amendments.rs` - add internal rollback-on-error behavior in drain path and test coverage.

### Reviewer
codex

### Amendment: AMEND-TEST-SEMANTICS-002

### Problem
The conformance test `quick_dev_checkpoint_failure_no_rollback_after_durable_success` does not actually assert that the checkpoint failure path occurred; it ignores command status (`let _output = ...`) and only checks queue emptiness ([src/validate/tests_amendments.rs:736](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs:736), [src/validate/tests_amendments.rs:775](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/validate/tests_amendments.rs:775)).  
That means the test can pass even when no checkpoint failure happened, so the name/claim is stronger than what it proves.

### Proposed Change
Make the test prove the intended path:
1. Assert non-zero run result and checkpoint/commit failure evidence in stderr, or
2. If deterministic failure cannot be guaranteed, rename the test to reflect current semantics and add a separate deterministic failure-path test.

### Affected Files
- `src/validate/tests_amendments.rs` - tighten assertions (or rename + split test).

---

### Reviewer
codex

### Amendment: AMQ-001

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

### Reviewer
claude


## Round 4

### Amendment: FR-20260309-001

### Problem
In the drain path, all errors from parsing are treated as “malformed” and quarantined, including file read I/O errors.  
This happens in [`src/project/amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs) (drain loop around lines 229-245) and parse helper (around lines 477-481).  
Result: transient/read-side failures (for example `PermissionDenied`) can silently sideline otherwise valid amendments instead of failing and preserving queue semantics.

### Proposed Change
Only quarantine true content errors (JSON/validation).  
Treat read/open I/O failures as fatal (except benign race cases like `NotFound`) so drain returns `Err` and already-drained items are rolled back via existing rollback logic.

### Affected Files
- [`src/project/amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs) - split read vs parse error handling; add regression tests for read-error behavior.

### Reviewer
codex

### Amendment: FR-20260309-002

### Problem
Two root-level implementation artifact files were committed even though they are not product source and are outside the feature scope:
- [`20260309T165947Z-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/20260309T165947Z-impl-notes.md)
- [`20260309T165947Z-impl-response-001.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/20260309T165947Z-impl-response-001.md)

This also exposes a cleanup gap: stray-file matching in [`src/git/commit.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/git/commit.rs) (around lines 275-293) only catches `YYYYMMDDHHMMSS-*`, while these files use `YYYYMMDDTHHMMSSZ-*`.

### Proposed Change
Remove the two committed root artifact files.  
Harden stray-artifact detection so ISO-basic timestamp variants used by generated impl artifacts are also cleaned before commit.

### Affected Files
- [`20260309T165947Z-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/20260309T165947Z-impl-notes.md) - remove from repository.
- [`20260309T165947Z-impl-response-001.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/20260309T165947Z-impl-response-001.md) - remove from repository.
- [`src/git/commit.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/git/commit.rs) - broaden stray filename matching and add tests.

---

### Reviewer
codex

### Amendment: STRAY-ROOT-FILES-001

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

### Reviewer
claude

