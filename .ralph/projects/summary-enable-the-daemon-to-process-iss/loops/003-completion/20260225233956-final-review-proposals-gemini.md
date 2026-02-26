---
artifact: final-review-proposals
loop: 3
project: summary-enable-the-daemon-to-process-iss
backend: gemini
role: final_reviewer
created_at: 2026-02-25T23:39:56Z
---

# Final Review: AMENDMENTS

## Amendment: IPD-SPEC-PARSE-UNWRAP

### Problem
In `src/daemon/interactive_prd.rs`, the function `extract_approved_spec_from_comments` contains a risky `unwrap()` call on line 210:
```rust
208:    let revision = caps.get(1).unwrap().as_str().parse::<u32>().unwrap();
```
If the capture group `1` is missing for some reason (e.g., a bug in the regex `STATUS_APPROVED_RE` or unexpected comment format), `caps.get(1).unwrap()` will panic, crashing the daemon. The second `.unwrap()` on `parse::<u32>()` will also panic if the revision number is not a valid `u32`. While the regex is designed to prevent this, robust code should avoid `unwrap()` on externally-derived data. A `continue` within the loop would be safer, allowing parsing to proceed with other comments instead of crashing the entire process.

### Proposed Change
Replace the `unwrap()` calls with a `let...else` block that safely destructures the captures and parses the revision. If parsing fails, it should `continue` to the next comment, logging the failure in debug mode. This makes the comment parsing logic more resilient to malformed or unexpected comment content.

### Affected Files
- `src/daemon/interactive_prd.rs` - Replace `unwrap()` with safe parsing and a `continue` on failure.

---
## Context Provided


## Master Prompt

### Title
Enable daemon dispatch for `ralph:prd-done` issues and recover approved PRD spec deterministically.

### Objective
Update daemon dispatch so issues that completed interactive PRD (`ralph:prd-done`) can be claimed and dispatched when `ralph:ready` is present. For those issues, task input must come from the latest approved PRD draft comment when available, with a safe fallback to issue title/body.

### In Scope
1. Narrow PRD label skip logic to in-progress labels only, with `ralph:prd-done` precedence.
2. Centralize draft heading/footer format into shared constants used by both draft generation and extraction.
3. Add approved-spec extraction from GitHub comments using highest approved revision.
4. Use extracted spec as `raw_idea` for `ralph:prd-done` issues in daemon dispatch path.
5. Add unit and validate conformance tests for new behavior.

### Out of Scope
- Changing PRD state machine variants or transitions.
- Changing downstream `dispatch_task()` or prompt refinement behavior.
- Auto-applying `ralph:ready` when PRD completes.
- Altering existing label lifecycle beyond the precedence rule above.
- Caching bot login across daemon iterations.

### Required Behavior

#### 1) Label gating in claim loop
1. Add `interactive_prd::has_in_progress_prd_label(labels: &[String]) -> bool`.
2. In-progress set is exactly: `ralph:prd`, `ralph:prd-active`, `ralph:prd-approved`, `ralph:prd-failed`.
3. If `ralph:prd-done` is present, function must return `false` even if in-progress labels are also present.
4. Keep existing `has_prd_label()` behavior unchanged for other call sites.
5. In `poll_and_claim()`, replace current PRD skip guard with `has_in_progress_prd_label()`.

#### 2) Shared draft format constants
1. In `src/daemon/interactive_prd.rs`, define:
   - `pub const DRAFT_HEADING_PREFIX: &str`
   - `pub const DRAFT_FOOTER: &str`
   - `pub fn format_draft_comment(revision: u32, spec_body: &str) -> String`
2. Replace inline draft comment formatting in both draft-posting paths with `format_draft_comment(...)`.
3. Extraction logic must use these same constants to prevent drift.

#### 3) Approved spec extraction
1. Add `pub fn extract_approved_spec(gh_bin: &str, owner: &str, repo: &str, issue_number: u32) -> Option<String>`.
2. Add a pure helper for testability (name is flexible) that parses approved revision + draft body from comments.
3. Extraction rules:
   - Resolve bot login from authenticated GitHub identity.
   - Fetch issue comments.
   - Consider only bot-authored comments.
   - Parse all `status-approved-vN` markers for this issue and choose highest `N`.
   - Find matching `draft-vN` bot comment.
   - If multiple `draft-vN` comments exist, use the latest one in API order.
   - Strip PRD marker lines.
   - Strip heading only if the first content line starts with `DRAFT_HEADING_PREFIX`.
   - Strip footer only if the trailing content line exactly matches `DRAFT_FOOTER`.
   - Trim surrounding whitespace.
   - Return `None` if any required step fails or cleaned body is empty.

#### 4) Dispatch input selection for `ralph:prd-done`
1. In `poll_and_claim()`, after claim succeeds and before dispatch:
   - If issue has `ralph:prd-done`, attempt `extract_approved_spec(...)` via existing blocking wrapper path.
   - On `Some(spec)`, dispatch with `raw_idea = spec`.
   - On `None` or error, fallback to `compose_raw_idea(&issue.title, issue.body.as_deref())`.
2. For non-`ralph:prd-done` issues, keep existing `compose_raw_idea(...)` path unchanged.

#### 5) Logging requirements
1. On successful PRD spec use, emit stderr containing substring: `prd-done: using approved spec`.
2. On fallback for PRD-done issue, emit stderr containing substring: `approved spec not found, falling back`.
3. Existing verbose skip logging should describe “in-progress PRD label” (not generic PRD label).

### Statelessness Constraint
- Do not read or depend on local daemon `InteractivePrdState` for this feature.
- All approved-spec recovery must derive from live issue comments and authenticated bot identity.

### Files To Change
1. `src/daemon/interactive_prd.rs`
2. `src/daemon/runtime.rs`
3. `src/validate/tests_interactive_prd.rs` (new)
4. `src/validate/mod.rs` (register new validate test module)

### Acceptance Criteria
1. `ralph:prd-done` + `ralph:ready` issues are claimed and dispatched.
2. In-progress PRD labels still block dispatch when `ralph:prd-done` is absent.
3. Mixed labels including `ralph:prd-done` are allowed through.
4. For PRD-done issues, dispatched `raw_idea` is approved draft body (markers/heading/footer removed).
5. Highest approved revision is used when multiple approvals exist.
6. User-authored spoofed markers are ignored.
7. Missing marker/matching draft/API failure/login failure/empty cleaned body all trigger fallback.
8. Fallback uses `compose_raw_idea(title, body)` exactly.
9. Shared draft format constants are used by both posting and extraction.
10. Existing in-progress PRD workflow behavior remains unchanged.

### Test Requirements

#### Unit tests (`src/daemon/interactive_prd.rs`)
1. `has_in_progress_prd_label` positive/negative cases.
2. `ralph:prd-done` precedence in mixed-label sets.
3. `has_prd_label` existing semantics unchanged.
4. Approved revision parsing selects highest `N`.
5. Bot-only filtering ignores user spoofing.
6. Draft cleanup removes marker/heading/footer and trims whitespace.
7. Empty post-cleanup result returns `None`.
8. Duplicate `draft-vN` uses latest in API order.
9. Round-trip with `format_draft_comment` remains consistent.

#### Validate conformance tests (`src/validate/tests_interactive_prd.rs`)
1. PRD-done issue dispatches with approved spec as idea.
2. Mixed labels (`prd-done` + `prd-approved`) are not blocked.
3. Missing markers fall back to title/body and warn.
4. Comments API failure falls back and warns.
5. User-spoofed marker does not affect selected revision.
6. Highest approved revision wins in end-to-end daemon run.
7. Existing in-progress conflict tests remain passing.

### Implementation Notes
- Avoid line-number-based assumptions; use symbol-level edits.
- Keep public API additions minimal and intentional.
- Preserve backward compatibility outside the behaviors explicitly listed above.

## Project State

```json
{
  "project_id": "summary-enable-the-daemon-to-process-iss",
  "project_name": "## Summary\n\nEnable the daemon to process issues that have co",
  "created_at": "2026-02-25T22:42:16.204881529Z",
  "prompt_file": "prompt.md",
  "prompt_hash": "197e858021b6dbe19afad65be3ba8de88189a30e5e3905a9d45dc200954b2a70",
  "prompt_hash_at_loop_start": "197e858021b6dbe19afad65be3ba8de88189a30e5e3905a9d45dc200954b2a70",
  "prompt_review_completed": true,
  "parent_project": null,
  "current_loop": 3,
  "current_phase": "final_review",
  "phase_iteration": 1,
  "status": "in_progress",
  "loops": [
    {
      "loop_number": 1,
      "slug": "label-gating-refinement-and-shared-draft-format",
      "feature_name": "Label gating refinement and shared draft format constants",
      "loop_type": "feature",
      "status": "completed",
      "backends": {
        "planner": "claude(opus)",
        "implementer": "codex(gpt-5.3-codex-high)",
        "reviewer": "claude(opus)",
        "qa": "codex(gpt-5.3-codex-high)"
      },
      "artifacts": {
        "spec": "loops/001-label-gating-refinement-and-shared-draft-format/20260225224540-spec.md",
        "impl_notes": "loops/001-label-gating-refinement-and-shared-draft-format/20260225224806-impl-notes.md",
        "reviews": [],
        "approval": "loops/001-label-gating-refinement-and-shared-draft-format/20260225224955-review-approved.md",
        "qa_results": [],
        "pending_qa_feedback": null
      },
      "commit": "a0e866f5e540b85ee05801056e1128e571d696e4",
      "started_at": "2026-02-25T22:45:40.146363868Z",
      "completed_at": "2026-02-25T22:49:57.428117370Z"
    },
    {
      "loop_number": 2,
      "slug": "approved-prd-spec-recovery-and-prd-done-dispatch",
      "feature_name": "Approved PRD Spec Recovery and `prd-done` Dispatch Input Selection",
      "loop_type": "feature",
      "status": "completed",
      "backends": {
        "planner": "codex(gpt-5.3-codex-xhigh)",
        "implementer": "claude(opus)",
        "reviewer": "codex(gpt-5.3-codex-high)",
        "qa": "claude(opus)"
      },
      "artifacts": {
        "spec": "loops/002-approved-prd-spec-recovery-and-prd-done-dispatch/20260225225223-spec.md",
        "impl_notes": "loops/002-approved-prd-spec-recovery-and-prd-done-dispatch/20260225230105-impl-notes.md",
        "reviews": [
          {
            "iteration": 1,
            "feedback": "loops/002-approved-prd-spec-recovery-and-prd-done-dispatch/20260225230420-review-001-feedback.md",
            "response": "loops/002-approved-prd-spec-recovery-and-prd-done-dispatch/20260225231512-impl-response-001.md"
          }
        ],
        "approval": "loops/002-approved-prd-spec-recovery-and-prd-done-dispatch/20260225231926-review-approved.md",
        "qa_results": [],
        "pending_qa_feedback": null
      },
      "commit": "8ef0f7c07d5cb026b77b1c62d5235030d6ea33d4",
      "started_at": "2026-02-25T22:52:23.435488242Z",
      "completed_at": "2026-02-25T23:19:28.526592193Z"
    }
  ],
  "completion_attempts": [
    {
      "loop_number": 3,
      "slug": "completion",
      "loop_type": "completion",
      "status": "completed",
      "backends": {
        "planner": "claude(opus)",
        "completers": [
          "claude(opus)",
          "codex(gpt-5.3-codex-xhigh)",
          "gemini(gemini-3-pro-preview)"
        ]
      },
      "artifacts": {
        "termination_request": "loops/003-completion/20260225232102-termination-request.md",
        "verdict": "loops/003-completion/20260225232911-completer-verdict-gemini-gemini-3-pro-preview.md",
        "acceptance_results": []
      },
      "verdict": "complete",
      "started_at": "2026-02-25T23:21:02.574721155Z",
      "completed_at": "2026-02-25T23:29:11.102526561Z"
    }
  ],
  "session_store": {
    "records": []
  }
}
```
