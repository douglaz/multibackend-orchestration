---
artifact: prompt-review
project: issue-194
backend: codex
role: prompt_reviewer
created_at: 2026-03-09T02:50:23Z
---

# Prompt Review

## Issues Found
- The objective says amendments should reach the implementer’s next prompt, but the standard-flow requirements only inject into the planner prompt. That ambiguity can change behavior.
- `priority` is defined as `Option<String>` while also “defaulting to P2,” which is inconsistent and weakly typed; parsing/validation outcomes are unclear.
- The completion guard proposal drains and re-enqueues pending items, which can reorder items, create duplicates, and introduce race conditions.
- Queue naming does not define ID sanitization or collision handling, so invalid filenames and timestamp collisions are possible.
- Concurrent drain behavior is under-specified (rename races and IO error policy), which can produce nondeterministic processing with two orchestrators.
- Final-review unification dedupe is not fully explicit about precedence, risking duplicate or dropped prompt content.
- Several requirements reference source line numbers, which is brittle and hard to maintain.
- Some test suggestions depend on low-level FS operation spying; outcome-based tests are clearer and more feasible.

## Refined Prompt
### Objective
Implement an external amendment queue so any active project can receive amendment requests from CLI and file-based writers at any time, then process them safely at orchestration phase boundaries.

### Non-Negotiable Constraints
1. Preserve current final-review amendment behavior by default.
2. Writers must use atomic handoff: write temp file in queue dir, then rename to final `.json`.
3. Draining must be crash-safe: claim file, parse, then delete.
4. Malformed files must not fail orchestration; quarantine and continue.
5. New CLI behavior must include `src/validate/` conformance tests.

### Data Model (`src/project/amendments.rs`)
Define:

```rust
pub struct AmendmentRequest {
    pub id: String,
    pub body: String,
    #[serde(default)]
    pub priority: AmendmentPriority, // default P2 when omitted
    pub source: AmendmentSource,     // cli | final-review | file
    #[serde(default)]
    pub source_detail: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

Define enums:
- `AmendmentPriority`: `P0 | P1 | P2 | P3` with `Default = P2`
- `AmendmentSource`: `Cli | FinalReview | File`

Rules:
1. `id` and `body` are required and non-empty.
2. `created_at` is RFC3339 UTC.
3. Missing `priority` in JSON deserializes to `P2`.

### Queue Layout and File Contract
1. Queue dir: `<project_dir>/amendment-queue/`
2. Quarantine dir: `<project_dir>/amendment-queue/.quarantine/`
3. Final filename: `<YYYYMMDDHHMMSS>-<sanitized-id>.json`
4. Temp filename: `.tmp-<uuid>.json`
5. If final filename exists, append `-<n>` before `.json`.

`sanitized-id`:
- Keep `[A-Za-z0-9._-]`; replace other chars with `_`.

### Required Functions (`src/project/amendments.rs`)
1. `enqueue_amendment(project_dir: &Path, req: &AmendmentRequest) -> Result<PathBuf>`
2. `drain_amendment_queue(project_dir: &Path) -> Result<Vec<AmendmentRequest>>`
3. `pending_amendment_count(project_dir: &Path) -> Result<usize>`
4. `format_external_amendments_for_prompt(amendments: &[AmendmentRequest]) -> String`

Drain requirements:
1. List `*.json` and `*.inflight`, sorted lexicographically.
2. Rename each `*.json` to `*.inflight` before reading.
3. Parse each `*.inflight`.
4. On parse failure, move to quarantine with unique name and log warning (path + error).
5. Delete `*.inflight` only after successful parse.
6. Missing queue dir returns empty result/count.
7. `NotFound` on rename (race) is skipped; other IO errors return `Err`.

### CLI Command (`src/cli/amend.rs` + `src/cli/mod.rs`)
Add `ralph amend`:

- `--project <id>` optional; defaults to active project
- `--body <text|@path>` required
- `--priority <P0|P1|P2|P3>` optional, default `P2`
- `--id <id>` optional; default `EXT-<YYYYMMDDHHMMSS>`

Behavior:
1. Resolve workspace/project using existing active-project logic.
2. Support `@path` body loading.
3. Validate priority before writing.
4. Build `AmendmentRequest` with `source = cli`.
5. Enqueue via atomic contract.
6. Print final queue filepath.

### Standard Orchestrator Integration (`src/workflow/orchestrator.rs`)
Planning phase:
1. Drain queue at start of `Phase::Planning`.
2. If `amendments.unify_final_review == true`, exclude drained `source == final-review` from external prompt block.
3. Format remaining items via `format_external_amendments_for_prompt`.
4. Pass into `build_planner_prompt` as new `external_amendments: &str`.

Prompt construction:
1. Add template variable `external_amendments`.
2. If template lacks placeholder and content is non-empty, append fallback section `## External Amendments` using `append_section_if_missing`.

Completion guard:
1. Before honoring `PlannerDecision::CompletionRequest`, call `pending_amendment_count`.
2. If count > 0, return `RalphError::Orchestration` with count.
3. Do not drain/re-enqueue in this guard.

### Quick-Dev Integration (`src/workflow/quick_dev_orchestrator.rs`)
In `QuickDevPhase::PlanAndImplement`:
1. After pre-commit feedback injection, drain queue.
2. If non-empty, append `## External Amendments` section to implementer prompt.
3. Reuse shared formatter.

### Final-Review Unification (Opt-In)
Config:
1. Add `amendments.unify_final_review: bool` default `false`.
2. Respect existing global/project merge precedence.

Behavior:
1. Default `false`: no change to existing final-review path.
2. If `true`: accepted final-review amendments are also enqueued as `AmendmentRequest` with:
- `source = final-review`
- `source_detail = reviewer backend`
- `priority = P2` if not explicitly mapped
- `id`/`body` from accepted amendment
3. During planning prompt creation, exclude queued `final-review` items from `external_amendments` text to avoid duplication with existing final-review amendment content.

### Files to Change
1. `src/project/amendments.rs` (new)
2. `src/project/mod.rs`
3. `src/cli/amend.rs` (new)
4. `src/cli/mod.rs`
5. `src/workflow/orchestrator.rs`
6. `src/workflow/quick_dev_orchestrator.rs`
7. `src/config.rs` (or equivalent)

### Acceptance Criteria
- [ ] Typed amendment model + serde defaults are implemented.
- [ ] `ralph amend` enqueues valid JSON via temp-then-rename.
- [ ] Queue supports multiple pending files.
- [ ] Standard orchestrator drains at planning boundary and injects `external_amendments`.
- [ ] Quick-dev drains at plan/implement boundary and injects amendments.
- [ ] Drain supports `.inflight` recovery and crash-safe semantics.
- [ ] Malformed files are quarantined with warnings; orchestration continues.
- [ ] Completion request is blocked while pending amendments exist.
- [ ] `amendments.unify_final_review` default is `false`; opt-in path works with dedupe.
- [ ] Validate conformance tests cover new CLI and orchestration behavior.

### Required Tests
1. Unit tests in `src/project/amendments.rs`:
- enqueue naming and cleanup
- deterministic drain ordering
- post-drain file removal
- `.inflight` recovery
- malformed JSON quarantine
- missing/empty queue behavior
- priority default behavior
- serialization roundtrip
2. CLI tests:
- parse all args
- defaults
- invalid priority rejection
- `@file` body expansion
3. Integration test:
- run `ralph amend`, verify produced file deserializes and drains successfully
4. Conformance tests (`src/validate/tests_amendments.rs`):
- `ralph amend` command behavior
- standard planner prompt injection
- quick-dev prompt injection
- completion guard rejection with pending queue

### Out of Scope
1. Mid-phase real-time interruption
2. Webhook ingestion service
3. Queue UI/dashboard
4. Cross-source semantic dedupe beyond the explicit final-review rule
5. Changes to final-review voting/consensus logic beyond optional enqueue mirroring
