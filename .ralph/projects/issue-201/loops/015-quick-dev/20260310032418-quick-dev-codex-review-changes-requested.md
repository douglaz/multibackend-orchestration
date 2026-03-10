---
artifact: quick-dev-codex-review
loop: 15
project: issue-201
backend: codex
role: reviewer
created_at: 2026-03-10T03:24:18Z
---

# Review: CHANGES REQUESTED

1. High: deferred reactivation is one-shot and will not retry on later cycles when capacity frees up.  
[runtime.rs:2470](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2470) returns early when `poll_results` is empty, and [pr_review.rs:511](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/pr_review.rs:511) only emits results for *newly* seen comments. If a task is deferred at [runtime.rs:2485](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2485), no new comments appear next cycle, so it is never resumed.  
Fix: in `pr_review_phase`, also scan staged amendment dirs (`.ralph/daemon/pr-review-amendments/*`) and attempt dispatch for those tasks even when `new_amendment_count == 0`; keep retrying until drained+dispatched.

2. High: paginated JSON merge can silently drop comments, especially with `[`/`]` inside comment bodies.  
[github.rs:2463](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2463) and [github.rs:2512](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2512) do bracket counting without JSON-string awareness. Parsing failures are then swallowed via `unwrap_or_default` at [github.rs:2326](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2326), [github.rs:2352](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2352), [github.rs:2380](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/github.rs:2380).  
Fix: replace manual merge with robust parsing (`gh api --paginate --slurp` and flatten, or `serde_json::Deserializer` over concatenated values), and propagate parse errors instead of defaulting to empty vectors.

3. Medium: PR-review resume path can mutate issue title/body/comment with synthetic text.  
`pr_review_phase` passes placeholder text at [runtime.rs:2541](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:2541), but `dispatch_task` still runs refinement and issue updates at [runtime.rs:1494](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1494), [runtime.rs:1508](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1508), [runtime.rs:1526](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/daemon/runtime.rs:1526).  
Fix: skip refinement/title-body/comment updates when `resume_existing_project == true` (or provide real issue text instead of placeholder).

4. Medium: required validate conformance coverage for this feature is missing.  
No `tests_pr_review` module is added/registered in [validate/mod.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-201/src/validate/mod.rs:14).  
Fix: add `src/validate/tests_pr_review.rs` and register it in `src/validate/mod.rs`; include at least whitelist filtering, completed-project resume, dedup across restart, capacity deferral retry, and quick-dev resume behavior.

Open assumption: I assumed deferral must eventually dispatch without requiring another new comment, per the acceptance criterion wording.

Targeted tests run: `nix develop -c cargo test pr_review --quiet` passed, but the above gaps remain against spec behavior.
