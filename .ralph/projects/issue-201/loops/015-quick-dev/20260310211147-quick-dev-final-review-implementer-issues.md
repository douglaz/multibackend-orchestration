---
artifact: quick-dev-final-review
loop: 15
project: issue-201
backend: claude
role: implementer
created_at: 2026-03-10T21:11:47Z
---

Good — the 5 root-level impl-notes files were properly deleted. These are stray artifacts from previous development loops.

Now I have a comprehensive picture. Let me compile my final review.

# Final Review: AMENDMENTS

## Summary

I reviewed all 27 changed files (5,891 insertions, 935 deletions) across the branch. The implementation adds a PR review polling system (`src/daemon/pr_review.rs`), integrates it into the daemon runtime loop (`src/daemon/runtime.rs`), extends the GitHub API surface (`src/daemon/github.rs`), refactors the quick-dev final-review handoff mechanism (`src/workflow/quick_dev_orchestrator.rs`), and adds comprehensive conformance tests (`src/validate/tests_pr_review.rs`). Compilation succeeds (`cargo check` passes) and all unit tests pass (60+ tests). The overall design is sound: crash-recovery via durable markers, atomic writes, dedup state, and proper label rollback. The stray impl-notes deletions are appropriate cleanup. However, I found three concrete issues worth addressing.

## Amendment: PRR-HANDOFF-EMPTY-SECTION

### Problem

`format_final_review_handoff` at `src/workflow/quick_dev_orchestrator.rs:1287-1297` unconditionally includes both `### Reviewer Final Review Findings` and `### Implementer Final Review Findings` headers even when one body is empty. The caller `load_final_review_findings` (line 1335) guards against *both* being empty, but when only one reviewer found issues, the handoff contains an empty section like:

```
### Implementer Final Review Findings
<empty>
```

This wastes prompt tokens and could confuse the implementer LLM into thinking there are implementer findings to close when there are none.

### Proposed Change

`[P3]` — In `format_final_review_handoff`, only include a section when the corresponding body is non-empty:

```rust
fn format_final_review_handoff(impl_body: &str, rev_body: &str) -> String {
    let mut handoff = String::from(
        "This implementation round was reopened by final review. ...\n\n",
    );
    if !rev_body.trim().is_empty() {
        handoff.push_str("### Reviewer Final Review Findings\n");
        handoff.push_str(rev_body);
        handoff.push('\n');
    }
    if !impl_body.trim().is_empty() {
        handoff.push_str("\n### Implementer Final Review Findings\n");
        handoff.push_str(impl_body);
    }
    handoff
}
```

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - conditionally include sections in `format_final_review_handoff`

---

## Amendment: PRR-DEDUP-STATE-SAVE-REVERT-INCOMPLETE

### Problem

In `src/daemon/pr_review.rs:705-743`, the dedup state persistence failure handling has a subtle gap. When `state.save()` fails at line 718:

1. The staged file was already written atomically (line 704-705) 
2. The code reverts the in-memory key (line 728) and attempts to delete the staged file (line 736)
3. If `fs::remove_file` also fails (line 736-740), the staged file persists as an orphan

On the *next* poll cycle, `PrReviewState::load()` loads fresh state from disk (which never recorded this key), and `stage_amendment()` at line 141-146 detects the existing file is valid with matching id/source — so it returns `Ok(())` idempotently. The key then gets re-inserted and `state.save()` is attempted again. If save keeps failing, this creates an infinite retry-without-progress loop that logs a warning each cycle but never escalates.

This is not a data-loss bug (the amendment is staged and will eventually be consumed), but the lack of escalation means a persistent I/O failure on the state file silently blocks new comments from being processed for that task indefinitely.

### Proposed Change

`[P2]` — Track consecutive save failures per task and skip the task entirely (with a more prominent warning) after N consecutive failures (e.g., 3), or at minimum, log at `warn!` level via `tracing` rather than `eprintln!` so structured logging can detect the pattern.

### Affected Files
- `src/daemon/pr_review.rs` - add failure tracking or escalation in `poll_pr_reviews` save-failure path

---

## Amendment: PRR-POLL-AUTH-SILENT-SUCCESS

### Problem

In `src/daemon/pr_review.rs:597-604`, when `fetch_authenticated_login_with_gh_bin` fails, the function returns `Ok(Vec::new())` — a successful empty result. This means the caller (`pr_review_phase`) has no signal that polling was skipped due to an auth failure versus there simply being no tasks to process. A persistent auth misconfiguration (expired token, wrong `gh_bin` path) would silently disable the entire PR review feature with only an `eprintln!` warning that could easily be missed in a busy daemon log.

### Proposed Change

`[P2]` — Return `Err` on auth failure so `pr_review_phase` can log it as a phase-level warning and still fall through to process already-staged amendments. Alternatively, use `tracing::warn!` instead of `eprintln!` and add a distinguishing prefix so monitoring can detect this specific failure mode.

### Affected Files
- `src/daemon/pr_review.rs` - change auth failure handling in `poll_pr_reviews` to propagate error or use structured logging

---
