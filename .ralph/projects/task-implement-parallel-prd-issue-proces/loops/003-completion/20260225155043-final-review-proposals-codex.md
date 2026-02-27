---
artifact: final-review-proposals
loop: 3
project: task-implement-parallel-prd-issue-proces
backend: codex
role: final_reviewer
created_at: 2026-02-25T15:50:43Z
---

Reviewing the completed changes now with a correctness/safety focus. I’ll first identify the merge base and run the required `git diff <base>...HEAD -- . ':(exclude).ralph'`, then inspect the touched code paths and tests for concurrency, panic/error handling, and isolation guarantees.
I found the feature branch and confirmed there are many `.ralph` workspace artifacts unrelated to source review. Next I’m diffing against `master` with the exact scoped command to inspect only code/test changes.
The source diff touches 12 files, including `interactive_prd`, backend wiring, runtime config plumbing, and tests. I’m reviewing patch-by-patch now, starting with the PRD daemon path and then verifying that tests truly assert the intended concurrency/safety properties.
I’ve confirmed the main structural changes in `poll_and_advance_prd` and `CliBackend`. Next I’m checking all remaining touched files for side effects, especially constructor signature changes, init/refine call sites, and test quality gaps.
I found additional `init` template-link removals outside the PRD target area; I’ll verify whether these are harmless cleanup or a backward-compatibility regression. In parallel I’m reading the new PRD test diffs to validate they prove the intended guarantees.
I’ve captured the core diffs, including extensive new integration/conformance tests. I’m now reading the updated `interactive_prd` and helper call paths with line numbers to verify real behavior (not just intended behavior), then I’ll cross-check whether the new tests actually catch regressions.
I confirmed the conformance suite runs in parallel and is largely timeout-free. I’m now inspecting the test runner and the new integration tests for potential hang scenarios and for whether they mutate process-global env in a race-prone way.
I confirmed the conformance runner has no global per-test timeout, which makes deadlock-prone tests a real risk. I’m now checking whether any of the unrelated `init` changes break backward compatibility or existing runtime behavior.
I’m moving from static review to execution checks now. I’ll run targeted tests around the new concurrency paths (including deadlock-prone cases) to validate behavior and confirm whether the new assertions are robust under real execution.
Targeted integration tests for dedup and bounded concurrency pass in practice. I’ll run at least one conformance-side test entry next, then finalize findings focused on real safety/correctness gaps rather than formatting-level issues.
I also verified there are no stray source-file edits beyond the reviewed diff. Next I’ll run a full type-check to confirm no hidden compile-path regressions, then I’ll finalize amendments with exact file/line references.
# Final Review: AMENDMENTS

## Amendment: FR-PRD-001

### Problem
Concurrent workers are enabled in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:421), but all issue backends run in the same clone directory (`config.repo_clone_path()`) via shared `cwd` wiring in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1081), [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1335), [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1479), and [`backend/mod.rs`](/tmp/final-review-test/src/backend/mod.rs:478).  
Default backend configs are write-capable (`Edit/Write` tools and bypass flags) in [`config/global.rs`](/tmp/final-review-test/src/config/global.rs:701) and [`config/global.rs`](/tmp/final-review-test/src/config/global.rs:730).  
Result: concurrent issues share one mutable filesystem workspace, which can cause cross-issue interference and nondeterministic outcomes.

### Proposed Change
Use per-issue isolated working directories for backend execution in a tick (e.g., per-issue worktree/snapshot), and pass that issue-specific path as `cwd`. Keep clone refresh once per tick, but do not run multiple issue backends in the same directory.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs) - create/use per-issue work dirs and thread them through PRD generation calls.
- [`src/backend/mod.rs`](/tmp/final-review-test/src/backend/mod.rs) - keep current `cwd` support; ensure callers provide isolated paths.

## Amendment: FR-PRD-002

### Problem
Panics are caught and logged in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:440) and [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:459), then the tick returns `Ok` in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:484).  
That bypasses durable failure accounting/state transitions implemented in [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1169) and [`interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs:1566).  
A repeatable panic can therefore retry forever without persisting failure state.

### Proposed Change
Route panic outcomes through the same persisted error path as normal failures: increment `error_count`, set `last_error`, save state, and transition to `Failed` at threshold. Best approach: convert panic to `Err` within a state-aware wrapper so `finish_transition` handles it uniformly.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/final-review-test/src/daemon/interactive_prd.rs) - unify panic/error persistence and failure-threshold behavior.
- [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs) - add regression asserting repeated panic reaches durable `Failed`.
- [`src/validate/tests_interactive_prd.rs`](/tmp/final-review-test/src/validate/tests_interactive_prd.rs) - add conformance assertion for panic persistence.

## Amendment: FR-PRD-003

### Problem
Deadlock-prone FIFO tests run `poll_and_advance_prd` without a watchdog timeout in [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs:3241), [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs:3502), and [`src/validate/tests_interactive_prd.rs`](/tmp/final-review-test/src/validate/tests_interactive_prd.rs:3839).  
The validate runner has no per-test timeout in [`runner.rs`](/tmp/final-review-test/src/validate/runner.rs:117).  
If concurrency regresses, these tests can hang the suite indefinitely.

### Proposed Change
Wrap those calls with a watchdog (`thread::spawn` + `recv_timeout`) and fail explicitly on timeout. Also restore env vars via RAII so timeout/panic paths do not leak mutated `PATH`.

### Affected Files
- [`tests/daemon_interactive_prd.rs`](/tmp/final-review-test/tests/daemon_interactive_prd.rs) - add watchdogs to FIFO/barrier tests.
- [`src/validate/tests_interactive_prd.rs`](/tmp/final-review-test/src/validate/tests_interactive_prd.rs) - add watchdog to bounded-worker test and RAII env restoration helper.

---
