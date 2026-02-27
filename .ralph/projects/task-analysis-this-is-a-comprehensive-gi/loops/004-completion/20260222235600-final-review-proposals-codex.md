---
artifact: final-review-proposals
loop: 4
project: task-analysis-this-is-a-comprehensive-gi
backend: codex
role: final_reviewer
created_at: 2026-02-22T23:56:00Z
---

# Final Review: AMENDMENTS

## Amendment: FR-20260222-PR-ALIAS-PRECEDENCE

### Problem
`resolve_effective_config` treats global `prompt_review_backends` as "explicit" only when its value differs from defaults (`src/config/mod.rs:182`, `src/config/mod.rs:183`, `src/config/mod.rs:189`).  
That is value-based, not presence-based, and breaks the alias contract:

1. If `prompt_review_backends` is explicitly set to the default value, it is treated as unset and the singular alias path is used (`src/config/mod.rs:192`, `src/config/mod.rs:194`), violating "if `prompt_review_backends` is set, use it."
2. Project singular alias overrides can be ignored when global plural is non-default, because project singular is only consulted in the fallback branch.

Defaults that trigger this ambiguity are defined at `src/config/global.rs:979` and `src/config/global.rs:983`.

### Proposed Change
Use explicit key presence (not value inequality) for global plural alias resolution, and apply precedence as:

1. project `prompt_review_backends` (if set)
2. else project `prompt_review_backend` (if set)
3. else global `prompt_review_backends` (if explicitly set)
4. else synthesize from global `prompt_review_backend`

Add regression tests for:
1. explicit global plural equal-to-default still winning over singular
2. project singular override behavior when global plural is set

### Affected Files
- `src/config/mod.rs` - fix precedence logic and add coverage for alias precedence edge cases.
- `src/config/global.rs` - preserve/propagate explicit presence signal for `workflow.prompt_review_backends` at load time.

## Amendment: FR-20260222-REPO-HYGIENE-ORPHANS

### Problem
Two root-level markdown files appear to be orphaned implementation notes and are not referenced by source/runtime code:

- `20260222T223018Z-impl-response-III.md:1`
- `IMPL-multi-completer-panel.md:1`

Cross-repo search found no references. These should not ship as tracked source files.

### Proposed Change
Remove these files from the branch (or relocate them outside tracked source if they must be retained as internal notes).

### Affected Files
- `20260222T223018Z-impl-response-III.md` - remove orphaned note file.
- `IMPL-multi-completer-panel.md` - remove orphaned note file.
