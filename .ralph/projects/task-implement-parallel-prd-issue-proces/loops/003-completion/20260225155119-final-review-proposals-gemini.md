---
artifact: final-review-proposals
loop: 3
project: task-implement-parallel-prd-issue-proces
backend: gemini
role: final_reviewer
created_at: 2026-02-25T15:51:19Z
---

# Final Review: AMENDMENTS

## Amendment: PRD-CONCURRENCY-STATE-LOSS

### Problem
In `src/daemon/interactive_prd.rs`, the concurrent implementation of `poll_and_advance_prd` has a critical flaw in its state management. When an issue is successfully processed by a worker thread, its state is updated in a local `prd_state` instance within that thread. However, this updated state is never propagated back to the main `PrdState` instance that is persisted to disk.

Specifically, in the `std::thread::scope` block (lines 207-259), each thread creates its own `prd_state` by calling `PrdState::from_root(root)`. It then performs state transitions on this local copy (e.g., `prd_state.record_gaps_generated(...)`). When the thread finishes, its `prd_state` copy is dropped, and any changes are lost. The main `prd_state` object on line 125 is never updated, and therefore the daemon will re-process the same issues on every tick, believing they are still in their original state.

This bug completely breaks the PRD advancement logic.

### Proposed Change
The `PrdState` object should be shared safely across all worker threads. Since `PrdState` internally uses `sled` which is thread-safe (`Send + Sync`), we can wrap it in an `Arc` and pass clones of the `Arc` to each worker thread. This ensures all state modifications happen on the same, single `PrdState` instance, and changes are correctly persisted.

1.  Wrap the main `prd_state` in an `Arc`.
2.  In the worker loop, clone the `Arc<PrdState>` for each thread instead of creating a new `PrdState` instance from the root path.
3.  Update the function calls inside the thread to use the `Arc`-wrapped state object.

### Affected Files
- `src/daemon/interactive_prd.rs` - Modify `poll_and_advance_prd` to wrap `PrdState` in an `Arc` and share it with worker threads, ensuring state updates are persisted.

## Amendment: PRD-INCORRECT-ERROR-HANDLING

### Problem
In `src/daemon/interactive_prd.rs`, the error handling for issue processing within worker threads is incorrect. When `generate_answers_with_timeout` or `generate_feedback_with_timeout` returns an error, the code correctly logs it but then proceeds to call `prd_state.record_gaps_failed()` or `prd_state.record_revision_failed()`.

This is wrong. These `record_*_failed()` methods are intended for when the *backend operation itself succeeds* but indicates a failure (e.g., the model couldn't generate an answer). They are not for handling I/O errors, timeouts, or other systemic failures that occur within the `generate_*_with_timeout` functions. When a systemic failure occurs, the state of the issue should not be transitioned to `Failed`. It should remain in its current state so it can be retried on a subsequent daemon tick.

This bug causes transient errors to permanently mark an issue as failed, preventing it from ever being successfully processed.

### Proposed Change
Remove the calls to `record_gaps_failed` and `record_revision_failed` from the `Err` match arms inside the worker threads. Logging the error is sufficient. The issue's state will remain unchanged, allowing for a retry on the next tick, which is the correct behavior for transient or operational failures.

### Affected Files
- `src/daemon/interactive_prd.rs` - Remove the incorrect state transition calls (`record_gaps_failed` and `record_revision_failed`) when a backend operation fails due to a system error or timeout.

## Amendment: PRD-DEAD-CODE-NORMALIZER

### Problem
In `src/daemon/interactive_prd.rs`, `poll_and_advance_prd` constructs a `Arc<Mutex<BackendOutputNormalizer>>` on lines 164-167. This normalizer is then cloned and passed into each worker thread.

However, it is never used. The `output_normalizer` variable is passed to `generate_answers_with_timeout` and `generate_feedback_with_timeout`, but inside those functions, the argument is completely ignored. The actual `CliBackend` instance created inside those functions initializes its own `BackendOutputNormalizer` internally.

This is dead, misleading, and slightly inefficient code that adds unnecessary complexity with `Arc<Mutex<...>>` that is never read from or written to.

### Proposed Change
Remove the creation and propagation of `BackendOutputNormalizer` from `poll_and_advance_prd` and the signatures of `generate_answers_with_timeout` and `generate_feedback_with_timeout`. The backend will continue to use its own internally-created normalizer, so the behavior remains unchanged.

### Affected Files
- `src/daemon/interactive_prd.rs` - Remove the unused `BackendOutputNormalizer` and all related code from `poll_and_advance_prd`, `generate_answers_with_timeout`, and `generate_feedback_with_timeout`.

---
