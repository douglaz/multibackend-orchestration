---
artifact: final-review-votes
loop: 6
project: issue-214
backend: claude
role: final_reviewer
created_at: 2026-03-19T06:51:19Z
---

# Vote Results

## Amendment: ORACLE-REVIEW-FR-001

### Vote
ACCEPT

### Rationale
I verified every claim against the current source and they all hold:

1. **The error boundary mismatch is confirmed.** `post_bot_comment_with_marker_metadata_with_gh_bin` (github.rs:2208) performs the actual `gh issue comment` at line 2230, checks for a zero exit at line 2250, and then immediately does a readback via `find_bot_comment_with_marker_with_gh_bin` at line 2257-2258. That readback delegates to `fetch_issue_comments_with_gh_bin` (github.rs:1923), which runs `gh issue view --json comments` and can fail independently. A readback failure propagates as `Err` from the whole function, even though the comment is already on GitHub.

2. **The cap/state impact is real.** In `oracle_review.rs:240-248`, both `success_count` increment and `state.mark_reviewed` are gated on the `Ok(_)` arm. A transient readback failure therefore does not consume the per-cycle cap at line 151, and does not persist the `(pr_number, head_sha)` pair. The loop continues and can post more reviews than `daemon_oracle_review_max_per_cycle` allows in a single cycle.

3. **The `already_exists` conflation is valid.** The pre-post idempotency check at github.rs:2217-2228 can return `Ok(Some(existing))` for a comment posted by a concurrent cycle or a previous cycle whose state was lost. The caller at oracle_review.rs:240-241 counts this as a fresh post toward `success_count`, slightly misrepresenting actual posting activity.

4. **The validate gap is confirmed.** The mock `gh` script handles `issue view` with `want_comments` at tests_daemon_oracle_review.rs:1164-1167 by always calling `emit_comments` and exiting 0. The only failure injection is `MOCK_GH_FAIL_COMMENT_PR` at line 1189, which fails the `issue comment` command itself. There is no mechanism to fail readback while succeeding on post.

The proposed fix — splitting the return type into `already_exists`, `posted`, and `post_failed` — cleanly addresses all three issues (misclassification, cap overshoot, and state loss) and the new validate case fills a genuine coverage gap. The scope is appropriate and the change is safe because it narrows the error surface rather than broadening it.
