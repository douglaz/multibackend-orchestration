---
artifact: final-review-planner-positions
loop: 11
project: implement-an-event-driven-multi-turn-prd
backend: claude(opus)
role: planner
created_at: 2026-02-22T08:42:49Z
---

Good. I now have all the information I need. Let me also quickly verify the master prompt's relevant constraints more carefully to ensure I'm evaluating these amendments accurately against the spec.

The master prompt states:
- "All state transitions are persisted atomically to disk (write temp + rename)" — this is a hard constraint
- "Idempotency: if same marker already exists, do not post duplicate" — for comments
- "Bot comment is determined by `author_login == daemon bot login`, not marker absence" — the bot identity rule

Now I have a thorough understanding of both amendments. Let me compose my positions.

# Planner Positions

## Amendment: PRD-AMD-20260222-01

### Position
ACCEPT

### Rationale
The amendment identifies a genuine violation of the master prompt's hard constraint: "All state transitions are persisted atomically to disk." The code as written performs label mutations (removing `ralph:prd-active`, adding `ralph:prd-done` or `ralph:prd-failed`) before calling `state.save()`. If the save fails — disk full, permission error, crash between label mutation and save — the issue becomes invisible to the polling loop (which only scans `ralph:prd` and `ralph:prd-active`) while on-disk state remains non-terminal. This is exactly the "restart-safe" violation the amendment describes.

The existing code at line 803-824 even includes a comment acknowledging ordering matters ("boundary-safe order: add ralph:prd-done first, remove ralph:prd-active second"), but this ordering only protects against partial label-mutation failure, not against subsequent save failure. The Failed path (lines 1270-1278) has the same issue — labels are mutated with best-effort `let _` ignoring errors, then save is called last.

The proposed fix — keeping `ralph:prd-active` until save succeeds, then removing it — is the correct approach and directly implements the atomic persistence guarantee the master prompt requires. The compensation alternative (re-adding `ralph:prd-active` on save failure) is also acceptable but slightly more complex. Treating save failures under the `error_count`/retry policy is consistent with the existing error handling model.

The scope is well-defined: reorder operations in `do_approval_transition` and `transition_to_failed`, plus targeted tests. This does not expand the feature surface or introduce new abstractions.

## Amendment: PRD-AMD-20260222-02

### Position
ACCEPT

### Rationale
The amendment identifies a real security/correctness gap that conflicts with the master prompt's bot-identity rule. The prompt explicitly states: "Bot comment is determined by `author_login == daemon bot login`, not marker absence." Yet the marker idempotency functions (`find_comment_with_marker` at github.rs:1263 and `post_comment_with_marker_metadata` at github.rs:1298) perform a body-only search with no author filtering — `comments.into_iter().find(|c| c.body.contains(marker))`.

This means a user (or any other bot) could post a comment containing `<!-- ralph:prd:42:questions-v1 -->` and the daemon would treat it as its own marker comment, skipping question generation entirely. Worse, at interactive_prd.rs:463-466, the code hydrates `questions_comment_id` and `questions_posted_at` from whatever comment matches the marker — so a spoofed marker would poison the timestamp cursor used for detecting user answers, potentially causing the daemon to miss real user responses or process the wrong ones.

The fix is well-scoped: add an `author_login` filter parameter to `find_comment_with_marker` (or a parallel bot-scoped variant), and use it in the PRD paths that rely on marker idempotency for state hydration. The `IssueComment` struct already contains `author_login`, so this is a minimal change. The proposed test coverage (spoofed-marker regression test) directly validates the fix.
