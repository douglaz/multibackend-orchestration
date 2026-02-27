Here is the engineering specification:

---

## Summary

Modify the daemon's task dispatch flow to post two GitHub issue comments as pipeline artifacts are created during `ralph auto` execution: (1) the Quick PRD specification immediately after generation, and (2) the final reviewed prompt immediately after prompt review completes. Comments use idempotent markers (HTML comments) to prevent duplicates on retries, consistent with the existing `refined-prompt` comment mechanism.

The core architectural challenge is that Quick PRD generation and prompt review both execute inside a **child process** (`ralph auto`) that has no knowledge of GitHub context (owner, repo, issue_number). The daemon process holds this context but cannot directly call into the child's pipeline steps. The chosen approach is to have the daemon **poll for artifact files** produced by the child process and post comments when they appear, using a lightweight file-watcher loop that runs concurrently with child execution.

## Acceptance Criteria

- [ ] Quick PRD spec (`SPEC.md`) is posted as a GitHub issue comment with header `### Quick PRD` immediately after the file is written by the child process
- [ ] Reviewed prompt (`prompt.md`, specifically the content written after prompt review) is posted as a GitHub issue comment with header `### Final Prompt (after review)` immediately after prompt review completes in the child process
- [ ] Comments use idempotent markers (`<!-- ralph:task:{task_id}:quick-prd -->` and `<!-- ralph:task:{task_id}:final-prompt -->`) to prevent duplicates on retry or re-dispatch
- [ ] On retried/re-dispatched tasks, existing comments with matching markers are not re-posted
- [ ] If the child process fails before producing an artifact, no comment is posted for that artifact (no partial/empty comments)
- [ ] Comments are posted as soon as each artifact file appears, not deferred to child completion
- [ ] The daemon's existing behavior (label management, refined-prompt comment, PR flow) is unchanged

## Technical Approach

**Architecture Decision: Daemon-Side File Polling.** Since the child process runs as a separate OS process with no GitHub context, and we want comments posted immediately when artifacts are created, the daemon will spawn a concurrent async task that polls for known artifact file paths in the child's worktree. When a file appears, the daemon reads its content and posts an idempotent comment.

The artifact locations are deterministic:
1. **Quick PRD spec**: `.ralph/quick-prd/*/SPEC.md` in the worktree (glob, exactly one match)
2. **Reviewed prompt**: Detected by watching for `.ralph/projects/*/prompt-original.md` (the backup file created when prompt review runs). When it appears, adjacent `prompt.md` contains the final reviewed prompt.

The watcher is spawned via `tokio::spawn` after the child process, polls at 2-second intervals, uses `post_idempotent_comment()` with phases `"quick-prd"` and `"final-prompt"`, and is cancelled via a `CancellationToken` when the child exits.

Comment format uses `### Quick PRD` and `### Final Prompt (after review)` headers, with the full file contents as the body.

Edge cases handled: child fails before artifact creation (no comment posted), `skip_prompt_review` (no final-prompt comment), retried tasks (idempotent markers prevent duplicates), `ralph run --project` resume path (only monitors for prompt review if not yet posted).

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/runtime.rs` | Add `post_artifact_comments()` async fn; modify `dispatch_task()` to spawn watcher; modify `collect_children()` to cancel/join watcher |
| `src/daemon/mod.rs` | Add `watcher_cancel` and `watcher_handle` fields to `ChildHandle` |
| `Cargo.toml` | Add `tokio-util` dependency for `CancellationToken` if not already present |

~120-150 lines of new code, ~20 lines of modified code. No changes to `github.rs`, `auto.rs`, `quick.rs`, or `orchestrator.rs`.

## Testing Strategy

1. **Unit test**: Watcher discovers SPEC.md via glob and posts correct phase/body
2. **Unit test**: Watcher uses `prompt-original.md` as signal to read `prompt.md` for final-prompt
3. **Unit test**: Watcher exits promptly on cancellation with no artifacts present
4. **Unit test**: Watcher handles missing artifacts gracefully (no panics/errors)
5. **Integration test (single-iteration)**: Full dispatch with mock backend, verify both comments posted
6. **Idempotency test**: Double-dispatch same task_id, verify single comment per phase

## Out of Scope

- Modifying `ralph auto` CLI or `QuickPrdPipeline` to accept GitHub context
- Posting comments for intermediate artifacts (draft.md, review-N.json, revision-N.md)
- Editing/updating existing comments if spec changes on retry (first-write-wins)
- Webhook/callback mechanism from child to daemon
- Changes to the existing `refined-prompt` comment