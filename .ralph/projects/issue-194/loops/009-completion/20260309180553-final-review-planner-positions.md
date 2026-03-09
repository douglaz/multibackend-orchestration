---
artifact: final-review-planner-positions
loop: 9
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T18:05:53Z
---

# Planner Positions

## Amendment: AMQ-001

### Position
ACCEPT

### Rationale
Both files confirmed to exist at the repository root via glob search:
- `20260309T165947Z-impl-notes.md`
- `20260309T165947Z-impl-response-001.md`

These are implementation workflow artifacts that have no relationship to the project source code. They would be included in any merge to master. Deleting them is a straightforward cleanup with zero risk.

## Amendment: AMEND-QUEUE-LOSS-001

### Position
ACCEPT

### Rationale
Verified by reading `src/project/amendments.rs:168-247`. The loss path is real:

1. The drain loop at line 196 processes files incrementally. For each file, it deletes the inflight file at **line 239** (`fs::remove_file(&inflight_path)?`) and then pushes to the `drained` vector at **line 243**.

2. If `fs::remove_file` succeeds for files 0..N-1 but fails for file N (e.g., filesystem becomes read-only, permissions change), the `?` operator causes the entire function to return `Err`. The `drained` vector—containing items 0..N-1 whose disk files have already been deleted—is dropped. Those amendments are permanently lost.

3. Similarly, `remove_file_if_exists` at **line 205** uses `?` and could fail mid-drain with the same consequence.

4. The callers at `src/workflow/orchestrator.rs:604` and `src/workflow/quick_dev_orchestrator.rs:347` both use `?` on the drain call. On failure, they never receive the partial results and cannot invoke `rollback_drained_amendments` (which exists at lines 273-294 and is designed for post-drain phase failures, not mid-drain failures).

While the failure condition requires an unusual filesystem event, the code's own design (at-most-once delivery via claim+delete) creates a correctness invariant that is violated when a mid-drain IO error discards already-consumed items. The proposed fix—best-effort re-enqueue of already-drained items before returning `Err`—is consistent with the existing `re_enqueue_amendments` and `rollback_drained_amendments` utilities already present in the module.

## Amendment: AMEND-TEST-SEMANTICS-002

### Position
ACCEPT

### Rationale
Verified by reading `src/validate/tests_amendments.rs:736-797` and the mock script at lines 1549-1595.

The test `quick_dev_checkpoint_failure_no_rollback_after_durable_success` has a genuine gap:

1. **Output is discarded**: Line 775 uses `let _output = ...`, so the command's exit status and stderr are never inspected. The `.expect()` only asserts the process could be spawned, not that it exited with a particular status.

2. **The only assertion is `pending == 0`** (lines 790-795), which holds in **both** the intended scenario (checkpoint fails after durable success, amendments correctly not re-enqueued) **and** the degenerate scenario (run succeeds normally with no checkpoint failure at all, amendments consumed normally).

3. The mock script at line 1558 writes `"corrupt" > .git/index` to sabotage the checkpoint commit. But the test never verifies that corruption actually took effect—if the mock's working directory differs from the git repository's root, or if the checkpoint path is skipped for any reason, the test passes vacuously while exercising the wrong code path.

The test name and comment claim to prove a specific invariant ("no rollback after durable success when checkpoint fails"), but the assertions only prove the weaker property ("queue is empty after a run"). Adding an assertion on the command's exit status or stderr output to confirm the checkpoint failure path was actually exercised would make the test match its stated intent.
