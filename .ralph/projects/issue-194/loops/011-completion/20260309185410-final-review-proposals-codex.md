---
artifact: final-review-proposals
loop: 11
project: issue-194
backend: codex
role: final_reviewer
created_at: 2026-03-09T18:54:10Z
---

I’m starting with a full diff audit against the branch base, then I’ll trace high-risk areas (queue draining, orchestrator integration, and tests) directly in source to verify correctness and failure behavior.
I’ve identified the branch context; next I’m computing the merge-base with `origin/master` and reviewing the full source diff (excluding `.ralph`) as requested.
I have the changed file list. I’m now reading the exact patch and then tracing each affected module (`project`, `cli`, `workflow`, `config`, and `validate/tests`) to find correctness and safety issues.
I’ve loaded the patch; next I’m doing line-by-line checks of `src/project/amendments.rs` and both orchestrators, since those are where crash-safety and replay correctness risks are concentrated.
I’ve finished the queue module scan and am now tracing every call site in `orchestrator`/`quick-dev` to ensure rollback and completion guards are placed on correct durable boundaries.
I’m now validating quick-dev’s amendment drain/rollback path and then I’ll cross-check config/CLI wiring plus conformance tests for false positives.
I’m moving through config and CLI wiring now to verify defaults/merge precedence, key paths in `config get/show/set`, and whether command validation fails safely.
I’m now auditing `src/validate/tests_amendments.rs` and `tests/amend_cli.rs` to check whether each test genuinely proves behavior (especially failure-path and recovery assertions).
I’m validating the orchestrator’s true durability boundary now by tracing where `state.json` is actually persisted relative to drained-amendment rollback hooks.
I’ve completed static tracing and will now run targeted tests (`amendments` unit tests plus new CLI/integration tests) to validate behavior and catch hidden regressions.
I’m also running the new conformance test module directly to verify these amendment behaviors under the harness, not just unit tests.
# Final Review: AMENDMENTS

## Amendment: FR-20260309-001 [P1]

### Problem
In the drain path, all errors from parsing are treated as “malformed” and quarantined, including file read I/O errors.  
This happens in [`src/project/amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs) (drain loop around lines 229-245) and parse helper (around lines 477-481).  
Result: transient/read-side failures (for example `PermissionDenied`) can silently sideline otherwise valid amendments instead of failing and preserving queue semantics.

### Proposed Change
Only quarantine true content errors (JSON/validation).  
Treat read/open I/O failures as fatal (except benign race cases like `NotFound`) so drain returns `Err` and already-drained items are rolled back via existing rollback logic.

### Affected Files
- [`src/project/amendments.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-194/src/project/amendments.rs) - split read vs parse error handling; add regression tests for read-error behavior.

## Amendment: FR-20260309-002 [P3]

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
