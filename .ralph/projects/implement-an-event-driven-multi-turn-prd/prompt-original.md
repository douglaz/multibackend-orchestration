I now have a thorough understanding of the codebase. Let me write the specification.

## Summary

Add an event-driven, multi-turn interactive PRD workflow to the Ralph daemon. When a user applies the `ralph:prd` label to a GitHub issue, the daemon enters a state-machine-driven conversation: it generates clarifying questions using two backends, waits for user answers via issue comments, drafts a 6-section engineering spec, and iterates on feedback until the user approves. All state persists to disk as JSON, making the workflow resilient to daemon restarts. The workflow operates within the existing 60-second poll cycle with no long-running child processes — each poll tick advances the state machine by one step at most.

## Acceptance Criteria

- [ ] Daemon polls for issues labeled `ralph:prd` alongside existing `ralph:ready` issues
- [ ] Five new lifecycle labels created at startup: `ralph:prd`, `ralph:prd-active`, `ralph:prd-approved`, `ralph:prd-done`, `ralph:prd-failed`
- [ ] On first pickup: label swapped `ralph:prd` → `ralph:prd-active`, clarifying questions generated using 2 backends (e.g. claude + codex), synthesized into a single question set, posted as an idempotent comment
- [ ] On subsequent poll: user reply detected by scanning comments after the questions comment timestamp; reply text extracted and used to generate a draft spec
- [ ] Draft generated using the existing quick-prd writer/reviewer pattern (2 backends: writer drafts, reviewer validates sections), posted as an idempotent comment with the 6 required sections
- [ ] Approval detection: comment containing "approved", "lgtm", or "ship it" (case-insensitive), OR the `ralph:prd-approved` label being applied
- [ ] Revision loop: non-approval feedback extracted from user comments → revision prompt sent to writer backend → updated draft posted as a new comment
- [ ] On approval: final spec posted/updated, label swapped to `ralph:prd-done`
- [ ] On unrecoverable error: label swapped to `ralph:prd-failed`, error posted as comment
- [ ] State machine persisted to `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json` with serde, surviving daemon restarts
- [ ] Existing `ralph:ready` workflow unaffected — no label conflicts, no behavioral regressions
- [ ] All existing tests pass; new unit tests cover state transitions, comment parsing, question synthesis, and approval detection

## Technical Approach

### State Machine

```
                  ┌─────────────┐
                  │   Pending   │  (ralph:prd detected)
                  └──────┬──────┘
                         │ generate questions (2 backends → synthesize)
                         ▼
              ┌──────────────────┐
              │ AwaitingAnswers  │  (questions posted as comment)
              └────────┬─────────┘
                       │ user reply detected
                       ▼
              ┌──────────────────┐
              │ GeneratingDraft  │  (writer+reviewer pipeline)
              └────────┬─────────┘
                       │ draft posted as comment
                       ▼
              ┌──────────────────┐
              │ AwaitingFeedback │◄───────────┐
              └────────┬─────────┘            │
                       │                      │
            ┌──────────┴──────────┐           │
            ▼                     ▼           │
    ┌──────────────┐    ┌─────────────────┐   │
    │   Approved   │    │    Revising     │───┘
    └──────┬───────┘    └─────────────────┘
           │
           ▼
    ┌──────────────┐
    │     Done     │  (ralph:prd-done)
    └──────────────┘
```

States are represented as a Rust enum (`PrdWorkflowState`) with associated data (timestamps, draft content, revision number, comment IDs). The full state struct (`InteractivePrdState`) is serialized to JSON on every transition.

### Integration with Daemon Poll Cycle

The interactive PRD workflow runs **inline** in the daemon's existing poll loop — no child processes. A new function `poll_and_advance_prd_issues()` is called in the main loop body (in `runtime::run()`) alongside the existing `poll_and_claim()`. It:

1. Calls `github::poll_issues()` with label `ralph:prd` (separate from `ralph:ready`)
2. For each issue: loads or initializes `InteractivePrdState` from disk
3. Advances the state machine by one step (at most one backend call per issue per cycle)
4. Persists state to disk after each transition

Backend calls use the existing `Backend` trait with `CliBackend` instances. Since these are async and short-lived (single prompt → response), they fit within the poll cycle without blocking other tasks. A per-issue timeout (configurable, default 120s) prevents runaway backend calls.

### Comment Interaction Protocol

Each comment posted by Ralph includes an HTML marker for idempotency:
- Questions: `<!-- ralph:prd:{issue_number}:questions-v{n} -->`
- Draft: `<!-- ralph:prd:{issue_number}:draft-v{n} -->`
- Status: `<!-- ralph:prd:{issue_number}:status -->`

User replies are detected by fetching all issue comments via `gh issue view --json comments` and finding comments posted **after** the most recent Ralph comment that are **not** authored by Ralph (detected via marker absence). This reuses the existing `comment_marker_exists` pattern but extends it with a new `fetch_issue_comments()` function that returns structured comment data (author, body, timestamp).

### Multi-Backend Question Generation

Two backends generate questions independently (parallel `tokio::join!`), then a synthesis step merges them:
1. Backend A (e.g. claude) generates 3-5 clarifying questions from the issue body
2. Backend B (e.g. codex) generates 3-5 clarifying questions independently
3. A synthesis prompt (sent to the primary backend) merges, deduplicates, and prioritizes the combined question set into a final numbered list

### Draft Generation

Reuses the `QuickPrdPipeline` pattern from `src/prd/quick.rs`:
- Writer backend generates the spec using `DRAFT_PROMPT` with the issue body + user answers as the `{{idea}}`
- Reviewer backend validates via `REVIEW_PROMPT`
- Section validation via `check_spec_sections()`
- Up to 2 writer/reviewer revision rounds before posting

### Approval Detection

A helper function `detect_approval(comment_body: &str) -> bool` checks for:
- Case-insensitive exact words: "approved", "lgtm", "ship it", "looks good"
- The `ralph:prd-approved` label is checked separately via `fetch_issue_labels()`

### Configuration

New fields in `WorkspaceConfig`:
- `daemon_prd_enabled: bool` (default: `true`)
- `daemon_prd_question_backends: Vec<String>` (default: `["claude", "codex"]`)
- `daemon_prd_writer_backend: String` (default: `"claude"`)
- `daemon_prd_reviewer_backend: String` (default: `"codex"`)
- `daemon_prd_max_revisions: u32` (default: `3`)

## Files & Modules

### New Files
| File | Purpose |
|------|---------|
| `src/daemon/interactive_prd.rs` | Core module: `InteractivePrdState`, `PrdWorkflowState` enum, state machine `advance()`, question synthesis, approval detection, comment parsing |
| `src/daemon/interactive_prd/state.rs` | State types and serialization (if module grows; otherwise inline in `interactive_prd.rs`) |

### Modified Files
| File | Change |
|------|--------|
| `src/daemon/mod.rs` | Add `pub mod interactive_prd;` |
| `src/daemon/runtime.rs` | Add `poll_and_advance_prd_issues()` call in main loop body after `poll_and_claim()`. Add PRD label constants. Wire `DaemonRuntimeConfig` fields for PRD config. |
| `src/daemon/github.rs` | Add `fetch_issue_comments() -> Vec<IssueComment>` (returns author, body, created_at, id). Extend `REQUIRED_LABELS` with PRD labels. Extend `LIFECYCLE_LABELS` with PRD lifecycle labels. Add `add_label()` / `remove_label()` convenience wrappers if needed. |
| `src/config/global.rs` | Add `daemon_prd_enabled`, `daemon_prd_question_backends`, `daemon_prd_writer_backend`, `daemon_prd_reviewer_backend`, `daemon_prd_max_revisions` to `WorkspaceConfig` with defaults. |
| `src/error.rs` | Add `InteractivePrdFailed(String)` variant to `RalphError`. |
| `src/lib.rs` | No change needed (daemon module already declared). |

### State Persistence Path
```
{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json
```

Example state JSON:
```json
{
  "issue_number": 42,
  "owner": "acme",
  "repo": "widgets",
  "state": "AwaitingFeedback",
  "questions_comment_id": "IC_abc123",
  "questions_posted_at": "2025-06-01T12:00:00Z",
  "draft_revision": 2,
  "current_draft": "## Summary\n...",
  "user_answers": "1. Yes, we need OAuth...",
  "last_advanced_at": "2025-06-01T12:05:00Z",
  "error_count": 0
}
```

## Testing Strategy

### Unit Tests (in `src/daemon/interactive_prd.rs`)
- **State serialization round-trip**: Serialize `InteractivePrdState` to JSON and back for each state variant
- **Approval detection**: Test `detect_approval()` with "approved", "LGTM", "ship it", "looks good", negative cases ("not approved", "I don't approve"), and edge cases (approval keyword in a code block)
- **Comment parsing**: Test extraction of user replies from a list of `IssueComment` structs, filtering out Ralph's own comments (by marker detection)
- **Question synthesis prompt**: Test that the synthesis prompt correctly includes questions from both backends
- **State transition logic**: Test `advance()` with mock backends for each transition (Pending→AwaitingAnswers, AwaitingAnswers→GeneratingDraft, etc.)
- **Error handling**: Test that backend failures increment error count and transition to failed state after threshold

### Integration Tests (in `tests/`)
- **State persistence across simulated restarts**: Write state, re-read, verify continuity
- **Label interaction with existing workflow**: Verify `ralph:prd` labels don't interfere with `ralph:ready` filtering in `filter_claimable()` and `classify_lifecycle_labels()`
- **Full state machine walkthrough with `MockBackend`**: Simulate the complete lifecycle from Pending to Done using mock backend responses and mock GitHub comment data

### Manual E2E Test
- Create a real GitHub issue, apply `ralph:prd`, verify questions appear
- Reply with answers, verify draft appears
- Post feedback, verify revision appears
- Comment "approved", verify `ralph:prd-done` label applied
- Kill and restart daemon mid-workflow, verify it resumes from persisted state

## Out of Scope

- **Auto-triggering implementation after PRD approval**: The `ralph:prd-done` state is terminal; users manually apply `ralph:ready` if they want implementation
- **Webhook-based event delivery**: This design uses polling, not GitHub webhooks
- **Multi-issue PRD dependencies**: Each issue's PRD workflow is independent
- **Rich comment formatting** (collapsible sections, reactions, etc.): Plain markdown only
- **Comment editing**: All interactions are append-only (new comments); existing comments are never edited
- **Authentication/authorization**: Any commenter can approve; no role-based gating
- **PRD template customization**: Uses the fixed 6-section format from quick-prd
- **Concurrent backend calls across multiple PRD issues**: Each issue advances sequentially within a single poll cycle (parallelism across issues may be added later)
- **CLI subcommand for interactive PRD**: No `ralph prd --interactive` CLI; this is daemon-only