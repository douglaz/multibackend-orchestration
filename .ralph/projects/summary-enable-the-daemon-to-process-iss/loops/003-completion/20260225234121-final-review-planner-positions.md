---
artifact: final-review-planner-positions
loop: 3
project: summary-enable-the-daemon-to-process-iss
backend: claude(opus)
role: planner
created_at: 2026-02-25T23:41:21Z
---

Now I have all the evidence. Let me construct the evaluation.

The key finding for AMND-PRD-001: The posted comment body is `{marker}\n{body_text}` where `body_text` is from `format_draft_comment()` which starts with `DRAFT_HEADING_PREFIX`. So the actual stored comment is:
```
<!-- ralph:prd:N:draft-vN -->
## Draft Engineering Specification (Revision N)
...
```

After marker removal in `clean_draft_body`, the first remaining line is `## Draft Engineering Specification...` — no blank lines between. The scenario described (blank lines before heading) cannot occur from bot-generated comments. It would only occur with malformed external input, which is not a realistic scenario since only bot-authored comments are considered.

# Planner Positions

## Amendment: STRAY-001

### Position
ACCEPT

### Rationale
Verified: the file `1740527543-impl-notes.md` exists at the repository root. It contains internal implementation notes (decisions, spec deviations, testing notes) that are working artifacts, not product code. This file does not belong in the committed source tree and should be deleted.

## Amendment: AMND-PRD-001

### Position
REJECT

### Rationale
The described problem is theoretically possible but cannot occur in practice with bot-generated comments. I verified the full comment construction chain:

1. `post_bot_comment_with_marker_metadata_with_gh_bin` (`src/daemon/github.rs:1649`) constructs: `format!("{marker}\n{body_text}")` — the marker is on line 1, body starts immediately on line 2.
2. `format_draft_comment` (`src/daemon/interactive_prd.rs:169-171`) starts with `DRAFT_HEADING_PREFIX` as the very first character.
3. `clean_draft_body` (`src/daemon/interactive_prd.rs:2171-2174`) filters out marker lines (lines starting with `<!-- ralph:prd:`).

After marker removal, the first remaining line is always the heading — there are no blank lines between marker and heading in bot-generated comments. The extraction function `parse_approved_spec_from_comments` (`src/daemon/interactive_prd.rs:2127-2165`) already filters to bot-authored comments only (`c.author_login == bot_login`), so externally-crafted comment bodies with unusual formatting are excluded by design.

The blank-lines-before-heading scenario would require a code change to `format_draft_comment` or `post_bot_comment_with_marker_metadata_with_gh_bin`, at which point the heading-stripping logic would naturally be revisited. This is not a real bug.

## Amendment: AMND-PRD-002

### Position
ACCEPT

### Rationale
Verified. The conformance tests at `src/validate/tests_interactive_prd.rs:4962-4985` only assert stderr substrings (`"prd-done: using approved spec"`) and verify the pure parser output in a separate call. They do not assert what `--idea` payload was actually dispatched by the daemon binary. The daemon mock ralph script at `src/validate/mock_scripts.rs:967-974` simply exits 0 on `auto` without capturing or recording the `--idea` argument. This means a regression that corrupts the dispatched idea (e.g., failing to pass the cleaned spec, or passing the wrong string) would not be caught by these tests. Capturing the actual dispatched `--idea` argument and asserting its content would provide meaningful end-to-end coverage of the dispatch path. This is a genuine test coverage gap.

## Amendment: AMND-PRD-003

### Position
ACCEPT

### Rationale
Identical to STRAY-001. Verified: `1740527543-impl-notes.md` exists at the repository root and contains internal implementation working notes. It should be removed from the branch.

## Amendment: IPD-SPEC-PARSE-UNWRAP

### Position
REJECT

### Rationale
The code described in the amendment does not exist. The amendment claims there is `caps.get(1).unwrap().as_str().parse::<u32>().unwrap()` at line 210 and references a `STATUS_APPROVED_RE` regex. I verified:

1. No `STATUS_APPROVED_RE` regex exists anywhere in `src/daemon/interactive_prd.rs` (grep returned no matches).
2. No `caps.get(1).unwrap()` exists anywhere in the file (grep returned no matches).
3. The actual `parse_approved_spec_from_comments` function (`src/daemon/interactive_prd.rs:2127-2165`) uses safe parsing throughout: `strip_prefix`/`strip_suffix` with `if let Some(...)` and `parse::<u32>()` with `if let Ok(n)`. There are zero `unwrap()` calls in the parsing logic.
4. Line 210 is actually `state_path` function body, unrelated to spec parsing.

The `unwrap()` calls found in the file are exclusively in test code (assertions, tempfile setup), not in production parsing logic. The amendment describes code that was never written.
