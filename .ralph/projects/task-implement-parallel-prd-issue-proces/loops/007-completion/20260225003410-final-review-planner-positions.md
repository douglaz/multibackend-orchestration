---
artifact: final-review-planner-positions
loop: 7
project: task-implement-parallel-prd-issue-proces
backend: claude(opus)
role: planner
created_at: 2026-02-25T00:34:10Z
---

Now I have all the information needed. Let me write the planner positions.

# Planner Positions

## Amendment: PRD-CONC-TEST-001

### Position
ACCEPT

### Rationale
The amendment correctly identifies real weaknesses in `concurrent_advancement_slow_and_fast`. The test at line 3065 uses a shell-level barrier with `sleep 0.1` polling (line 3119) and a 5-second timeout fallback (`i < 50` iterations of 100ms). This design has two problems:

1. **False positive risk**: A sequential implementation could pass because the barrier has a 5-second timeout. If issue #80 hits the timeout, it proceeds anyway, and the test still passes — there is no assertion that #90 was processed *before* #80's barrier expired.

2. **Non-deterministic timing**: The `sleep 0.1` polling in the bounded concurrency test (line 3294) introduces timing sensitivity that doesn't constitute a proof of concurrent execution.

The master prompt explicitly states "Use deterministic synchronization, not sleep-based timing assumptions" under Testing Requirements. The current tests violate this requirement. Replacing these with proper FIFO/barrier handshakes that assert explicit event ordering (fast issue created the barrier *before* the slow issue unblocked) would provide a genuine proof of no-head-of-line-blocking rather than a probabilistic one. This is a correctness gap, not a polish issue.

## Amendment: PRD-CONFORMANCE-002

### Position
ACCEPT

### Rationale
The amendment correctly observes that the conformance test registry (lines 190–207) covers dedup, error isolation, panic isolation, bounded concurrency, and refresh ordering, but is missing a slow-vs-fast concurrent advancement test. The master prompt's Testing Requirement #1 ("Concurrent advancement test: One slow issue and one immediately-advanceable issue. With `max_concurrent >= 2`, assert both advance in one tick.") should have conformance coverage matching the integration test.

The integration test `concurrent_advancement_slow_and_fast` exists but has the weaknesses noted in PRD-CONC-TEST-001. Adding a conformance test here provides a second layer of defense against regressions on the core no-head-of-line-blocking invariant. Since the conformance tests use `run_case()` with `catch_unwind` and test the actual `poll_and_advance_prd` path (as seen in sibling conformance tests like `concurrent_dedup_invariant` at line 3497), adding one more for slow-vs-fast advancement is straightforward and fills a genuine gap.

## Amendment: PRD-HERMETIC-003

### Position
REJECT

### Rationale
The amendment's premise is incorrect. The tests are **already hermetic**. Examining the actual code:

- At line 2743, `GlobalConfig::default()` is used but the backend names (`"claude"`, `"codex"`) in `question_backends`/`writer_backend`/`reviewer_backend` are **never invoked** in these concurrency tests. The tests exercise `poll_and_advance_prd` which calls `advance_issue`, and the transitions being tested (Pending → AwaitingAnswers) only perform GitHub label edits and comment posting via the mock `gh` script — they don't spawn backend processes.

- The `backend_timeout_secs` values (30 or 60) at lines 2753, 3189, 3501, 3778 are irrelevant because no backend execution occurs in these test paths. The Pending → AwaitingAnswers transition calls `gh issue edit` to add labels and `gh issue comment` to post questions, both of which are handled by the mock shell script with immediate responses.

- The conformance tests at lines 3497 and 3778 use the same pattern — `GlobalConfig::default()` with mock gh scripts handling all the actual I/O. No `claude` or `codex` binary is ever executed.

The amendment would add complexity to "fix" something that isn't broken. The tests are fully hermetic via PATH-injected mock `gh` scripts and don't depend on any host-installed CLI tools. Setting backend_timeout to minimal values is also unnecessary since timeouts are never reached.

## Amendment: INTERACTIVE-PRD-TEST-REFACTOR-INCOMPLETE

### Position
REJECT

### Rationale
The amendment's factual claims are entirely incorrect. The entities it references do not exist in the codebase:

1. **`TestHarness::with_config_and_mock_backend`** does not exist anywhere in the test file. No `TestHarness` struct is defined. All tests use either `TempDir` directly for unit-style tests or `RalphHarness::new_daemon()` for integration tests that invoke the daemon binary.

2. **The four "older" tests** (`advances_pending_to_awaiting_answers`, `advances_awaiting_answers_to_awaiting_feedback`, `updates_bot_login_cache_from_feedback`, `dedups_issues_across_labels`) do not exist. A grep for these function names returns zero matches.

3. **The three "newer" tests** (`advances_concurrently_with_slow_issue`, `concurrency_is_bounded`, `panic_in_one_issue_is_isolated`) also do not exist by these names. The actual concurrency tests are: `concurrent_advancement_slow_and_fast`, `bounded_concurrency_peak_never_exceeds_max`, and `panic_isolation_tick_completes_despite_panic`.

The test file uses a consistent pattern throughout: the concurrency tests (lines 2644+) use `TempDir` + direct `PrdPollConfig` construction + mock gh scripts + `poll_and_advance_prd()`, while the state-machine workflow tests (lines 183–2493) use `RalphHarness::new_daemon()` + daemon binary invocation. These are appropriately different patterns for different test types (unit-level function tests vs binary-level integration tests), and there is no incomplete refactoring to address. This amendment appears to be based on hallucinated code rather than actual file contents.
