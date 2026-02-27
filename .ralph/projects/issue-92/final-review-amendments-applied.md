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

