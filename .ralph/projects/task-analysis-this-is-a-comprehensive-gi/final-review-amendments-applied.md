# Final Review Amendments Applied

## Round 1

### Amendment: A1

### Problem
Two stray implementation artifact files were committed to the repository root and are not project deliverables.

### Proposed Change
Remove the stray files from the repository history tip by deleting them in a follow-up commit:
`git rm 20260222T223018Z-impl-response-III.md IMPL-multi-completer-panel.md`

### Affected Files
`20260222T223018Z-impl-response-III.md`  
`IMPL-multi-completer-panel.md`

### Reviewer
claude

### Amendment: FR-20260222-PR-ALIAS-PRECEDENCE

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

### Reviewer
codex

