---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T04:08:47Z
---

# Review: CHANGES REQUESTED

1. **Critical: `daemon_pr_review_whitelist` is not configurable through `ralph config set`, so the new PR-review validate tests fail immediately.**  
The new field exists in the struct, but the global key mutation switch does not accept `workspace.daemon_pr_review_whitelist`, so it falls into `"unsupported global config key"`. See [`src/config/global.rs:1358`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/config/global.rs:1358) and fallback at [`src/config/global.rs:1654`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/config/global.rs:1654). The new conformance tests call this key at [`src/validate/tests_pr_review.rs:54`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/tests_pr_review.rs:54), and `validate --filter pr_review` currently fails on all 5 tests.  
**Fix:** Add a `set_global_config_value` arm for `"workspace.daemon_pr_review_whitelist"` using `cfg_parse_string_list(raw_value)?`, and add/extend a unit test in `global.rs` to verify set/roundtrip for this key.

2. **High: staged PR-review amendments can be stranded if dispatch fails after draining.**  
`dispatch_task` drains staged files (and removes them from staging) before task spawn at [`src/daemon/runtime.rs:1445`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1445). On dispatch failure, `pr_review_phase` only reverts labels at [`src/daemon/runtime.rs:2655`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2655). Future retries only look at staging (`has_staged_amendments`) at [`src/daemon/runtime.rs:2530`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2530) / [`src/daemon/pr_review.rs:173`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:173), so those amendments may no longer be auto-resumed.  
**Fix:** Make draining failure-safe: either keep staged files until spawn succeeds (then delete), or restage from worktree queue on dispatch error before returning to `ralph:completed`. Add a regression validate test for “dispatch failure preserves retryable staged amendments.”
