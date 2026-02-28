---
artifact: completer-verdict
loop: 6
project: issue-93
backend: openrouter(openai/gpt-5.3-codex)
role: completer
created_at: 2026-02-28T22:02:31Z
---

# Verdict: CONTINUE

## Missing Requirements
1. **Conformance test #3 is not actually validating watcher behavior**:  
   Required: “Draft watcher creates draft PR when branch first moves ahead.”  
   Current `src/validate/tests_pr_runtime.rs` (`draft_watcher_creates_draft_when_branch_ahead`) only checks `github::has_commits_ahead_of_base(...)`; it does **not** run `draft_pr_watcher`, does not assert PR creation, and does not assert “first move ahead” semantics.

2. **Conformance test #4 does not assert push-before-create ordering**:  
   Required: “Draft watcher pushes before draft PR creation (order assertion).”  
   Current `draft_watcher_pushes_before_create` in `src/validate/tests_pr_runtime.rs` only calls `push_branch` and checks ahead status; it never records/compares call order with `create_pr`.

3. **Conformance test #5 does not exercise the real watcher shutdown path**:  
   Required: “Draft watcher exits cleanly on cancellation.”  
   Current `draft_watcher_exits_cleanly_on_cancellation` only validates `CancellationToken` + a generic `tokio::select!`, not the actual `draft_pr_watcher` task lifecycle/cancellation behavior.

4. **Conformance tests #7 and #8 are predicate-only, not lifecycle integration**:  
   Required: ready/close behavior in PR lifecycle flow.  
   Current `tests_pr_lifecycle.rs` checks `decide_draft_pr_transition(...)` logic only; it does not verify `handle_pr_flow` side effects (`github::mark_pr_ready`, `github::close_pr`, and metadata clearing via `save_task_metadata(... pr_url: None)`).

5. **Conformance tests #9 and #10 do not validate runtime retry execution count**:  
   Required: “complete_task retries transient failures exactly up to 3 attempts” and “does not retry terminal errors.”  
   Current tests verify helper predicates (`should_retry_complete_task`, `complete_task_retry_delay`) rather than observing `complete_task`/`complete_task_attempt` call counts and real retry loop behavior.

6. **Conformance test #12 is not the required lifecycle E2E**:  
   Required: real-binary flow covering draft create → ready mark (or close no diff).  
   Current `e2e_draft_create_via_binary` in `src/validate/tests_pr_runtime.rs` validates `--pr-url` propagation only; it does not cover draft creation, ready promotion, or no-diff close.

## Recommended Next Features
1. **Replace weak PR runtime tests with real watcher-driven harness tests** using mocked `gh` scripts that log invocation order and outputs; assert `push` occurs before `pr create --draft`, and assert single creation attempt.
2. **Add integration tests around `handle_pr_flow`** with mocked GitHub commands to verify:
   - draft PR gets marked ready when completed+has-diff,
   - no-diff draft PR gets closed,
   - PR URL metadata is cleared on close.
3. **Add retry-loop execution tests for `complete_task`** by injecting deterministic transient/terminal failures and asserting exact attempt counts and delay policy (3 max, 30s policy path).
4. **Add true real-binary PR lifecycle E2E validate case** (mock `gh` + temp repo) covering at least one of:
   - draft create → mark ready, or
   - draft create → close on no diff.
