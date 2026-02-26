---
artifact: final-review-proposals
loop: 2
project: task-simplify-ralph-init-to-minimal-scaf
backend: codex
role: final_reviewer
created_at: 2026-02-26T15:34:26Z
---

Reviewing the completed changes end-to-end for correctness and safety. I’ll start by identifying the correct base branch, run the required triple-dot diff excluding `.ralph`, then inspect the touched source and tests for behavioral and concurrency/error-path issues.
I’ve confirmed the branch context and will use `master` as the diff base unless merge-base suggests otherwise, then review every touched file plus related tests for hidden regressions.
I have the changed file list; next I’m inspecting each diff in source and tests, with extra focus on the new sparse TOML writer and init overlay logic since they carry most safety risk.
I’m now digging into `src/config/global.rs` and related config tests, since that’s where subtle sparse-write correctness and no-mutation-on-failure guarantees can break.
I’ve finished the core source diff and am now validating whether the new/updated tests actually prove the claimed behaviors, especially no-mutation failure paths and overlay semantics.
Next I’m running the test/build pipeline to confirm behavior in practice and to catch any hidden regressions not obvious from static review. I’ll report concrete failures if any appear.
The full test run is still in progress and currently clean. I’m polling to completion so we can confirm final pass/fail status across integration and validate-related suites.
Unit/integration tests and build succeeded. I’m running the conformance suite against the built binary now to confirm behavioral coverage at CLI level.
Conformance is still executing long-running cases; no failures surfaced yet. I’m waiting for completion and final summary before concluding the audit.
# Final Review: AMENDMENTS

## Amendment: RALPH-FR-001

### Problem
Sparse global config edits are incorrect when `ralph.toml` uses inline tables.

In [src/config/global.rs:1283](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1283), path traversal treats any non-`Table` as replaceable and in [src/config/global.rs:1285](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1285) replaces it with an empty table. If the user has `workflow = { ... }`, setting one `workflow.*` key drops sibling keys.

In [src/config/global.rs:1309](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:1309), removals only traverse `Item::as_table_mut`, so clearing optional values (`null` / `none`) inside inline tables becomes a silent no-op.

Current sparse-save tests (for example [src/config/global.rs:3229](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs:3229)) don’t cover inline-table inputs, so this regression is untested.

### Proposed Change
Update sparse mutation/removal traversal to support inline tables without data loss:
1. Preserve existing inline-table entries when descending path segments.
2. Allow key removal from inline tables.
3. Add unit tests that use inline-table configs and assert:
1. sibling keys are preserved after `set`,
2. optional keys are actually removed after `null`/`none`.

### Affected Files
- [src/config/global.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-90/src/config/global.rs) - fix sparse traversal/removal for inline tables and add inline-table coverage tests.

---
