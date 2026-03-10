---
artifact: quick-dev-apply-fixes
loop: 15
iteration: 1
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T12:57:00Z
---

# Apply Reviewer-Requested Fixes

Both reviewer-requested changes have been applied and all 1084 tests pass.

## Fix 1 (High): PR comment polling resilience against malformed/null-author comments

**Problem:** A single comment with `user: null` would fail deserialization of the entire endpoint response, blocking all comment processing for that PR.

**Changes in `src/daemon/github.rs`:**

1. **Made `user` fields optional** on all three raw structs (`RawPullComment`, `RawIssueComment`, `RawReview`) — changed from `user: RawUser` to `#[serde(default)] user: Option<RawUser>`.

2. **Skip entries with missing/empty login** — each parse loop now checks `match &c.user { Some(u) if !u.login.is_empty() => ..., _ => { warn + continue } }`.

3. **Handle parse errors as per-endpoint warnings** — extracted parsing into three testable helpers (`parse_pull_comments`, `parse_issue_comments`, `parse_review_summaries`) that return `Vec::new()` on parse failure instead of propagating `?`. The `fetch_pr_review_comments` async function now calls these helpers, so a parse error on one endpoint doesn't block the others.

4. **Updated existing tests** that accessed `.user.login` to use `.user.as_ref().unwrap().login`.

5. **Added 9 new tests:**
   - `parse_pull_comments_skips_null_user_keeps_valid`
   - `parse_pull_comments_skips_missing_user_field`
   - `parse_pull_comments_skips_empty_login`
   - `parse_pull_comments_returns_empty_on_malformed_json`
   - `parse_issue_comments_skips_null_user_keeps_valid`
   - `parse_issue_comments_returns_empty_on_malformed_json`
   - `parse_review_summaries_skips_null_user_keeps_valid`
   - `parse_review_summaries_returns_empty_on_malformed_json`
   - `parse_review_summaries_skips_empty_body_even_with_valid_user`

## Fix 2 (Medium): `dispatch_task` fail-fast before drain/reset

**Problem:** For `PrReviewResume` dispatches, `drain_staged_amendments` and `reset_project_state_for_resume` ran before the resume validity check, causing side effects even when the dispatch would be immediately rejected.

**Changes in `src/daemon/runtime.rs`:**

1. **Moved the fail-fast check before drain/reset** — the `if origin == PrReviewResume && !resume_existing_project { return Err(...) }` guard now runs immediately after `should_resume_issue_project`, before any drain/reset logic executes.

2. **Simplified the post-drain branch** — removed the now-impossible `else if origin == PrReviewResume` arm from the post-drain `if resume_existing_project` block.

**Regression test in `src/daemon/pr_review.rs`:**

- `staged_amendments_preserved_when_resume_invalid` — stages an amendment, creates a project dir without `prompt.md`, and asserts that staged amendments remain intact and no `amendment-queue` directory is created.
