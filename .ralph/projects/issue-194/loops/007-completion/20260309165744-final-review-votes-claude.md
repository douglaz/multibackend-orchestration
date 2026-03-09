---
artifact: final-review-votes
loop: 7
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T16:57:44Z
---

# Vote Results

## Amendment: A-194-REVIEW-001

### Vote
ACCEPT

### Rationale
Code inspection confirms the problem. `drain_amendment_queue_with_hook` deletes `.inflight` files on disk at `src/project/amendments.rs:239` (`fs::remove_file(&inflight_path)?`) and returns parsed data only in-memory. Both call sites consume this in-memory vector and then proceed to fallible operations:

- **Standard orchestrator** (`orchestrator.rs:603`): drains, then `build_planner_prompt` at L623 and `execute_with_parse_retries` at L660 can both propagate errors via `?`, dropping the in-memory amendments.
- **Quick-dev orchestrator** (`quick_dev_orchestrator.rs:345`): drains, then `execute_backend` at L363 can fail with `.await?`, same loss path.

There is no recovery or re-enqueue logic anywhere in the codebase for these failure paths. A transient backend timeout or template error permanently destroys externally submitted amendments with no trace. The proposed fix — holding drained amendments in memory and re-enqueuing on phase failure — is a sound at-least-once delivery pattern that directly addresses the gap without over-engineering.
