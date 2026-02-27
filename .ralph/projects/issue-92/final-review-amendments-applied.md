# Final Review Amendments Applied

## Round 1

### Amendment: FR-PRD-001

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

### Reviewer
codex

### Amendment: FR-PRD-002

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

### Reviewer
codex


## Round 2

### Amendment: DEDUP-REGRESSION-01

### Problem
The `dedup_invariant_issue_processed_at_most_once` integration test (`tests/daemon_interactive_prd.rs:2654`) passes on master but fails on this branch. The test counts **all** `gh issue edit` calls as a proxy for "how many times issue #50 was processed." Before this PR, the `Pending->AwaitingAnswers` path made 1 edit call (removing `ralph:prd`). After this PR, `ensure_waiting_feedback_label` at `src/daemon/interactive_prd.rs:1051` adds a second edit call (`--add-label ralph:waiting-feedback`), so the counter reaches 2 instead of the expected 1.

The test assertion at line 2776 fails:
```
assertion `left == right` failed: issue #50 should be processed exactly once per tick, but was processed 2 times
```

This causes `nix build -L` to fail because the integration test suite is included in the nix build.

### Proposed Change
Update the test's expected count from 1 to 2 at line 2776, and update the comment at lines 2770-2773 to explain that the Pending->AwaitingAnswers path now makes 2 edit calls (remove `ralph:prd` + add `ralph:waiting-feedback`). The dedup invariant still holds — the issue is processed once, it just has 2 label mutations now.

Alternatively, the test could be restructured to count only `--remove-label ralph:prd` calls as the processing marker, which would keep the expected count at 1 and be more resilient to future label additions. Either approach is acceptable.

### Affected Files
- `tests/daemon_interactive_prd.rs` - Update expected count and/or counting logic in `dedup_invariant_issue_processed_at_most_once` (lines 2770-2778)

---

### Reviewer
claude

### Amendment: FR-PRD-WAITING-001

### Problem
`ralph:waiting-feedback` reconciliation in waiting states is currently gated behind bot-login resolution.  
In both waiting-state transitions, `get_or_fetch_bot_login(...)` runs before the helper is reached (`[interactive_prd.rs#L1151](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1151)`, `[interactive_prd.rs#L1163](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1163)`, `[interactive_prd.rs#L1179](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1179)`, `[interactive_prd.rs#L1293](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1293)`, `[interactive_prd.rs#L1305](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1305)`, `[interactive_prd.rs#L1320](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs#L1320)`).  
If `gh api user` fails, waiting-state ticks take the retry/error path without any waiting-label reconciliation.

Test coverage misses this gap: the bot-login-failure waiting-state tests log label edits but only assert `ralph:prd-failed`, not waiting-label add (`[daemon_interactive_prd.rs#L1621](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs#L1621)`, `[daemon_interactive_prd.rs#L1766](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs#L1766)`, `[tests_interactive_prd.rs#L2622](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs#L2622)`, `[tests_interactive_prd.rs#L2736](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs#L2736)`).

### Proposed Change
Move waiting-label reconciliation to the start of waiting-state transition wrappers, before bot-login lookup, so it always runs on no-op, processing, and retry/error ticks when labels are present.  
Then remove duplicate helper calls inside `do_awaiting_answers_to_awaiting_feedback` / `do_awaiting_feedback`.  
Add assertions in both integration and conformance bot-login-failure waiting-state tests that `--add-label ralph:waiting-feedback` is attempted.

### Affected Files
- [`src/daemon/interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs) - run reconciliation before bot-login in waiting-state handlers.
- [`tests/daemon_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs) - add waiting-label assertions in bot-login-failure waiting-state tests.
- [`src/validate/tests_interactive_prd.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs) - add equivalent conformance assertions.

### Reviewer
codex


## Round 3

### Amendment: FR-PRD-WAITING-POSTSAVE-001

### Problem
`Done` can be durably saved even when terminal label cleanup fails, which can leave stale `ralph:prd-active` and `ralph:waiting-feedback` labels indefinitely.

In [interactive_prd.rs:1505](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1505), removal of `ralph:prd-active` is fallible (`?`). If it fails, execution returns before the new waiting-label removal at [interactive_prd.rs:1518](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1518).  
Then `finish_transition` persists state again on error paths at [interactive_prd.rs:1675](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs:1675) without reverting `Done` unless save itself fails. This makes the issue terminal in state storage, so subsequent ticks skip transition logic and cleanup is never retried.

### Proposed Change
Make post-save `Done` cleanup failures retryable instead of terminally committed:

1. In `finish_transition`, when `result.is_err()` and `state.state == Done`, revert to `pre_transition_state` before saving so retries remain possible.
2. In `do_approval_transition`, attempt waiting-label removal even if active-label removal fails (collect cleanup errors, don’t short-circuit before trying both).
3. Add tests for the exact case “save succeeds, `--remove-label ralph:prd-active` fails” to prove state remains non-terminal and cleanup retries on next tick.

### Affected Files
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/daemon/interactive_prd.rs` - fix `Done` error-path state handling and cleanup sequencing.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/tests/daemon_interactive_prd.rs` - add integration coverage for post-save active-label removal failure.
- `/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-92/src/validate/tests_interactive_prd.rs` - add conformance coverage for the same failure mode.

---

### Reviewer
codex

