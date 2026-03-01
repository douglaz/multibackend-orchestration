---
artifact: completer-verdict
loop: 13
project: issue-93
backend: openrouter(openai/gpt-5.3-codex)
role: completer
created_at: 2026-03-01T15:27:04Z
---

# Verdict: CONTINUE

## Missing Requirements
1. **Child-process PR URL resolution is not using the actual head branch used for work**  
   Requirement 5 says PR URL dispatch should resolve by exact head-branch match (especially when multiple PRs exist).  
   In `dispatch_task`, the code sets `branch_name = "ralph/daemon/{task_id}"` and uses that for PR lookup and watcher branch context (`src/daemon/runtime.rs`, around lines 1348, 1368, 1417, 1422).  
   But earlier in the same flow, the worktree is explicitly switched to the project branch `ralph/issue-{n}` via `sync_project_branch` (`src/daemon/runtime.rs` ~1227-1235).  
   This mismatch means draft PR lookup/creation can target the daemon branch instead of the actual working head branch.

2. **Draft watcher push/create branch target is inconsistent with the project branch requirement**  
   Requirement 2 expects push before draft PR create for the working branch when work begins.  
   The watcher does perform push-before-create correctly in sequence, but it pushes/creates using the daemon branch value passed in (`branch_name`), not the synced project branch that actually advances.  
   This can lead to creating/finding the wrong PR head ref in real multi-branch scenarios.

3. **Conformance coverage for required test #11 is incomplete in behavior depth**  
   The required test calls for `--pr-url` plumbing through child args **and timing correctness**.  
   Current validate test `pr_runtime::pr_url_plumbed_through_child_args` only checks CLI arg parsing (`src/validate/tests_pr_runtime.rs`), not end-to-end dispatch timing/propagation at child spawn time relative to PR lookup/watcher behavior.

## Recommended Next Features
1. **Fix branch source-of-truth in daemon dispatch/runtime**
   - Resolve the actual current branch after `sync_project_branch` and use that for:
     - `find_existing_pr`
     - `draft_pr_watcher` branch argument
     - `ChildHandle.branch`
   - Keep exact `--head` matching, but against the real active branch.

2. **Add a validate test that proves branch-correct PR lookup/create in dispatch**
   - Simulate multiple PRs and assert the daemon chooses the PR for the exact active project branch (not daemon branch).
   - Assert watcher `git push` + `gh pr create --head <active-branch>` use the same branch.

3. **Strengthen required test #11 to include spawn timing path**
   - Validate that resolved `pr_url` is passed to `spawn_ralph_auto`/`spawn_ralph_run` in the real dispatch path (not only clap parse), and that watcher suppression/creation timing is correct when `pr_url` is already known.
