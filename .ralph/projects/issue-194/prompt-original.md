## Summary

Extend the amendment mechanism so any active project can receive amendment requests from external sources (CLI, file-based queue), independent of the final review phase. Amendments are written as JSON files into a per-project queue directory, batch-drained by both orchestrators at well-defined phase boundaries, and injected into the implementer's next prompt. External writers must follow an atomic write contract (write to temp file, then rename into the queue directory). The drain uses a two-phase protocol — rename to in-flight, process, then delete — to prevent data loss or double-processing on crash. The existing final reviewer `# Final Review: AMENDMENTS` path is left unchanged by default; a configuration flag `amendments.unify_final_review` (default `false`) enables routing accepted final-review amendments through the queue, with source-based deduplication to prevent double-injection.

## Acceptance Criteria

- [ ] A formal `AmendmentRequest` struct is defined in `src/project/amendments.rs` with fields: `id`, `body`, `priority` (optional, defaults to `P2`), `source` (`cli`/`final-review`/`file`), `source_detail` (optional, e.g. reviewer backend name), `created_at`
- [ ] A `ralph amend` CLI command accepts `--project`, `--body`, `--priority`, and writes a timestamped JSON file into `<project_dir>/amendment-queue/` using the atomic temp-file-then-rename write contract
- [ ] Multiple amendment files can accumulate in the queue directory before any are processed
- [ ] The standard orchestrator (`ralph run`) drains the queue in the `Phase::Planning` match arm (before calling `build_planner_prompt`) and appends drained items to the planner prompt via a new `external_amendments` template variable, with `append_section_if_missing` fallback
- [ ] The standard orchestrator rejects `PlannerDecision::CompletionRequest` when the amendment queue is non-empty, analogous to the existing final-review-restart guard at line 686 of `orchestrator.rs`
- [ ] The quick-dev orchestrator (`ralph quick-dev-run`) drains the queue at the top of the `QuickDevPhase::PlanAndImplement` arm (after the `pending_pre_commit_feedback` injection at line 343 of `quick_dev_orchestrator.rs`) and appends drained items to the prompt
- [ ] Drain uses a two-phase protocol: rename each file to `<name>.inflight`, read and parse, then delete; `.inflight` files from a prior interrupted drain are re-processed on the next drain
- [ ] External writers must write to a temp file in the queue directory (e.g. `.tmp-<uuid>.json`) and rename to the final `<timestamp>-<id>.json` name; partial/malformed files are moved to `amendment-queue/.quarantine/` and logged as warnings rather than failing orchestration
- [ ] Existing final-reviewer amendment behaviour is unchanged by default; when `amendments.unify_final_review = true`, accepted amendments from the voting pipeline are also enqueued through `AmendmentRequest` with `source: "final-review"`, and `build_planner_prompt` deduplicates by skipping queue entries with `source == "final-review"` since they are already present in `final-review-amendments-applied.md`
- [ ] Unit tests cover: queue write with atomic contract, two-phase drain (ordering, crash-safety, `.inflight` recovery), malformed file quarantine, prompt injection for both orchestrators, CLI argument parsing, and completion guard rejection
- [ ] An integration test performs an end-to-end `ralph amend` CLI invocation and verifies the resulting queue file is valid and consumable by `drain_amendment_queue`

## Technical Approach

### 1. Amendment queue module — `src/project/amendments.rs` (new file)

Define the queue data model and IO:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmendmentRequest {
    pub id: String,                          // unique, e.g. "EXT-001" or UUID
    pub body: String,                        // markdown: Problem / Proposed Change / Affected Files
    #[serde(default = "default_priority")]
    pub priority: Option<String>,            // "P0"–"P3"; defaults to "P2"
    pub source: String,                      // "cli" | "final-review" | "file"
    #[serde(default)]
    pub source_detail: Option<String>,       // e.g. reviewer backend name for final-review source
    pub created_at: DateTime<Utc>,
}

fn default_priority() -> Option<String> {
    Some("P2".to_owned())
}
```

`priority` is `Option<String>` with a serde default of `"P2"`. This lets final-review amendments omit the field entirely (they carry `source_detail` with the reviewer backend name instead), while CLI-submitted amendments always have an explicit priority.

**Queue directory**: `<project_dir>/amendment-queue/`. Each request is a single JSON file named `<YYYYMMDDHHMMSS>-<id>.json`. This gives natural chronological ordering and avoids locking.

**Atomic write contract**: Writers (CLI, final-review unification) must:
1. Write to a temp file in the queue directory: `.tmp-<uuid>.json`
2. Rename (via `std::fs::rename`) to the final `<YYYYMMDDHHMMSS>-<id>.json` name

This guarantees readers never see partial writes, since `rename` is atomic on POSIX filesystems.

**Functions**:

- `enqueue_amendment(project_dir: &Path, req: &AmendmentRequest) -> Result<PathBuf>` — serialize to a temp file `.tmp-<uuid>.json` in the queue directory, then `rename` to `<YYYYMMDDHHMMSS>-<id>.json`. Creates the queue directory if it doesn't exist.

- `drain_amendment_queue(project_dir: &Path) -> Result<Vec<AmendmentRequest>>` — two-phase drain:
  1. List all `*.json` and `*.inflight` files in the queue directory, sorted by name.
  2. For each `.json` file: rename to `<name>.inflight`. For each `.inflight` file (including those from a prior interrupted drain): proceed directly.
  3. Read and parse each `.inflight` file. On parse failure: move to `amendment-queue/.quarantine/` and log a warning; continue with remaining files.
  4. Delete each successfully parsed `.inflight` file.
  5. Return the batch as `Vec<AmendmentRequest>`.

  If the queue directory doesn't exist or is empty, return `Ok(vec![])`.

  **Crash safety**: If the process crashes between step 2 and step 4, `.inflight` files survive and are re-processed on the next drain. A file is only deleted after successful parsing, so data loss requires a crash during the atomic delete call itself (kernel-level, not application-level). Double-processing is prevented because `.json` files are renamed to `.inflight` before reading — a concurrent drain would not see the original `.json` file.

- `format_external_amendments_for_prompt(amendments: &[AmendmentRequest]) -> String` — render as markdown. Named `format_external_amendments_for_prompt` to avoid collision with the existing `format_amendments_for_prompt` in `orchestrator.rs` (which operates on `FinalReviewAmendment`). Output format:

  ```markdown
  ## External Amendment: <id>

  **Priority**: <priority>
  **Source**: <source>

  <body>
  ```

  Amendments are joined with double newlines, sorted by the order returned from drain (chronological).

### 2. CLI command — `ralph amend`

Add `Amend(AmendArgs)` variant to `Commands` enum in `src/cli/mod.rs`. New file `src/cli/amend.rs`:

```rust
#[derive(Debug, Args)]
pub struct AmendArgs {
    /// Project ID to amend (defaults to the active project in the current workspace)
    #[arg(long)]
    pub project: Option<String>,

    /// Amendment body text, or @<filepath> to read from a file
    #[arg(long)]
    pub body: String,

    /// Priority level: P0 (critical) through P3 (low). Defaults to P2.
    #[arg(long, default_value = "P2")]
    pub priority: String,

    /// Amendment ID. Auto-generated as "EXT-<timestamp>" if omitted.
    #[arg(long)]
    pub id: Option<String>,
}
```

`execute()` resolves the workspace and project directory (same resolution logic as `ralph status`), validates that the priority is one of `P0`–`P3`, expands `@<filepath>` body syntax, constructs an `AmendmentRequest` with `source: "cli"`, calls `enqueue_amendment()`, and prints the resulting file path to stdout. No orchestrator interaction — fire-and-forget.

Validation: if `--priority` is not one of `P0`, `P1`, `P2`, `P3`, exit with a clear error message before writing anything.

### 3. Standard orchestrator integration — `src/workflow/orchestrator.rs`

**3a. Drain at the phase boundary, not inside the prompt builder.**

In the `Phase::Planning` match arm (~line 584), drain the queue *before* calling `build_planner_prompt`. This keeps `build_planner_prompt` side-effect-free (it remains a pure prompt-construction function):

```rust
Phase::Planning => {
    info!(loop = loop_number, "starting planning phase");

    // Drain external amendment queue before building the planner prompt
    let external_amendments = drain_amendment_queue(&project_dir)?;
    let external_amendments_text = if !external_amendments.is_empty() {
        info!(count = external_amendments.len(), "drained external amendments");
        format_external_amendments_for_prompt(&external_amendments)
    } else {
        String::new()
    };

    // ... existing backend assignment ...

    let planner_prompt = build_planner_prompt(
        &effective,
        &state,
        &prompt_content,
        loop_number,
        planner_backend.name(),
        &feature_backends.implementer,
        &project_dir,
        &external_amendments_text,  // new parameter
    )?;
    // ...
}
```

**3b. Update `build_planner_prompt` to accept and inject external amendments.**

Add an `external_amendments: &str` parameter. Insert into the template variables as `"external_amendments"` and append via `append_section_if_missing` (analogous to the existing `final_review_amendments` block):

```rust
vars.insert("external_amendments".to_owned(), external_amendments.to_owned());

// ... after the final_review_amendments append_section_if_missing block ...

if !external_amendments.is_empty() {
    append_section_if_missing(
        &mut prompt,
        &template_source,
        &["external_amendments"],
        "## External Amendments",
        external_amendments,
    );
}
```

This uses a *separate* template variable (`external_amendments`) from the existing `final_review_amendments`, avoiding any collision or dedupe issues when `unify_final_review` is disabled.

**3c. Completion guard for pending external amendments.**

In the `PlannerDecision::CompletionRequest` handler (~line 686), add a guard *before* the existing final-review restart guard:

```rust
PlannerDecision::CompletionRequest { body } => {
    // Guard: reject completion if external amendments are pending.
    // Re-check the queue (it may have received new entries since drain).
    let pending = drain_amendment_queue(&project_dir)?;
    if !pending.is_empty() {
        // Re-enqueue drained items so they're not lost
        for req in &pending {
            enqueue_amendment(&project_dir, req)?;
        }
        return Err(RalphError::Orchestration(
            format!(
                "planner requested completion but {} external amendment(s) are pending",
                pending.len()
            ),
        ));
    }

    // Existing final-review restart guard ...
}
```

This ensures the planner cannot skip externally submitted amendments by requesting completion. The error will trigger a re-plan on the next loop iteration, which will drain and process the amendments.

### 4. Quick-dev orchestrator integration — `src/workflow/quick_dev_orchestrator.rs`

At the top of the `QuickDevPhase::PlanAndImplement` arm (~line 325), after the `pending_pre_commit_feedback` injection (~line 343), drain the queue and append to the prompt:

```rust
// Drain external amendment queue
let external_amendments = drain_amendment_queue(&project_dir)?;
if !external_amendments.is_empty() {
    info!(count = external_amendments.len(), "drained external amendments for quick-dev");
    let formatted = format_external_amendments_for_prompt(&external_amendments);
    prompt.push_str("\n\n## External Amendments\n\
        The following amendment requests were submitted externally. Address each one:\n\n");
    prompt.push_str(&formatted);
}
```

Insert this *after* the `pending_pre_commit_feedback` injection so all feedback sources are appended before backend execution. `project_dir` is already in scope at this point.

### 5. Unify existing final-review path (opt-in, config-gated)

**Configuration**: Add `amendments.unify_final_review: bool` (default `false`) to the project config schema. When `true`, the `append_final_review_amendments_file` function (~line 4675) additionally calls `enqueue_amendment` for each accepted amendment, with:
- `source: "final-review"`
- `source_detail: Some(amendment.reviewer_backend.clone())`
- `priority: None` (will serde-default to `P2`)
- `id` and `body` from the existing `FinalReviewAmendment`

**Deduplication**: When `unify_final_review` is enabled, `build_planner_prompt` would receive both the `final_review_amendments` content (from `final-review-amendments-applied.md`) and `external_amendments` content (from the queue, which now includes final-review entries). To prevent duplication, the drain call in the `Phase::Planning` arm filters out entries with `source == "final-review"` before formatting, since those are already covered by the `final_review_amendments` template variable:

```rust
let external_amendments: Vec<_> = drain_amendment_queue(&project_dir)?
    .into_iter()
    .filter(|a| a.source != "final-review")
    .collect();
```

Final-review entries are still drained (removed from the queue) but excluded from the `external_amendments` prompt text. This means they serve only as a secondary ingestion path and do not duplicate the primary `final-review-amendments-applied.md` content.

When `unify_final_review` is `false` (default), `append_final_review_amendments_file` is unchanged — no enqueue call is added, no dedup filtering is needed.

## Files & Modules

| File | Change |
|---|---|
| `src/project/amendments.rs` | **New** — `AmendmentRequest` struct, `enqueue_amendment`, `drain_amendment_queue`, `format_external_amendments_for_prompt` |
| `src/project/mod.rs` | Add `pub mod amendments;` |
| `src/cli/amend.rs` | **New** — `AmendArgs` struct, `execute()` function with priority validation and `@file` body expansion |
| `src/cli/mod.rs` | Add `mod amend;`, `Amend(amend::AmendArgs)` variant to `Commands`, dispatch in `run()` |
| `src/workflow/orchestrator.rs` | In `Phase::Planning` arm: drain queue before `build_planner_prompt`. Add `external_amendments: &str` param to `build_planner_prompt`, inject as new template variable and `append_section_if_missing` block. In `PlannerDecision::CompletionRequest`: add pending-amendments guard. In `append_final_review_amendments_file`: conditionally enqueue when `unify_final_review` is enabled. |
| `src/workflow/quick_dev_orchestrator.rs` | In `QuickDevPhase::PlanAndImplement` arm: drain queue after pre-commit feedback injection, append to prompt |
| `src/config.rs` (or equivalent config module) | Add `amendments.unify_final_review: bool` field (default `false`) |

## Testing Strategy

**Unit tests in `src/project/amendments.rs`**:
- `enqueue_creates_json_file_with_correct_name` — verify file exists in `amendment-queue/` with `<YYYYMMDDHHMMSS>-<id>.json` naming, and no `.tmp-*` files remain
- `enqueue_uses_atomic_write` — verify that during enqueue, the temp file is created then renamed (mock/spy on fs operations, or verify no `.tmp-*` files exist post-enqueue)
- `drain_returns_amendments_in_chronological_order` — enqueue 3 items with different timestamps, drain, assert order matches timestamp sort
- `drain_removes_files_after_reading` — drain, verify directory contains no `.json` or `.inflight` files
- `drain_recovers_inflight_files` — create a `.inflight` file manually (simulating a prior crash), drain, verify it is processed and deleted
- `drain_quarantines_malformed_files` — write an invalid JSON file to the queue directory, drain, verify it is moved to `.quarantine/` and remaining valid files are still processed
- `drain_on_empty_or_missing_dir_returns_empty_vec` — no panic on missing queue dir
- `format_external_amendments_produces_expected_markdown` — snapshot test of the markdown output, including optional priority rendering
- `enqueue_and_drain_roundtrip` — serialize/deserialize fidelity for all field combinations (with/without priority, with/without source_detail)
- `priority_defaults_to_p2_when_omitted` — deserialize JSON without a `priority` field, verify it defaults to `"P2"`

**CLI parsing tests in `src/cli/mod.rs`** (existing test pattern):
- `parses_amend_with_all_args` — verify all fields parse correctly
- `parses_amend_with_defaults` — verify `--priority` defaults to `P2`, `--id` is `None`
- `amend_rejects_invalid_priority` — verify `--priority P5` is rejected with a clear error

**CLI integration test in `src/cli/amend.rs` or `tests/`**:
- `amend_cli_end_to_end` — set up a minimal workspace and project directory, invoke `ralph amend --project <id> --body "fix bug" --priority P1`, verify the resulting JSON file in `amendment-queue/` deserializes to a valid `AmendmentRequest` with correct field values

**Orchestrator integration tests in `src/validate/`** (new file `tests_amendments.rs`):
- Using the existing mock-backend harness, run a quick-dev orchestration (`quick-dev-run`) with a pre-seeded amendment queue file and verify the implementer prompt contains the amendment text
- Verify the standard orchestrator's planner prompt includes drained amendments in the `## External Amendments` section
- Verify the standard orchestrator's completion guard rejects `CompletionRequest` when the amendment queue is non-empty (items are re-enqueued and error is raised)

## Out of Scope

- GitHub issue comment trigger (webhook-based submission) — requires a webhook server or GitHub Actions integration not present in the current daemon architecture
- Real-time notification to the running orchestrator (amendments are picked up at the next natural phase boundary, not mid-phase)
- Amendment deduplication or conflict resolution across multiple sources (beyond the source-based dedupe for the `unify_final_review` opt-in)
- UI/dashboard for viewing queued amendments
- Modifications to the final-review voting/consensus/arbiter pipeline
- Changes to existing prompt templates (the feature uses a new `external_amendments` template variable and `append_section_if_missing` fallback, leaving existing templates untouched)
- Concurrent-writer locking beyond the atomic rename contract (single-file atomicity is sufficient; directory-level locking is not needed since each amendment is an independent file)