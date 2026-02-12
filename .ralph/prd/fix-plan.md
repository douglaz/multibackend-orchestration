# Plan: Fix stage-skip bug and add section-check retry

## Context

Manual testing of `ralph prd --non-interactive --ask-max 0` revealed two issues after the auto-apply defaults feature was implemented:

1. **Synthesis stage was skipped entirely** — `stage_hashes.json` had {Ideation, Research, Prd} with no Synthesis, and no `03_synthesis.md` was produced.
2. **Deterministic section check is fatal** — the Prd stage LLM returned a summary instead of actual PRD content, and the pipeline immediately aborted (exit 12) with no retry attempt.

---

## Bug 1: MaybeRerun can skip intermediate stages

### Root cause

In `src/prd/pipeline.rs`, `check_gaps_phase` (line ~271) and `apply_answers_phase` (line ~343), the pipeline computes a `rerun_stage` from the minimum `impact_stage` across all gap questions and then transitions to `PrdPhase::MaybeRerun(rerun_stage)`.

```rust
let rerun_stage = min_question_impact_stage(&gap_report.questions).unwrap_or(stage);
return Ok(PrdPhase::MaybeRerun(rerun_stage));
```

If the current stage is Research and all gap questions have `impact_stage: Prd`, then `rerun_stage = Prd`. `MaybeRerun(Prd)` only clears stages >= Prd (i.e. just Prd) and returns `RunStage(Prd)`. This skips Synthesis entirely — it was never run.

Stage ordering: `Ideation < Research < Synthesis < Prd` (derived `Ord`).

### Observed behavior

- `stage_hashes.json`: `{Ideation, Research, Prd}` — no Synthesis
- No `03_synthesis.md` in cache directory
- Prd stage ran without Synthesis context, producing a low-quality summary

### Fix: `check_gaps_phase` auto-apply path

When `rerun_stage > stage`, the current stage's output is fine and the auto-applied answers are already in `self.context.answers`. Downstream stages will pick them up naturally. Just advance to the next stage:

```rust
let rerun_stage = min_question_impact_stage(&gap_report.questions)
    .unwrap_or(stage);
if rerun_stage > stage {
    // Impact is only on later stages; defaults are already in context.
    return Ok(match next_stage(stage) {
        Some(next) => PrdPhase::RunStage(next),
        None => PrdPhase::ValidatePrd,
    });
}
return Ok(PrdPhase::MaybeRerun(rerun_stage));
```

### Fix: `apply_answers_phase`

Same forward-jump bug exists here. `fallback_stage` is the stage whose gap check raised the questions (stored in `self.pending_gap_stage`). Cap the rerun:

```rust
// Current code:
Ok(PrdPhase::MaybeRerun(rerun_stage))

// New code:
let effective_rerun = rerun_stage.min(fallback_stage);
Ok(PrdPhase::MaybeRerun(effective_rerun))
```

### Edge cases

- `rerun_stage == stage`: Normal path, falls through to `MaybeRerun(stage)` as before.
- `rerun_stage < stage`: Normal backward rerun, falls through to `MaybeRerun(rerun_stage)` as before.
- `rerun_stage > stage` when `stage == Prd`: `next_stage(Prd)` returns `None`, so we go to `ValidatePrd`. Correct — Prd is the last stage.
- All questions impact Prd during Ideation gap: `rerun_stage = Prd > Ideation`, advance to Research. Correct — Ideation output is fine, answers in context.

---

## Bug 2: Deterministic section check has no retry

### Root cause

In `check_gaps_phase` (lines 230-235), after `run_stage` stores the output, the deterministic check re-validates required section headers:

```rust
let check = check_stage_output(stage, &stage_output);
if !check.missing_sections.is_empty() {
    let report = format_deterministic_missing_info_report(stage, &check.missing_sections);
    self.cache.write_missing_info_report(&report)?;
    return Err(RalphError::PrdMissingInfo);
}
```

No retry, no fallback. A single malformed LLM response kills the pipeline.

### Observed behavior

- Backend (Claude) returned a meta-description/summary of the PRD (17 lines, 2KB) instead of the actual PRD content (which it wrote to `prd.md` directly)
- All 13 required `## ...` sections were reported as missing
- Pipeline aborted with exit 12

### Existing retry pattern

The codebase already has retry logic for gap analysis JSON parsing in `run_llm_gap_analysis()` (`src/prd/gaps.rs`, lines 126-139): 3 attempts in a tight loop, falling back to `GapReport::default()` on exhaustion.

### Fix: Per-stage retry via state machine re-entry

Add a retry counter field and constant:

```rust
const MAX_SECTION_RETRIES: u8 = 2;

// In PrdPipeline struct:
stage_section_retries: BTreeMap<Stage, u8>,
// Initialized as BTreeMap::new() in PrdPipeline::new()
```

Replace the fatal check with retry logic:

```rust
let check = check_stage_output(stage, &stage_output);
if !check.missing_sections.is_empty() {
    let retries = self.stage_section_retries.entry(stage).or_insert(0);
    if *retries < MAX_SECTION_RETRIES {
        *retries += 1;
        self.interaction.status(&format!(
            "{:?} stage output missing {} required section(s) (retry {}/{})",
            stage, check.missing_sections.len(), retries, MAX_SECTION_RETRIES,
        ));
        // Clear the bad output so run_stage re-executes
        self.context.stage_outputs.remove(&stage);
        self.context.stage_input_hashes.remove(&stage);
        self.skipped_stages.remove(&stage);
        return Ok(PrdPhase::RunStage(stage));
    }
    // Exhausted retries: continue best-effort to LLM gap analysis
    self.interaction.status(&format!(
        "{:?} stage still missing {} section(s) after {} retries; continuing best-effort",
        stage, check.missing_sections.len(), MAX_SECTION_RETRIES,
    ));
}
```

On exhaustion, fall through to LLM gap analysis instead of returning a fatal error. The gap analysis may still catch and flag the issues, and the pipeline can proceed best-effort.

### Edge cases

- Output valid on first attempt: retry counter never touched, no behavior change.
- Output always malformed (3 attempts): continues best-effort after 2 retries, LLM gap analysis runs next.
- `--resume` mode: `stage_section_retries` is ephemeral (in-memory only), retries start fresh on resume. Correct.
- Retry clears `stage_outputs` and `stage_input_hashes`, so `should_skip_stage` returns false even with `--resume`.

---

## Files to modify

| File | Change |
|------|--------|
| `src/prd/pipeline.rs` | Add `stage_section_retries` field + `MAX_SECTION_RETRIES` constant; rewrite section check with retry; cap rerun_stage in `check_gaps_phase` and `apply_answers_phase` |
| `tests/prd.rs` | Add 4 tests (see below) |

### New tests

1. **`forward_impact_stage_advances_without_skipping_stages`** — Non-interactive, Research gap has questions with `impact_stage: Prd`. Verify pipeline runs all 4 stages including Synthesis. Mock: ideation → empty_gap → research → gap_with_prd_impact → synthesis → empty_gap → prd → empty_gap → validation_pass. Assert 9 backend calls, pipeline succeeds.

2. **`apply_answers_caps_rerun_to_current_stage`** — Interactive, Research gap has question with `impact_stage: Prd`. User answers. Verify rerun is from Research (not Prd), Synthesis is not skipped. Assert `rerun_stages` contains Research.

3. **`missing_sections_retried_then_succeeds`** — First ideation output is malformed (no sections), second attempt succeeds. Mock: malformed_ideation → valid_ideation → empty_gap → ... → validation_pass. Assert 10 backend calls (1 extra retry), pipeline succeeds.

4. **`missing_sections_retry_exhaustion_continues_best_effort`** — Ideation always malformed (3 attempts). After exhaustion, pipeline falls through to gap analysis which returns empty (no questions), and continues to next stage. Assert pipeline completes.

---

## Verification

1. `nix develop -c cargo test` — all tests pass
2. `nix build -L` — clean build
3. Manual: `./result/bin/ralph prd --idea "a CLI tool that converts CSV files to JSON" --non-interactive --ask-max 0` — should produce all 4 stage files and a final PRD.md
