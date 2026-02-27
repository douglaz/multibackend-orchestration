---
artifact: final-review-proposals
loop: 2
project: issue-92
backend: codex
role: final_reviewer
created_at: 2026-02-27T04:10:39Z
---

Reviewing the completed changes as requested. I’ll start by identifying the base branch and running the required `git diff <base>...HEAD -- . ':(exclude).ralph'`, then inspect the modified source and tests for correctness, safety, and robustness.
I’ve captured the full diff against `origin/master`; only three files changed (`src/daemon/interactive_prd.rs`, `src/validate/tests_interactive_prd.rs`, `tests/daemon_interactive_prd.rs`). Next I’m reading surrounding code paths to verify ordering, failure handling, and whether waiting-label management is actually safe/idempotent in each state transition.
I’m now tracing the three modified transition handlers in detail (`Pending`, `AwaitingAnswers`, `AwaitingFeedback`) to confirm reconciliation runs at the right point and can’t break retry/error paths.
I confirmed startup still routes through `ensure_prd_labels_best_effort_with_gh_bin`, so adding the label to `PRD_LABELS` includes startup ensure automatically. I’m moving to test-quality review now, especially the new conformance cases, to check for false positives and missed assertions.
I’m validating test rigor now: checking whether each newly added test truly verifies the intended path (and not just side effects), plus scanning for weak string-matching assertions that can pass incorrectly.
I’ve finished static inspection of modified paths and haven’t found an obvious logic flaw yet. I’ll run the relevant test suites now to validate behavior and catch any hidden regressions before finalizing the review.
Targeted unit/integration tests for the new label classification assertions pass. I’ll quickly check validate CLI options next so I can run the new conformance coverage directly (or as much as the CLI supports).
`nix build` is still running in the background; I’m waiting for completion, then I’ll run `ralph validate` help and execute the relevant conformance checks.
Build is executing its full Nix check phase (including tests). I saw one pre-existing failure in `dedup_invariant_issue_processed_at_most_once`; I’ll let the run finish and then continue with validate command checks.
`nix build` failed due an existing unrelated test (`dedup_invariant_issue_processed_at_most_once`) in the repo’s current baseline, so `./result/bin/ralph` was not produced. I’ll use `target/debug/ralph` from a normal cargo build to run validate commands for this review.
I found an unintended regression in the conformance test file: several `gh` mock scripts were refactored to `format!(\"...{}...\", raw_script)` while still keeping doubled JSON braces (`{{`/`}}`), which makes the mock emit invalid JSON and causes unrelated conformance tests to fail. I’m pinpointing exact locations now for amendments.
# Final Review: AMENDMENTS

## Amendment: FR-PRD-001

### Problem
Conformance mock scripts were refactored to `format!("#!/bin/sh\nLLOG=\"{}\"\n{}", ..., r#"..."#)` / `.replace(...)`, but the embedded JSON still uses escaped braces (`{{` / `}}`).  
Because that second raw string is inserted verbatim, the mock now emits invalid JSON (for example `printf '[{{"number":...}}]'`), so daemon polling/parsing breaks and state assertions fail.

This affects at least:
- `restart_continuity_marker_timestamp_hydration` (state file not created),
- `bot_login_failure_exhaustion_awaiting_answers` / `..._awaiting_feedback` (error_count remains `0`),
- `bot_login_failure_exhaustion_pending` (state file missing).

### Proposed Change
Fix those mock scripts to output valid JSON:
- Either revert each block to a single `format!(r#"..."#)` with escaped braces,
- Or keep current style and change embedded JSON to single braces (`{` / `}`) inside the inserted raw script.

Re-run the four failing conformance tests after correction.

### Affected Files
- [`src/validate/tests_interactive_prd.rs:2532`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:2532) - invalid-brace JSON in restart continuity mock.
- [`src/validate/tests_interactive_prd.rs:2647`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:2647) - invalid-brace JSON in AwaitingAnswers bot-login-failure mock.
- [`src/validate/tests_interactive_prd.rs:2761`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:2761) - invalid-brace JSON in AwaitingFeedback bot-login-failure mock.
- [`src/validate/tests_interactive_prd.rs:2859`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:2859) - invalid-brace JSON in Pending bot-login-failure mock.

## Amendment: FR-PRD-002

### Problem
Two new label-removal assertions can pass for the wrong reason.  
They currently assert:

- `label_raw.contains("--remove-label") && label_raw.contains("ralph:waiting-feedback")`

This does not guarantee `--remove-label` and `ralph:waiting-feedback` are part of the same command. In these scenarios, a `--remove-label ralph:prd-active` plus a separate `--add-label ralph:waiting-feedback` would still pass.

### Proposed Change
Make assertions command-specific, for example:
- `label_raw.lines().any(|l| l.contains("--remove-label ralph:waiting-feedback"))`
or equivalent tokenized matching.

Apply this to both Done-path and Failed-path removal tests.

### Affected Files
- [`src/validate/tests_interactive_prd.rs:1790`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:1790) - Done-path waiting-label removal assertion.
- [`src/validate/tests_interactive_prd.rs:1988`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs:1988) - Failed-path waiting-label removal assertion.
