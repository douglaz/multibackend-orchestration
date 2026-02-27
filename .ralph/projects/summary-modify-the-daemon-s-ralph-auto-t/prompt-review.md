---
artifact: prompt-review
project: summary-modify-the-daemon-s-ralph-auto-t
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-21T05:49:02Z
---

# Prompt Review

## Issues Found
- "Immediately" is ambiguous; the prompt says 2s polling but gives no measurable latency target.
- Artifact discovery is not scoped to the current child run, so stale files from prior runs could trigger comments.
- "Glob, exactly one match" for `SPEC.md` is brittle; multiple matches are possible and selection is undefined.
- `prompt-original.md` as a trigger can race with `prompt.md` write completion, risking partial/incorrect final prompt comments.
- Canceling watcher on child exit can miss artifacts created just before exit unless a final sweep is required.
- Failure handling for GitHub API calls is underspecified (retry policy, logging, and whether dispatch should fail).
- Idempotency depends on `{task_id}`, but retry behavior is unclear if `task_id` changes across redispatch.
- Resume/re-dispatch behavior is vague; no deterministic rule for what to watch/post when artifacts already exist.
- Test plan misses key edge cases above and does not explicitly require validate conformance coverage for this feature.

## Refined Prompt
Implement daemon-side posting of two GitHub issue comments during `ralph auto` child execution, based on artifact file appearance in the child worktree.

### Objective
Post pipeline artifacts as GitHub comments as soon as they are available, without changing child-process architecture:
1. Quick PRD: comment from `SPEC.md`
2. Final reviewed prompt: comment from `prompt.md` after prompt review signal appears

### Constraints
- Child process (`ralph auto`) has no GitHub context.
- Daemon owns GitHub context (`owner`, `repo`, `issue_number`, `task_id`) and must do all posting.
- Do not modify `ralph auto`, quick PRD pipeline internals, or prompt review internals.
- Preserve existing behavior for labels, PR flow, and existing refined-prompt comment logic.

### Required Behavior
- Spawn a daemon watcher task when dispatching the child process (only when GitHub context exists).
- Watcher polls every 2 seconds.
- Define `child_start_time` at child spawn; ignore artifacts with mtime older than this timestamp.
- Post comments idempotently via HTML markers:
  - `<!-- ralph:task:{task_id}:quick-prd -->`
  - `<!-- ralph:task:{task_id}:final-prompt -->`
- Comment headers:
  - `### Quick PRD`
  - `### Final Prompt (after review)`
- Comment body is full artifact content (subject to GitHub comment size limits; truncate safely with a clear "[truncated]" note if needed).
- Do not post empty/partial comments:
  - Quick PRD: file must exist and be readable.
  - Final prompt: `prompt-original.md` signal must appear, and adjacent `prompt.md` must exist, be readable, and non-empty.
- On child exit, perform one final watcher sweep before shutdown to avoid race-related misses.
- If artifact never appears, do not post that comment.
- If GitHub post fails transiently, retry on subsequent polls until watcher ends; do not crash dispatch.
- If comment with marker already exists, skip posting (idempotent on retry/re-dispatch).

### Artifact Detection Rules
- Quick PRD candidate files: `.ralph/quick-prd/*/SPEC.md`
- If multiple candidates exist after `child_start_time`, choose the newest by mtime; if tied, lexical path order.
- Final prompt signal: `.ralph/projects/*/prompt-original.md` (mtime >= `child_start_time`)
- Final prompt source: sibling `prompt.md` in the same directory as signal file.

### Implementation Scope
- `src/daemon/runtime.rs`
  - Add `post_artifact_comments(...)` async watcher.
  - Start watcher in `dispatch_task(...)` after child spawn.
  - Ensure final sweep + shutdown coordination in child collection path.
- `src/daemon/mod.rs`
  - Extend `ChildHandle` with watcher cancellation + join handle.
- `Cargo.toml`
  - Use existing cancellation utilities if available; otherwise add `tokio-util` for `CancellationToken`.

### Acceptance Criteria
- [ ] Quick PRD comment is posted within 3 seconds of `SPEC.md` becoming readable.
- [ ] Final prompt comment is posted within 3 seconds of reviewed `prompt.md` becoming readable after prompt-review signal.
- [ ] Markers prevent duplicate comments across retries/re-dispatch.
- [ ] No comment is posted for missing artifacts.
- [ ] Child failure before artifact creation results in no artifact comment.
- [ ] Existing daemon behaviors remain unchanged.
- [ ] No missed post at child-exit boundary (final sweep implemented).

### Testing Requirements
Add both unit/integration coverage and validate conformance coverage.

1. Unit: quick-prd detection posts correct marker/header/body.
2. Unit: final prompt uses `prompt-original.md` as signal and reads sibling `prompt.md`.
3. Unit: ignores stale artifacts older than `child_start_time`.
4. Unit: multiple `SPEC.md` candidates choose deterministic newest file.
5. Unit: cancellation/final sweep behavior (no missed artifact at shutdown boundary).
6. Unit: GitHub post failure retries without panic.
7. Integration: single dispatch posts both comments with mock GitHub.
8. Idempotency: re-dispatch/retry does not duplicate either comment.
9. Validate: add conformance case(s) that exercise artifact-comment behavior in real binary flow with mocks.

### Out of Scope
- Webhook/callback from child to daemon.
- Posting intermediate artifacts (draft/review/revision files).
- Editing/replacing already-posted artifact comments.
- Changes to existing refined-prompt comment semantics.
