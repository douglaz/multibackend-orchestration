---
artifact: final-review-proposals
loop: 13
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T09:28:15Z
---

Reviewing the full quick-dev implementation now with a correctness/safety focus. I’ll start by identifying the base branch, run the required `git diff <base>...HEAD -- . ':(exclude).ralph'`, then inspect changed code paths and tests for real behavioral issues.
I found the branch context (`ralph/issue-146` off `master`). Next I’m diffing against `master` (and checking merge-base alignment) to inspect exactly what changed outside `.ralph`.
I have the changed file list. I’ll now inspect the core logic (`quick_dev_orchestrator`, parser/state/config/CLI/daemon) plus tests, then call out any concrete defects with line references and fixes.
I’m drilling into `quick_dev_orchestrator.rs` in full now, including failure paths and resume logic, because the initial diff output was truncated and the critical issues will likely be there.
I’ve confirmed the phase-machine core and I’m now checking the remaining changed modules (CLI wiring, daemon dispatch, config merge, artifacts/lifecycle interactions, and validate tests) for regressions and hidden edge cases.
I’m moving through the non-orchestrator changes now (CLI, daemon, config, prompt builders, and lifecycle hooks) to catch integration bugs and unintended side effects.
I’ve identified a few high-risk spots and I’m validating them directly against the surrounding lifecycle/state code to ensure they’re real issues before I report amendments.
I’m now running targeted tests around quick-dev, parser, daemon dispatch, and state reconstruction to confirm behavior under execution and catch anything static review might miss.
# Final Review: AMENDMENTS

## Amendment: QD-FR-001

### Problem
A stray non-source artifact was committed at repo root: `20260304T082736-impl-response-001.md` (starts at line 1).  
This is implementation-process output, not runtime/source/test code, and it is outside the project’s intended deliverables.

### Proposed Change
Remove the file from the repository history for this branch/PR.

### Affected Files
- `20260304T082736-impl-response-001.md` - delete stray artifact file.

## Amendment: QD-FR-002

### Problem
`QuickDevOrchestrator` hard-caps phase transitions at 100 (`src/workflow/quick_dev_orchestrator.rs:281`, `:781-783`).  
This can cause false failures (`"quick-dev: exceeded maximum phase transitions (100)"`) before user-configured guards (`--max-review-iterations`, `--max-final-review-retries`) are reached, so configured limits are not reliably honored for larger values.

### Proposed Change
Replace the fixed `0..100` cap with a bound derived from configured limits (or remove the fixed cap and rely on guard-based termination). Add a regression test with elevated limits to prove no premature cap-triggered failure.

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - remove/replace fixed 100-step bound with config-aware termination logic.
- `tests/quick_dev_orchestrator.rs` - add regression coverage for high iteration/retry settings.

## Amendment: QD-FR-003

### Problem
Quick-dev state persistence is non-atomic: `save_state_to_disk` writes directly via `fs::write` (`src/workflow/quick_dev_orchestrator.rs:892-896`).  
During crash/power-loss windows, `state.json` can be partially written/corrupted, undermining the “crash-safe resumable” guarantee. Recovery currently silently ignores parse failure (`src/project/lifecycle.rs:458-503`), which can drop persisted phase/counter state.

### Proposed Change
Write `state.json` atomically (temp file in same dir, flush/fsync, then rename; optionally fsync parent dir). Also emit a warning/error log when `state.json` parsing fails during reconstruction so state-loss is observable.

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - implement atomic state write path.
- `src/project/lifecycle.rs` - log parse failures for `state.json` quick-dev metadata loading.

---
