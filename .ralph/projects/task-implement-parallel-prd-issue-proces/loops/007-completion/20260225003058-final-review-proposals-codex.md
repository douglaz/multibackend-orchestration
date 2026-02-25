---
artifact: final-review-proposals
loop: 7
project: task-implement-parallel-prd-issue-proces
backend: codex
role: final_reviewer
created_at: 2026-02-25T00:30:58Z
---

Reviewing the implementation against the master prompt now. I’ll start by inspecting the exact source diff from the base branch, then trace related code paths and tests to verify concurrency, ordering, and failure isolation behavior end to end.
I’ve confirmed the working branch and refs. Next I’m pulling the full three-dot diff against `origin/master` and then reading the changed files plus their dependencies for cross-file correctness.
I found nine modified files, with large additions in daemon PRD tests and validate conformance tests. I’m now auditing core runtime/backend changes first, then I’ll verify tests truly enforce the required guarantees.
Backend constructor signatures were extended for optional working directory and wired into process spawning. I’m now deep-reading `interactive_prd.rs` to validate bounded concurrency, dedup semantics, panic/error isolation, and refresh ordering.
I confirmed runtime wiring now passes `daemon_max_concurrent` into PRD polling. Next I’m auditing the new integration and validate tests to ensure each required guarantee is actually exercised deterministically and not just superficially asserted.
Core PRD logic now uses a scoped bounded worker pool with per-thread caches and panic capture. I’m currently tracing the new integration tests and helper scripts in detail to confirm they deterministically verify each required behavior and don’t mask regressions.
I’ve reviewed the new integration tests for all listed concurrency scenarios. Next I’m cross-checking the conformance suite additions and module registration to ensure these guarantees are enforced in `validate`, not only in integration tests.
The core logic mostly matches the requested architecture, so I’m moving to execution checks now. I’ll run targeted test subsets (integration + validate-focused) to confirm behavior and to catch cross-file regressions that static reading might miss.
I executed targeted integration tests; they pass, including panic isolation with real panic injection. I’m now running one conformance-path test directly from the validate module to verify that the newly added validate coverage is wired and executable.
I’ve validated the new validate cases are registered and runnable. I’m now checking for hidden cross-file regressions (panic propagation, spawn-blocking behavior, and backend invocation contracts) before deciding whether this needs amendments.
# Final Review: AMENDMENTS

## Amendment: PRD-CONC-TEST-001

### Problem
The integration “slow vs fast” concurrency test is not a strict proof of no-blocking and uses sleep polling with a timeout fallback.

- In [tests/daemon_interactive_prd.rs:3065](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3065), `concurrent_advancement_slow_and_fast` allows issue `#80` to proceed after a 5s timeout even if `#90` never unblocks it.
- The loop at [tests/daemon_interactive_prd.rs:3119](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3119) (`sleep 0.1` polling) means a sequential implementation can still pass.
- Similar sleep-based overlap appears in bounded-concurrency checks at [tests/daemon_interactive_prd.rs:3294](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3294).

### Proposed Change
Replace sleep/polling with deterministic synchronization primitives (FIFO/barrier handshake) and remove timeout fallback that allows false positives. Assert explicit event ordering proving that the fast issue advanced before the slow issue was released.

### Affected Files
- `tests/daemon_interactive_prd.rs` - rewrite concurrency synchronization in `concurrent_advancement_slow_and_fast` (and optionally bounded test) to deterministic handshakes.

## Amendment: PRD-CONFORMANCE-002

### Problem
Conformance coverage is missing an explicit “slow issue does not block fast issue in same tick when `max_concurrent > 1`” case.

- Registered concurrency tests in [src/validate/tests_interactive_prd.rs:190](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:190) through [src/validate/tests_interactive_prd.rs:207](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:207) include dedup/error/panic/bounded/refresh, but no slow-vs-fast advancement test.
- That leaves a regression gap for the core no-head-of-line-blocking requirement.

### Proposed Change
Add a new validate test (for example `interactive_prd::concurrent_advancement_slow_fast`) using deterministic synchronization, and register it in `tests()`.

### Affected Files
- `src/validate/tests_interactive_prd.rs` - add and register explicit slow-vs-fast concurrent advancement conformance case.

## Amendment: PRD-HERMETIC-003

### Problem
Several new concurrency tests are not hermetic and depend on host-installed `claude`/`codex` CLIs with long backend timeouts.

- Integration tests build configs with `GlobalConfig::default()` and real backend specs (e.g. [tests/daemon_interactive_prd.rs:2743](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:2743), [tests/daemon_interactive_prd.rs:2749](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:2749), [tests/daemon_interactive_prd.rs:3189](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/tests/daemon_interactive_prd.rs:3189)).
- Conformance tests do the same (e.g. [src/validate/tests_interactive_prd.rs:3497](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:3497), [src/validate/tests_interactive_prd.rs:3778](/tmp/ralph-daemon-v2/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-91/src/validate/tests_interactive_prd.rs:3778)).
- This creates environment-dependent latency/flakiness and can mask intent.

### Proposed Change
Make these tests fully hermetic by wiring mock backend commands in `global_config` (or pre-seeding marker state to bypass backend execution where backend behavior is irrelevant), and reduce timeouts to minimal deterministic values.

### Affected Files
- `tests/daemon_interactive_prd.rs` - replace real backend defaults with mock backends in concurrency tests.
- `src/validate/tests_interactive_prd.rs` - same hermetic backend setup for added concurrency conformance tests.
