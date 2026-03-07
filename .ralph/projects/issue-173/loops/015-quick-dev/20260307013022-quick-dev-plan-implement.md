---
artifact: quick-dev-plan-implement
loop: 15
project: issue-173
backend: claude
role: implementer
created_at: 2026-03-07T01:30:22Z
---

# Implementation Complete

All changes are implemented across the three files specified:

## Changes Made

### 1. `src/prompts/templates.rs` — `default_final_reviewer_template()`
- Replaced template body with structured review framework
- Preserved the sentinel first line verbatim: `"You are a final reviewer auditing a completed project for correctness, safety, and robustness."`
- Added 7 "WHEN TO FLAG AN ISSUE" qualification rules
- Added "HOW TO WRITE AMENDMENT BODIES" section with priority tags `[P0]`–`[P3]`
- Added "PRIORITY LEVELS" definitions
- Added "ADDITIONAL GUIDELINES" and "YOUR WORKFLOW" sections
- Template uses `{{review_diff_command}}` (pre-computed shell command), `{{base_branch}}`, and `{{merge_base_sha}}`
- Output format (Amendment ID heading, subsections) unchanged

### 2. `src/workflow/orchestrator.rs`
- **`FINAL_REVIEWER_GUARDRAILS`** — replaced with updated constant including priority tag guidance, style rules, and one-amendment-per-issue rule
- **`build_final_reviewer_prompt()`** — added `repo_root: Option<&Path>` parameter; inserts `base_branch`, `merge_base_sha`, and `review_diff_command` into template vars
- **Call site** (line ~3530) — passes `repo_root_ref` as new argument
- **`compute_merge_base_sha()`** — new `pub(super)` helper using `git::commit::merge_base` + `git::branch::remote_ref_exists`, with fallback chain (origin → local → None)
- **`build_review_diff_command()`** — new `pub(super)` helper; falls back to `git diff HEAD -- . ':(exclude).ralph'` when merge-base is empty

### 3. `src/workflow/quick_dev_orchestrator.rs`
- **`build_final_review_prompt()`** — added `repo_root: Option<&Path>` parameter; inserts same three template vars; imports helpers from `orchestrator`
- **Two call sites** (lines ~686, ~729) — pass `repo_root` as new argument
- Renamed `_repo_root` parameter to `repo_root` since it's now used

## Verification
- `cargo check` — passes with no errors
- `cargo test --lib final_review` — all 36 matching tests pass unchanged
- No modifications to `parser.rs` or any test files
