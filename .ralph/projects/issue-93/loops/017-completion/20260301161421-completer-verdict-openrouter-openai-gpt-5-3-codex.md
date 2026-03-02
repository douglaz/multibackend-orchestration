---
artifact: completer-verdict
loop: 17
project: issue-93
backend: openrouter(openai/gpt-5.3-codex)
role: completer
created_at: 2026-03-01T16:14:21Z
---

# Verdict: CONTINUE

## Missing Requirements
1. **Child-process PR URL plumbing test coverage is incomplete (required test #11):**  
   The required conformance test says `--pr-url` must be “plumbed through child args **and timing is correct**.”  
   In `src/validate/tests_pr_runtime.rs`, `pr_url_plumbed_through_child_args` only validates CLI parsing (`Cli::parse_from`) and does **not** verify dispatch-time resolution/propagation timing in `runtime` (before `spawn_ralph_auto` / `spawn_ralph_run`).

2. **Exact head-branch match behavior is not fully enforced when multiple PRs exist:**  
   Dispatch uses `github::find_existing_pr(owner, repo, branch)` (`src/daemon/runtime.rs:1368`, `1880`) and that helper runs `gh pr list --head <branch> ... -q .[0].url` (`src/daemon/github.rs:532-550`).  
   This selects the first result and does not explicitly disambiguate multiple open PRs that can share a branch name across different heads/forks. The requirement asks for exact head-branch matching under multi-PR conditions.

## Recommended Next Features
1. Add a conformance test that starts daemon dispatch and asserts resolved PR URL is injected into spawned child command **before process launch** (not just clap parsing).
2. Harden PR lookup to enforce exact head identity (e.g., query richer PR JSON and filter deterministically by exact head ref/owner), and add a conformance test with multiple candidate PRs.
3. Remove or wire the unused `complete_task_retry_limits()` helper to eliminate dead-code warning and keep lifecycle retry logic single-sourced.

---
