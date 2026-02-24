---
artifact: final-review-planner-positions
loop: 3
project: task-implement-parallel-prd-issue-proces
backend: claude(opus)
role: planner
created_at: 2026-02-24T22:47:02Z
---

Now I have a thorough understanding of the codebase state. Let me check the master prompt's testing requirements section once more to confirm what it says about conformance tests specifically.

Looking at the master prompt:

**Testing Requirements section** lists 7 test categories including:
> 6. Repo refresh ordering test: Assert refresh occurs once per non-empty tick and before any backend invocation.

**File-Level Change Targets** says:
> `src/validate/tests_interactive_prd.rs` - Add/extend conformance coverage for new concurrency guarantees.

The master prompt explicitly names `src/validate/tests_interactive_prd.rs` as a target for conformance coverage of the new concurrency guarantees. The "Repo refresh ordering test" is one of the 7 required test cases. While the integration test in `tests/daemon_interactive_prd.rs` covers this, the master prompt lists both test files as targets, and the conformance file has zero refresh-related tests.

# Planner Positions

## Amendment: REFRESH-CONFORMANCE-001

### Position
ACCEPT

### Rationale
The master prompt explicitly lists `src/validate/tests_interactive_prd.rs` under "File-Level Change Targets" with the directive to "Add/extend conformance coverage for new concurrency guarantees." The "Repo refresh ordering test" is enumerated as test requirement #6: "Assert refresh occurs once per non-empty tick and before any backend invocation." The reviewer correctly identified that the integration test `refresh_repo_clone_once_before_processing` exists in `tests/daemon_interactive_prd.rs` but has no corresponding conformance test — grep for "refresh" and "repo_clone" in the conformance file returns zero matches. Since the master prompt asks for coverage in both test files, this is a genuine gap. The proposed approach (mock git/gh to log events to a shared file, assert refresh is first and appears exactly once, register in the `tests()` vector) follows the established conformance test patterns already used for other concurrency tests like `concurrent_dedup_invariant` and `concurrent_bounded_worker_count`.
