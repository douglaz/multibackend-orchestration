---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T10:18:55Z
---

# Applied reviewer-requested fixes

## Fix 1: `exactly once` dedup violation on state persistence failure (High)

**File:** `src/daemon/pr_review.rs:644-673`

On `state.save()` failure, the code now:
1. Removes the key from `processed_keys` (reverts in-memory insertion)
2. Decrements `new_count`
3. Deletes the just-staged amendment file (deterministic filename)
4. `continue`s to the next comment

This ensures the comment retries cleanly on the next poll cycle. Previously, a save failure would leave the staged file while durable dedup state never recorded the key — if a later dispatch purged staged files, the comment would be re-enqueued as a duplicate.

## Fix 2: Label-log test assertions (Medium)

**Files:** `src/validate/tests_pr_review.rs:693-712` and `src/validate/tests_pr_review.rs:1179-1191`

`swap_lifecycle_label()` performs **two separate** `gh issue edit` calls (remove then add), producing 4 log lines for a forward+rollback cycle:
1. `--remove-label ralph:completed`
2. `--add-label ralph:in-progress`
3. `--remove-label ralph:in-progress`
4. `--add-label ralph:completed`

Updated both tests to assert the expected transition sequence by searching for operations in order (`position`/`rposition`) rather than checking fixed `lines[0]`/`lines[1]` indices.

## Open question (acknowledged)

The spec says quick-dev resume should set `quick_dev_phase=ReviewAndIterate`, while the implementation at `pr_review.rs:275` sets `plan_and_implement`. This was not changed — flagged for explicit confirmation/spec update.
