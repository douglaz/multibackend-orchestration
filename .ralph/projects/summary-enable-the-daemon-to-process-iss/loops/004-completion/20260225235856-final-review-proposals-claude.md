---
artifact: final-review-proposals
loop: 4
project: summary-enable-the-daemon-to-process-iss
backend: claude
role: final_reviewer
created_at: 2026-02-25T23:58:56Z
---

I'll start by examining the git diff and understanding the changes made in this project.
Let me now read all the changed files in parallel.
The file is too large. Let me read it in chunks and the other files separately.
Now let me read the key new functions - `has_in_progress_prd_label`, `extract_approved_spec`, `parse_approved_spec_from_comments`, and the runtime changes for `poll_and_claim`.
Now let me read the core new functions in detail.
Good. Let me now read the rest of runtime.rs after the dispatch section and the unit tests in interactive_prd.rs.
Now let me read the validate test file for the conformance tests and check the label unit tests.
Now let me read the new conformance tests (prd_done_* tests) at the end of the validate test file.
Now let me check the helper functions used in validate tests and look for the `make_test_issue_comment` and `json_comment` helpers.
Now let me check how `format_draft_comment` is used in the draft-posting paths to verify the shared constants are actually used consistently.
Good. The shared constants are used in both draft-posting paths (lines 1209 and 1367) and in extraction (lines 2178, 2194). Now let me check the `compose_raw_idea` function and the `gh_bin` configuration in the runtime.
Let me verify how `gh_bin` is wired in the daemon config - specifically in the startup/build config path.
Now let me check the stray file `1740527543-impl-notes.md` at the repo root and the `prd_status_approved_marker` function that's imported in tests.
Good, the tree is clean. Now let me check the stray file `1740527543-impl-notes.md` that appeared in the diff.
This is a stray file committed in the repo root. Let me now check a few more things in parallel: the `clean_draft_body` logic carefully, and the `prd_done_mixed_labels_not_blocked` test to understand if the lifecycle label check in `poll_and_claim` would actually allow mixed labels through.
Let me check the `classify_lifecycle_labels` function to understand the lifecycle label gating in `poll_and_claim`.
This is important. `LIFECYCLE_LABELS` are: `ralph:ready`, `ralph:in-progress`, `ralph:completed`, `ralph:failed`. PRD labels like `ralph:prd-done` and `ralph:prd-approved` are NOT in this set.

So in `poll_and_claim` (runtime.rs:694):
1. `classify_lifecycle_labels(&issue.labels)` only returns lifecycle labels from that set
2. An issue with `ralph:ready` + `ralph:prd-done` + `ralph:prd-approved` will have `lifecycle = ["ralph:ready"]` (only 1 lifecycle label)
3. It passes the check at line 731 (`lifecycle.len() != 1 || lifecycle[0] != "ralph:ready"` → false, so continues)
4. Then `has_in_progress_prd_label` is checked — `ralph:prd-done` takes precedence and returns `false`
5. Issue proceeds to claim and dispatch

This is correct! The PRD labels are not lifecycle labels, so they don't trigger multi-lifecycle normalization. The `has_in_progress_prd_label` check is the only guard, and `ralph:prd-done` precedence correctly allows the issue through.

Now let me verify the build compiles.
The test filter doesn't match like that. Let me find the exact unit test module path.
All 11 new unit tests pass. Let me now also run the full test suite to confirm no regressions.
All 1017 tests pass (773 + 35 + 22 + 38 + 10 + 20 + 12 + 24 + 16 + 4 + 4 + 25 + 11 + 8 + 12 + 3 = 1017), 0 failures, 1 ignored. Now let me verify the logging substrings match exactly what's specified.
The logging messages match the spec:
- Success: `"prd-done: using approved spec"` (contains required substring)
- Fallback: `"approved spec not found, falling back"` (contains required substring)

Now let me check the verbose skip logging requirement (should say "in-progress PRD label" not generic).
Good — the verbose skip log mentions "in-progress PRD label" specifically.

Now let me verify a subtle point: when `clean_draft_body` strips the heading, it checks `first.starts_with(DRAFT_HEADING_PREFIX)`. But after marker lines are removed, the first line would be the heading line. Let me trace through `format_draft_comment` and `clean_draft_body` to verify the round-trip.

`format_draft_comment(revision, spec_body)` produces:
```
## Draft Engineering Specification (Revision {revision})

{spec_body}

*Reply with feedback...*
```

When a draft comment is posted with `prd_marker` prepended:
```
<!-- ralph:prd:42:draft-v5 -->
## Draft Engineering Specification (Revision 5)

{spec_body}

*Reply with feedback...*
```

`clean_draft_body` processing:
1. Filter out lines starting with `<!-- ralph:prd:` → removes marker line
2. Remaining first line: `## Draft Engineering Specification (Revision 5)` — starts with `DRAFT_HEADING_PREFIX` → removed
3. Trailing empty lines popped, then footer `*Reply with feedback...*` → removed
4. Remaining: `\n{spec_body}` after trim → `{spec_body}`

This is correct. The round-trip test at line 4056 confirms this.

Let me also verify one edge case: what happens if the heading line has something after the prefix but doesn't end with `)`. The `starts_with` check is still satisfied, so it's stripped regardless. This seems intentional — it handles any `format_draft_comment` output.

Now let me finalize my review.
# Final Review: AMENDMENTS

## Amendment: STRAY-001

### Problem
A stray file `1740527543-impl-notes.md` was committed to the repository root in commit `2b14acf`. This is an implementation notes artifact from the loop 2 implementation phase and should not be part of the shipped source tree. It is not referenced by any code and appears to be an accidental commit of a workflow artifact.

### Proposed Change
Remove `1740527543-impl-notes.md` from the repository root.

### Affected Files
- `1740527543-impl-notes.md` - delete this stray file

---

## Summary of Review

### What was verified

**Label gating (`interactive_prd.rs:583-606`, `runtime.rs:735-744`)**:
- `IN_PROGRESS_PRD_LABEL_NAMES` correctly lists exactly `ralph:prd`, `ralph:prd-active`, `ralph:prd-approved`, `ralph:prd-failed` — line 583-588
- `has_in_progress_prd_label` correctly short-circuits to `false` when `ralph:prd-done` is present (line 600-601), even when mixed with in-progress labels
- `has_prd_label` is unchanged and still matches all 5 PRD labels including `ralph:prd-done` (line 591-593)
- `poll_and_claim` uses `has_in_progress_prd_label` at line 736, not the old `has_prd_label`
- PRD labels are NOT in `LIFECYCLE_LABELS` (`github.rs:14-19`), so `ralph:prd-done` + `ralph:ready` issues correctly pass the lifecycle check at runtime.rs:731 without triggering multi-lifecycle normalization

**Shared draft format constants (`interactive_prd.rs:165-171`)**:
- `DRAFT_HEADING_PREFIX` and `DRAFT_FOOTER` are used consistently in both draft-posting paths (lines 1209, 1367) via `format_draft_comment()` and in extraction logic (`clean_draft_body` at lines 2178, 2194)
- Round-trip test at line 4056 confirms format/parse consistency

**Approved spec extraction (`interactive_prd.rs:2127-2221`)**:
- `parse_approved_spec_from_comments` correctly filters to bot-authored comments only (line 2133-2136)
- Finds highest approved revision N by scanning `status-approved-vN` markers (lines 2139-2152)
- Uses `.rev().find()` for draft selection to get latest in API order (lines 2158-2161)
- `clean_draft_body` strips markers, heading, footer, trims whitespace, returns `None` on empty (lines 2170-2204)
- `extract_approved_spec` resolves bot login and fetches comments from live API (lines 2211-2221)
- All failure modes (login fail, API fail, no markers, no matching draft, empty body) propagate as `None`

**Dispatch input selection (`runtime.rs:772-809`)**:
- `has_prd_done` flag correctly detected from issue labels (line 774)
- `extract_approved_spec` called via `spawn_blocking_op` (lines 780-789), join failures collapse to `None` via `.unwrap_or(None)` (line 789)
- Success logs `"prd-done: using approved spec"` (line 794), fallback logs `"approved spec not found, falling back"` (line 801) — both contain required substrings
- Non-prd-done issues use `compose_raw_idea` unchanged (line 808)

**Statelessness**: No local `InteractivePrdState` is read for this feature; all spec recovery is from live comments + bot identity

**Unit tests** (11 new tests, all passing):
- Label gating: positive/negative/mixed/precedence tests
- Parsing: highest revision, duplicate draft latest, bot-only filtering, no marker, no matching draft
- Cleanup: marker/heading/footer removal, empty body → `None`
- Round-trip: `format_draft_comment` → `parse_approved_spec_from_comments` consistency

**Validate conformance tests** (6 new tests registered at `validate/mod.rs:121`):
- `prd_done_dispatch_uses_approved_spec`: end-to-end daemon + parser verification
- `prd_done_mixed_labels_not_blocked`: prd-done + prd-approved not blocked
- `prd_done_missing_markers_fallback`: fallback + warning on missing markers
- `prd_done_comments_api_failure_fallback`: fallback when gh fails
- `prd_done_user_spoof_ignored`: user-authored markers ignored
- `prd_done_highest_revision_wins`: multi-revision selection

**Build**: `cargo check` passes, `cargo test` passes 1017 tests with 0 failures

**Concurrency safety**: The `extract_approved_spec` call in `poll_and_claim` runs via `spawn_blocking_op` on a separate blocking task. It uses only its own stack-local state and makes stateless API calls — no shared mutable state concerns.

**No other issues found**: Error paths are handled correctly, the code follows existing patterns, and no regressions are introduced.
