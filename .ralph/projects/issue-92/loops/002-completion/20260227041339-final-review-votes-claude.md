---
artifact: final-review-votes
loop: 2
project: issue-92
backend: claude
role: final_reviewer
created_at: 2026-02-27T04:13:39Z
---

Good, I now have all the context I need to form my votes. Let me verify one more key detail: the Done-path and Failed-path tests (lines 1727 and 1925) use `format!(r#"..."#)` — which means `{{` and `}}` are format escape sequences that produce `{` and `}` at runtime. This is *correct* for those tests. But the tests at lines 2532, 2647, 2761 use `format!("...\n{}", ..., r#"..."#)` where the raw string is the second argument and is inserted verbatim — so `{{`/`}}` in that raw string portion remain as literal `{{`/`}}`, producing invalid JSON. The test at line 2859 uses `r#"..."#.replace(...)` with no `format!`, so `{{`/`}}` also remain literal.

# Vote Results

## Amendment: FR-PRD-001

### Vote
ACCEPT

### Rationale
I verified the defect independently. The four affected tests use two different patterns:

1. **Lines 2532, 2647, 2761**: `format!("#!/bin/sh\nLLOG=\"{}\"\n{}", label_log_str, r#"..."#)` — The second argument is a raw string inserted verbatim via `{}`. The `{{`/`}}` inside the raw string are **not** `format!` escapes — they are literal double braces that get written into the shell script as-is. The generated `printf` commands will emit `[{{"number":...}}]` rather than valid JSON `[{"number":...}]`.

2. **Line 2859**: `r#"..."#.replace("__LABEL_LOG__", ...)` — No `format!` at all, so `{{`/`}}` are again literal double braces in the output script.

This contrasts with the working Done-path (line 1727) and Failed-path (line 1925) tests, where the *entire* script is a `format!(r#"..."#)` string — meaning `{{`/`}}` are correctly interpreted as format escapes producing single braces.

Since `write_mock_gh` writes the body verbatim (line 508) and the daemon parses with strict `serde_json::from_str`, the invalid JSON will cause parse failures, which explains the test failures described in the amendment. The fix is straightforward: change `{{`/`}}` to `{`/`}` in the raw-string portions that are inserted verbatim.

## Amendment: FR-PRD-002

### Vote
ACCEPT

### Rationale
The assertion weakness is real but the practical false-positive risk is low. Here's my analysis:

The production code (github.rs:1068, github.rs:1130) issues `--add-label` and `--remove-label` as separate `gh issue edit` invocations, each logged as a separate line via `echo "$@" >> "$LLOG"`. The current assertions check:

```rust
label_raw.contains("--remove-label") && label_raw.contains("ralph:waiting-feedback")
```

This checks across the *entire* log file, not per-line. If a test scenario involved `--remove-label ralph:prd-active` on one line and `--add-label ralph:waiting-feedback` on another, both substrings would match even though the remove operation never targeted `ralph:waiting-feedback`.

In the current test setup, this false positive is unlikely because the mock flows don't produce that exact combination — but **conformance tests should be precise by design**. They exist to catch regressions, and a sloppy assertion undermines that purpose. The proposed per-line check (`label_raw.lines().any(|l| l.contains("--remove-label ralph:waiting-feedback"))`) is a minimal, targeted improvement that makes the assertion robust against future test changes.
