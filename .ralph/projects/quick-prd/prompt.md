# Feature Spec: Add `quick-prd` command — two-backend engineering spec generator

## Overview

Add a `ralph quick-prd` command that generates focused engineering specifications using two backends: claude writes the spec, codex reviews it. This is a lightweight alternative to the full `ralph prd` pipeline for small features.

## CLI Interface

```
ralph quick-prd --idea "add retry logic to backend execute()" \
    [--writer-backend claude] \
    [--reviewer-backend codex] \
    [--max-revisions 2] \
    [--non-interactive] \
    [--dry-run]
```

- `--idea` (required): Feature description (validated non-empty)
- `--writer-backend`: Backend that drafts the spec (default: `claude`)
- `--reviewer-backend`: Backend that reviews the spec (default: `codex`)
- `--max-revisions`: Max write→review cycles, minimum 1 (default: 2). Use a `parse_positive_u32` clap validator.
- `--non-interactive`: Suppress status output (auto-detected from TTY like `prd`)
- `--dry-run`: Print the initial draft prompt only (review/revision prompts depend on prior output)

## Pipeline Flow

Simple loop (NO state machine enum — there's no branching like the full PRD pipeline):

```
Draft (writer) → Review (reviewer) → [Revise (writer) → Review (reviewer)]* → Write SPEC.md
```

1. **Draft** — Writer backend generates engineering spec from the idea
2. **Review** — Reviewer backend validates the spec, returns structured JSON feedback (3-attempt parse with reformat retry, same pattern as `run_llm_gap_analysis` in `src/prd/gaps.rs:112-140`)
3. **Revise** (conditional) — If review found issues, writer incorporates feedback into the **latest** spec (not original draft) and produces updated spec
4. Steps 2-3 repeat up to `max-revisions` times
5. **Finalize** — Write `SPEC.md` to cwd. If max-revisions exhausted without approval, still write last revision with a warning. Cache artifacts under `.ralph/quick-prd/{idea_hash}/`.

Edge cases:
- `approved: false` with empty issues → treat as approved (no feedback to act on)
- Same writer and reviewer backend is allowed (not an error)
- Backend timeout mid-revision → error propagates, cached artifacts preserved

## Prompts

### Writer (Draft) Prompt — `DRAFT_PROMPT`
```
You are a senior software engineer writing a focused engineering specification.

**Feature Idea:**
{{idea}}

**Required Output Format:**
Your response must be a markdown document with the following exact section headings:

## Summary
## Acceptance Criteria
## Technical Approach
## Files & Modules
## Testing Strategy
## Out of Scope

Each section should be concise, specific, and implementation-ready.
```

### Reviewer Prompt — `REVIEW_PROMPT`
```
You are a senior engineer reviewing an engineering specification for completeness and feasibility.

**Feature Idea:**
{{idea}}

**Engineering Spec:**
{{spec}}

**Task:**
Review the spec for: technical feasibility, missing edge cases, completeness of acceptance criteria, testing coverage, and clarity.

**Required Output Format:**
Your response MUST be a single fenced JSON block:

```json
{"approved": true, "issues": []}
```

If issues found:

```json
{"approved": false, "issues": [{"area": "...", "feedback": "..."}]}
```
```

### Revision Prompt — `REVISION_PROMPT`
```
You are a senior software engineer revising an engineering specification based on review feedback.

**Feature Idea:**
{{idea}}

**Current Spec:**
{{spec}}

**Review Issues:**
{{issues}}

**Task:**
Address each review issue and produce an updated specification. You MUST preserve the same 6 required section headings:
## Summary, ## Acceptance Criteria, ## Technical Approach, ## Files & Modules, ## Testing Strategy, ## Out of Scope
```

## Module Structure — nested under `src/prd/`

The quick-prd pipeline is a variant of PRD generation, so it lives under the existing `prd` module.

### New file: `src/prd/quick.rs`

Pipeline + prompts + types in a single file. Contains:

**Types:**
- `QuickPrdOptions` struct: `idea: String`, `writer_spec: String`, `reviewer_spec: String`, `max_revisions: u32`, `dry_run: bool`
- `QuickPrdResult` struct: `spec_path: PathBuf`, `cache_dir: PathBuf`, `revision_count: u32`, `approved: bool`, `summary: String`
- `QuickPrdMeta` struct (Serialize, Deserialize): `idea`, `idea_hash`, `writer_backend`, `reviewer_backend`, `started_at`, `completed_at`, `revision_count`, `approved`, `draft_time_secs: f64`, `review_times_secs: Vec<f64>`, `revision_times_secs: Vec<f64>`
- `ReviewFeedback` struct (Serialize, Deserialize): `approved: bool`, `issues: Vec<ReviewIssue>`
- `ReviewIssue` struct (Serialize, Deserialize): `area: String`, `feedback: String`

**Prompt constants:** `DRAFT_PROMPT`, `REVIEW_PROMPT`, `REVISION_PROMPT` (as defined above)

**Functions:**
- `render_prompt(template: &str, replacements: &[(&str, &str)]) -> String` — simple placeholder replacement (same pattern as `render_template` in `src/prd/stages.rs:373`)
- `check_spec_sections(raw: &str) -> (String, Vec<String>)` — validates 6 required headings (`## Summary`, `## Acceptance Criteria`, `## Technical Approach`, `## Files & Modules`, `## Testing Strategy`, `## Out of Scope`), returns (cleaned_output, missing_sections). Uses `strip_frontmatter()` from `src/workflow/parser.rs`.
- `parse_review_feedback(raw: &str) -> Result<ReviewFeedback>` — uses `extract_fenced_json` (promoted to `pub(crate)` in `src/prd/gaps.rs`) + serde
- `fn format_issues(issues: &[ReviewIssue]) -> String` — formats issues as numbered list for the revision prompt
- `async fn run_review_with_retry(backend: Arc<dyn Backend>, prompt: String) -> Result<ReviewFeedback>` — 3-attempt parse with reformat prompt on failure (mirrors `run_llm_gap_analysis` pattern in `src/prd/gaps.rs`)

**Pipeline:**
- `QuickPrdPipeline` struct with fields: `writer: Arc<dyn Backend>`, `reviewer: Arc<dyn Backend>`, `options: QuickPrdOptions`
- `QuickPrdPipeline::new(writer, reviewer, options) -> Self`
- `QuickPrdPipeline::run(self) -> Result<QuickPrdResult>` — the main loop:
  1. Create cache dir `.ralph/quick-prd/{idea_hash}/` with file locking (same `fs2::FileExt` exclusive lock pattern as `src/prd/cache.rs:50-69`)
  2. Build draft prompt, call writer, time it
  3. Check sections (retry up to 2 times if required sections missing, same as `MAX_SECTION_RETRIES` pattern in `src/prd/pipeline.rs`)
  4. Cache `draft.md`
  5. Loop up to max_revisions times:
     a. Build review prompt, call reviewer with `run_review_with_retry`, time it
     b. Cache `review-{N}.json`
     c. If approved (or `approved: false` with empty issues), break
     d. Build revision prompt with latest spec + formatted issues, call writer, time it
     e. Check sections on revision output
     f. Cache `revision-{N}.md`
     g. Update current spec to revision output
  6. Write `SPEC.md` to cwd
  7. Write `meta.json` with timing data
  8. If not approved after all revisions, print warning
  9. Return `QuickPrdResult`

### New file: `src/cli/quick_prd.rs`

CLI args struct + execute function. Pattern matches `src/cli/prd.rs`:

- `QuickPrdArgs` struct (clap `#[derive(Debug, Args)]`):
  - `#[arg(long)] idea: String`
  - `#[arg(long, default_value = "claude")] writer_backend: String`
  - `#[arg(long, default_value = "codex")] reviewer_backend: String`
  - `#[arg(long, default_value_t = 2, value_parser = parse_positive_u32)] max_revisions: u32`
  - `#[arg(long, conflicts_with = "interactive")] non_interactive: bool`
  - `#[arg(long, conflicts_with = "non_interactive")] interactive: bool`
  - `#[arg(long)] dry_run: bool`

- `pub async fn execute(args: QuickPrdArgs) -> Result<()>`:
  1. Validate idea is non-empty
  2. `Workspace::discover()`
  3. `BackendRegistry::new(&config, BackendRegistryTmuxConfig { enabled: false, session_name: config.workspace.tmux_session.clone(), window_keep_seconds: config.workspace.tmux_window_keep_seconds })`
  4. Resolve writer and reviewer backend specs (default from args, fall back to config default)
  5. Validate both specs via `backend_spec::validate_backend_spec()`
  6. `registry.get_or_create_for_spec()` for writer and reviewer
  7. Health-check both backends
  8. Auto-detect TTY mode (same logic as `src/cli/prd.rs:58-59`)
  9. Print summary: idea, writer backend name, reviewer backend name, mode, max-revisions
  10. Create `QuickPrdOptions` and `QuickPrdPipeline`, call `.run().await`
  11. Print result: spec path, cache dir, revision count, approved/warning

### Modified files

**`src/prd/mod.rs`** — Add `pub mod quick;`

**`src/prd/gaps.rs` line 182** — Change `fn extract_fenced_json` to `pub(crate) fn extract_fenced_json`

**`src/cli/mod.rs`**:
- Add `mod quick_prd;`
- Add `QuickPrd(quick_prd::QuickPrdArgs)` variant to `Commands` enum
- Add dispatch: `Commands::QuickPrd(args) => quick_prd::execute(args).await`

**`src/error.rs`** — Add `QuickPrdFailed(String)` variant with exit code 13

## Acceptance Criteria

1. `src/prd/quick.rs` exists with all types, prompts, and pipeline logic as specified
2. `src/cli/quick_prd.rs` exists with CLI args and execute function
3. `extract_fenced_json` in `src/prd/gaps.rs` is `pub(crate)`
4. `Commands::QuickPrd` registered in `src/cli/mod.rs` with dispatch
5. `QuickPrdFailed` error variant in `src/error.rs` with exit code 13
6. `pub mod quick;` in `src/prd/mod.rs`
7. `cargo check` compiles with zero errors
8. `cargo test` passes all existing + new tests
9. `nix build -L` clean release build succeeds

## Tests Required

Unit tests in `src/prd/quick.rs` `#[cfg(test)] mod tests`:
1. `test_render_prompt` — verify placeholder replacement works
2. `test_check_spec_sections_all_present` — all 6 sections present, empty missing list
3. `test_check_spec_sections_some_missing` — subset of sections, correct missing list
4. `test_check_spec_sections_with_frontmatter` — frontmatter stripped before checking
5. `test_parse_review_feedback_approved` — valid `{"approved": true, "issues": []}` parses correctly
6. `test_parse_review_feedback_rejected` — valid rejected with issues parses correctly
7. `test_parse_review_feedback_malformed` — missing fence returns error
8. `test_review_feedback_serde_roundtrip` — serialize + deserialize preserves data
9. `test_format_issues` — numbered list formatting

CLI parsing tests in `src/cli/mod.rs` `#[cfg(test)] mod tests`:
10. `parses_quick_prd_with_defaults` — verify default writer=claude, reviewer=codex, max_revisions=2
11. `parses_quick_prd_with_all_args` — all flags specified
12. `rejects_quick_prd_with_conflicting_interactive_flags` — --interactive + --non-interactive rejected
