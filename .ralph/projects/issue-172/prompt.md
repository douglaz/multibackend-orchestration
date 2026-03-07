## Summary

Add a pre-commit checks gate to the orchestrator that runs `cargo fmt --check`, `cargo clippy`, and optionally `nix build` after reviewer approval but before the checkpoint commit that transitions to `Phase::Committing`. When any check fails, its output is fed back to the implementer as a fix iteration instead of proceeding to commit. This prevents CI-failing code (e.g. PR #167) from being committed by daemon worktrees, which do not inherit `core.hooksPath` from the `nix develop` shellHook.

The gate is placed inside the `ReviewerDecision::Approved` arm of the `Phase::Reviewing` match (after the approval artifact is written but before `state.current_phase` is set to `Phase::Committing`). This ensures checks execute before the `Reviewing → Committing` checkpoint commit created by `checkpoint_phase_transition`, which stages all changes, commits, and pushes to the remote branch.

## Acceptance Criteria

- [ ] After reviewer approval and before `state.current_phase` is set to `Phase::Committing`, pre-commit checks run in the worktree
- [ ] `cargo fmt --check` runs; failure routes output back to the implementer (if `pre_commit_fmt_auto_fix` is enabled, `cargo fmt` is attempted first — only a failure of the auto-fix itself is reported)
- [ ] `cargo clippy --all-targets -- -D warnings` runs; failure routes output back to the implementer
- [ ] `nix build` is supported but **disabled** by default; toggleable via `pre_commit_nix_build` in both global `[workflow]` and per-project `[workflow]` config
- [ ] Per-project config keys `pre_commit_fmt`, `pre_commit_clippy`, `pre_commit_nix_build`, `pre_commit_fmt_auto_fix` (all `Option<bool>`) exist in `ProjectWorkflowOverrides`
- [ ] Global config keys `pre_commit_fmt` (default `true`), `pre_commit_clippy` (default `true`), `pre_commit_nix_build` (default `false`), `pre_commit_fmt_auto_fix` (default `false`) exist in `WorkflowConfig`
- [ ] All four keys are wired through `EffectiveWorkflowConfig`, `set_global_config_value`, `set_project_value`, and the `config show`/`config get` display builders
- [ ] Failed check output is injected into the implementer prompt as fix feedback (via the `pending_pre_commit_feedback` field on `FeatureLoopArtifacts`, following the `pending_qa_feedback` pattern) and the phase returns to `Phase::Implementing`
- [ ] The approval artifact is cleared from `loop_state.artifacts.approval` on pre-commit failure so the fix is re-reviewed before another commit attempt
- [ ] Checks do **not** run if the reviewer rejected (existing `Phase::Implementing` feedback path is unaffected)
- [ ] Checks run at most once per approval (re-entering `Phase::Reviewing` after a fix iteration and receiving a new approval re-runs them)
- [ ] Tool errors (spawn failure, timeout) are caught by the check runner and converted to `passed: false` feedback — they never abort orchestration or create unfixable loops
- [ ] Cargo checks are skipped (treated as passed) when no `Cargo.toml` exists in the worktree root, making the feature safe for non-Rust projects and existing tests
- [ ] `infer_phase_iteration` and `reconstruct_project_state_from_project_dir` correctly handle `pending_pre_commit_feedback` for crash/resume safety
- [ ] The quick-dev orchestrator runs pre-commit checks after both final reviewers return Complete, before the `FinalReview → Completing` checkpoint; failure follows the existing issues-found reloop path
- [ ] Existing tests pass; new validate conformance tests in `src/validate/tests_pre_commit_checks.rs` cover the feedback-loop path for a failing check
- [ ] New unit tests in `src/workflow/pre_commit_checks.rs` cover skip-when-no-Cargo.toml, all-disabled, and error-to-feedback conversion

## Technical Approach

### 1. Configuration — add pre-commit check flags

Add four new fields to `WorkflowConfig` (`src/config/global.rs`, after the existing `session_reuse_reset_on_rollback` field at line 395):

```rust
#[serde(default = "default_pre_commit_fmt")]
pub pre_commit_fmt: bool,                  // default: true
#[serde(default = "default_pre_commit_clippy")]
pub pre_commit_clippy: bool,               // default: true
#[serde(default)]
pub pre_commit_nix_build: bool,            // default: false
#[serde(default)]
pub pre_commit_fmt_auto_fix: bool,         // default: false
```

Add corresponding default helper functions (`fn default_pre_commit_fmt() -> bool { true }`, etc.) following the existing pattern used by `default_auto_commit`, `default_pre_commit_clippy`, etc.

Mirror them as `Option<bool>` in `ProjectWorkflowOverrides` (`src/config/project.rs`, after the existing `session_reuse_reset_on_rollback` field at line 61):

```rust
pub pre_commit_fmt: Option<bool>,
pub pre_commit_clippy: Option<bool>,
pub pre_commit_nix_build: Option<bool>,
pub pre_commit_fmt_auto_fix: Option<bool>,
```

Add resolved fields to `EffectiveWorkflowConfig` (`src/config/mod.rs`, after `session_reuse_reset_on_rollback` at ~line 71):

```rust
pub pre_commit_fmt: bool,
pub pre_commit_clippy: bool,
pub pre_commit_nix_build: bool,
pub pre_commit_fmt_auto_fix: bool,
```

Wire them through `resolve_effective_config` (~line 132) using the standard project-overrides-global fallback pattern:

```rust
pre_commit_fmt: project_ref
    .and_then(|p| p.workflow.pre_commit_fmt)
    .unwrap_or(global.workflow.pre_commit_fmt),
// ... same for clippy, nix_build, fmt_auto_fix
```

#### CLI config surface

In `set_global_config_value` (`src/config/global.rs`, the match block starting ~line 1322), add four new arms:

```rust
"workflow.pre_commit_fmt" => { ... parse_bool ... config.workflow.pre_commit_fmt = val; }
"workflow.pre_commit_clippy" => { ... }
"workflow.pre_commit_nix_build" => { ... }
"workflow.pre_commit_fmt_auto_fix" => { ... }
```

In `set_project_value` (`src/cli/config.rs`, the match block starting ~line 391), add four new arms following the `workflow.auto_commit` pattern.

In the `config show` JSON builder (`src/cli/config.rs`, ~lines 122–195), add the four keys to the workflow object. In the `config get` builder (~lines 211–302), add them to the same JSON object so `ralph config get workflow.pre_commit_fmt` works.

### 2. Pre-commit check runner — new module `src/workflow/pre_commit_checks.rs`

Create a single public function and result struct:

```rust
pub struct PreCommitCheckResult {
    pub passed: bool,
    pub feedback: String,  // combined stderr/stdout of failing checks; empty if passed
}

pub fn run_pre_commit_checks(
    repo_root: &Path,
    fmt_enabled: bool,
    clippy_enabled: bool,
    nix_build_enabled: bool,
    fmt_auto_fix: bool,
) -> PreCommitCheckResult
```

Note: this function returns `PreCommitCheckResult` directly — it **never** returns `Err`. All internal errors are captured as feedback.

Implementation details:

1. **Cargo.toml guard**: Before running any cargo command, check `repo_root.join("Cargo.toml").exists()`. If absent, skip all cargo checks (return `PreCommitCheckResult { passed: true, feedback: String::new() }` for the cargo portion). This prevents failures in non-Rust projects and in test worktrees that lack a Cargo.toml.

2. **For each enabled check**, use `std::process::Command` with `current_dir(repo_root)`, `stdout(Piped)`, `stderr(Piped)`, and pass it to `run_command_with_timeout` from `src/daemon/process.rs:409`:
   - `cargo fmt --check` — timeout 120s
   - `cargo clippy --all-targets -- -D warnings` — timeout 300s
   - `nix build` — timeout 600s

3. **Error conversion**: Every call to `run_command_with_timeout` is wrapped in a match. `Err(e)` (timeout, spawn failure, poll error) is converted to a failure entry in the feedback string: `"## {check_name}\nError: {e}\n\n"`. This ensures tool-level failures produce deterministic implementer feedback rather than aborting orchestration.

4. **`cargo fmt` auto-fix fallback**: If `fmt_enabled && fmt_auto_fix` and `cargo fmt --check` fails (non-zero exit or `Err`):
   - Run `cargo fmt` (no `--check`) with the same timeout.
   - If `cargo fmt` succeeds (exit 0), treat the fmt check as passed (the formatting was auto-fixed in the worktree).
   - If `cargo fmt` itself fails or errors, include the error output in feedback as a failure.

5. **No short-circuit**: If `cargo fmt --check` fails, still run `cargo clippy` so the implementer receives all failures in one feedback round.

6. **Feedback aggregation**: Failures are aggregated into a single string with clear section headers:
   ```
   ## cargo fmt --check
   <stdout + stderr>

   ## cargo clippy
   <stdout + stderr>
   ```

7. **`passed`**: `true` only if every enabled check either succeeded or was skipped (no Cargo.toml / check disabled).

### 3. Orchestrator integration — inject checks inside the `ReviewerDecision::Approved` arm

The gate is placed in `src/workflow/orchestrator.rs` inside the `ReviewerDecision::Approved` match arm (~line 1741), **after** the approval artifact is written (~line 1758) and **before** `state.current_phase = Phase::Committing` (~line 1773). This ensures checks execute before the `checkpoint_phase_transition` call at ~line 2421 that creates the `Reviewing → Committing` commit and push.

Pseudocode for the insertion point (between current lines 1770 and 1773):

```rust
// --- begin pre-commit check gate ---
let any_check_enabled = effective.workflow.pre_commit_fmt
    || effective.workflow.pre_commit_clippy
    || effective.workflow.pre_commit_nix_build;

let pre_commit_passed = if any_check_enabled {
    let repo_root = self.workspace.root.parent().ok_or_else(|| {
        RalphError::Orchestration("workspace root has no parent".to_owned())
    })?;
    let result = run_pre_commit_checks(
        repo_root,
        effective.workflow.pre_commit_fmt,
        effective.workflow.pre_commit_clippy,
        effective.workflow.pre_commit_nix_build,
        effective.workflow.pre_commit_fmt_auto_fix,
    );
    if !result.passed {
        // Write failure artifact
        let iteration = review_count;
        let failure_path = write_artifact(
            &project_dir,
            ArtifactWriteInput {
                project_id: &state.project_id,
                loop_number,
                loop_slug: &loop_slug,
                backend: "pre-commit-checks",
                role: "pre-commit",
                kind: ArtifactKind::PreCommitCheckFailure { iteration },
                body: &result.feedback,
            },
        )?;
        let failure_rel = artifact_relative_path(&project_dir, &failure_path);

        // Set pending feedback for Implementing phase pickup
        {
            let loop_state = state.current_feature_loop_mut().ok_or_else(|| {
                RalphError::Orchestration(
                    "failed to reload loop after pre-commit check failure".to_owned(),
                )
            })?;
            loop_state.artifacts.pending_pre_commit_feedback = Some(failure_rel);
            loop_state.artifacts.approval = None; // force re-review after fix
        }

        state.current_phase = Phase::Implementing;
        state.phase_iteration = iteration;
        logs.push(format!(
            "loop {loop_number}: pre-commit checks failed, routing back to implementer"
        ));
        false
    } else {
        true
    }
} else {
    true
};

if !pre_commit_passed {
    // Skip the Committing transition; the phase is already set to Implementing.
    // The outer loop will detect the Reviewing → Implementing transition
    // and create the corresponding checkpoint commit.
} else {
    // Original approval flow continues here
    state.current_phase = Phase::Committing;
    state.phase_iteration = 1;
    logs.push(format!("loop {loop_number}: reviewer approved changes"));
    // ... rest of existing approval logic (until_review, commit_message, etc.)
}
// --- end pre-commit check gate ---
```

The existing code at lines 1773–1787 (setting `Phase::Committing`, logging approval, handling `until_review`, logging commit message) moves inside the `else` branch.

When checks fail, the phase transition detected at line 2409 will be `Reviewing → Implementing` — the same transition as reviewer rejection. The checkpoint commit captures the current worktree state (including the pre-commit failure artifact) and pushes it, following the normal checkpoint pattern.

### 4. Implementer feedback injection — `pending_pre_commit_feedback` path

Add a new field to `FeatureLoopArtifacts` (`src/project/state.rs`, after `pending_qa_feedback` at line 168):

```rust
#[serde(default)]
pub pending_pre_commit_feedback: Option<String>,
```

In the `Phase::Implementing` arm of the orchestrator (`src/workflow/orchestrator.rs`), add a new branch **between** the `pending_qa_feedback` check (~line 928) and the review feedback `else` branch (~line 1094):

```rust
} else if let Some(pre_commit_feedback_path) = {
    let loop_state = state.current_feature_loop().ok_or_else(|| { ... })?;
    loop_state.artifacts.pending_pre_commit_feedback.clone()
} {
    // Handle pre-commit failure feedback
    let feedback_content = read_project_relative_file(&project_dir, &pre_commit_feedback_path)?;
    let labeled_feedback = format!(
        "## Pre-Commit Check Failures\n\
         The following automated checks failed after reviewer approval. \
         Fix these issues without changing unrelated logic:\n\n{}",
        feedback_content
    );

    // Session setup (same pattern as review feedback path)
    // ...

    let impl_prompt = build_implementer_prompt(
        &effective, &state, &prompt_content, &feature_name, &loop_slug,
        implementer_backend.name(), &planner_backend, &spec_content, &git_diff,
        Some(iteration),
        Some(&labeled_feedback),  // reuse existing review_feedback parameter
        &project_dir,
        session_id.is_some(),
    )?;

    // Execute implementer backend (same pattern as review feedback path)
    // ...

    // Write response artifact
    write_artifact(&project_dir, ArtifactWriteInput {
        // ...
        kind: ArtifactKind::ImplPreCommitResponse { iteration: parsed_iteration },
        body: &body,
    })?;

    // Clear pending feedback
    {
        let loop_state = state.current_feature_loop_mut().ok_or_else(|| { ... })?;
        loop_state.artifacts.pending_pre_commit_feedback = None;
    }

    stage_changes_for_review(&self.workspace.root)?;
    state.current_phase = Phase::Reviewing;
    state.phase_iteration += 1;
    logs.push(format!(
        "loop {loop_number}: implementer responded to pre-commit check failure iteration {parsed_iteration}"
    ));
}
```

This reuses the existing `review_feedback` parameter of `build_implementer_prompt` rather than adding a new parameter. The labeled header makes the feedback type clear to the implementer. No changes to `build_implementer_prompt`'s signature are required.

### 5. Artifact kinds

Add two new variants to `ArtifactKind` in `src/project/artifacts.rs` (~line 51):

```rust
PreCommitCheckFailure { iteration: u32 },
ImplPreCommitResponse { iteration: u32 },
```

With corresponding filename generation in `file_name()`:

```rust
ArtifactKind::PreCommitCheckFailure { iteration } =>
    format!("pre-commit-failure-{iteration:03}.md"),
ArtifactKind::ImplPreCommitResponse { iteration } =>
    format!("impl-pre-commit-response-{iteration:03}.md"),
```

And `base_type()` entries:

```rust
ArtifactKind::PreCommitCheckFailure { .. } => "pre-commit-failure",
ArtifactKind::ImplPreCommitResponse { .. } => "impl-pre-commit-response",
```

### 6. Resume / state reconstruction

In `src/project/lifecycle.rs`, within the loop-artifact reconstruction logic (~line 691, near where `pending_qa_feedback` is derived):

```rust
let pending_pre_commit_feedback = {
    // Find pre-commit-failure-NNN.md artifacts
    let failures: Vec<_> = artifacts.iter()
        .filter(|a| a.base_type == "pre-commit-failure")
        .collect();
    // Find impl-pre-commit-response-NNN.md artifacts
    let responses: Vec<_> = artifacts.iter()
        .filter(|a| a.base_type == "impl-pre-commit-response")
        .collect();
    // A failure is pending if there's no matching response
    failures.iter().rev()
        .find(|f| {
            let iter = parse_iteration_from_filename(&f.file_name, "pre-commit-failure-");
            iter.map_or(false, |i| {
                !responses.iter().any(|r| {
                    parse_iteration_from_filename(&r.file_name, "impl-pre-commit-response-")
                        == Some(i)
                })
            })
        })
        .map(|f| f.rel_path.clone())
};
```

Wire `pending_pre_commit_feedback` into the `FeatureLoopArtifacts` struct construction (~line 766).

In `infer_phase_iteration` (`src/project/lifecycle.rs:952`), add a check for `pending_pre_commit_feedback` in the `Phase::Implementing` arm, between the `pending_qa_feedback` check and the review-count fallback:

```rust
Phase::Implementing => {
    if let Some(pending) = &feature_loop.artifacts.pending_qa_feedback {
        // ... existing QA logic ...
    }
    // NEW: check for pending pre-commit feedback
    if feature_loop.artifacts.pending_pre_commit_feedback.is_some() {
        return feature_loop
            .artifacts
            .reviews
            .last()
            .map(|review| review.iteration + 1)
            .unwrap_or(1);
    }
    // ... existing review-count fallback ...
}
```

This ensures that after a crash/restart during a pre-commit feedback iteration, the orchestrator reconstructs the correct `phase_iteration` and finds the pending feedback artifact.

### 7. Quick-dev orchestrator integration

In `src/workflow/quick_dev_orchestrator.rs`, the pre-commit check gate is inserted at **one specific location**: after both final reviewers return `Complete` (~line 772) and **before** the `persist_destination_and_checkpoint()` call that transitions `FinalReview → Completing` (~line 782).

```rust
// Both reviewers said Complete
// --- begin pre-commit check gate ---
let any_check_enabled = effective.workflow.pre_commit_fmt
    || effective.workflow.pre_commit_clippy
    || effective.workflow.pre_commit_nix_build;

if any_check_enabled {
    let result = run_pre_commit_checks(
        repo_root,
        effective.workflow.pre_commit_fmt,
        effective.workflow.pre_commit_clippy,
        effective.workflow.pre_commit_nix_build,
        effective.workflow.pre_commit_fmt_auto_fix,
    );
    if !result.passed {
        // Write failure artifact
        write_artifact(&project_dir, ArtifactWriteInput {
            kind: ArtifactKind::PreCommitCheckFailure { iteration: final_review_attempts },
            body: &result.feedback,
            // ...
        })?;

        // Follow the existing issues-found reloop path
        final_review_attempts += 1;
        save_state_to_disk(...)?;

        if final_review_attempts >= max_final_review_retries {
            // Force-complete (same as existing max-retries guard)
            // ...
        } else {
            // Reloop to PlanAndImplement (same as existing issues-found path)
            logs.push("pre-commit checks failed, re-entering PlanAndImplement");
            persist_destination_and_checkpoint(
                // dest_phase: QuickDevPhase::PlanAndImplement
                // from_phase: Phase::FinalReview
                // to_phase: Phase::Implementing
                // ...
            )?;
            current_qd_phase = QuickDevPhase::PlanAndImplement;
            continue;
        }
    }
}
// --- end pre-commit check gate ---
// proceed with existing Completing transition
```

The failure artifact is persisted before the reloop checkpoint, so `reconstruct_project_state_from_project_dir` can recover the pre-commit failure context on resume. The `final_review_attempts` counter is incremented to prevent infinite pre-commit failure loops, using the same max-retries guard as the existing issues-found path.

Checks do **not** run at the `CodexReview → FinalReview` transition or any other checkpoint site — only at the final exit point before completion. This avoids redundant check runs during intermediate phase transitions.

### 8. Error handling policy

The `run_pre_commit_checks` function never propagates errors to the caller. All failure modes are handled internally:

| Failure mode | Handling |
|---|---|
| `run_command_with_timeout` returns `Err` (timeout, spawn failure, poll error) | Captured as `passed: false` with `"Error: {err}"` in the feedback section for that check |
| Command exits non-zero | Captured as `passed: false` with stdout+stderr in feedback |
| `Cargo.toml` absent | Cargo checks skipped entirely (treated as passed) |
| All checks disabled | Returns `PreCommitCheckResult { passed: true, feedback: String::new() }` immediately |
| `cargo fmt` auto-fix succeeds after `--check` fails | Treated as passed for the fmt check; clippy still runs independently |

This guarantees that tool-level failures always produce implementer feedback and never abort orchestration or create unfixable loops.

## Files & Modules

| File | Change |
|---|---|
| `src/config/global.rs` | Add `pre_commit_fmt`, `pre_commit_clippy`, `pre_commit_nix_build`, `pre_commit_fmt_auto_fix` to `WorkflowConfig` with defaults; add match arms in `set_global_config_value` |
| `src/config/project.rs` | Add the four keys as `Option<bool>` to `ProjectWorkflowOverrides` |
| `src/config/mod.rs` | Add fields to `EffectiveWorkflowConfig`; wire through `resolve_effective_config` |
| `src/cli/config.rs` | Add match arms in `set_project_value`; add keys to `config show` and `config get` JSON builders |
| `src/workflow/pre_commit_checks.rs` | **New file** — `PreCommitCheckResult` struct and `run_pre_commit_checks()` function with unit tests |
| `src/workflow/mod.rs` | Add `pub mod pre_commit_checks;` |
| `src/workflow/orchestrator.rs` | Insert pre-commit check gate inside `ReviewerDecision::Approved` arm before `Phase::Committing` transition; add `pending_pre_commit_feedback` handling branch in `Phase::Implementing` arm |
| `src/workflow/quick_dev_orchestrator.rs` | Insert pre-commit check gate after both final reviewers return Complete, before `FinalReview → Completing` checkpoint |
| `src/project/artifacts.rs` | Add `ArtifactKind::PreCommitCheckFailure` and `ArtifactKind::ImplPreCommitResponse` variants |
| `src/project/state.rs` | Add `pending_pre_commit_feedback: Option<String>` to `FeatureLoopArtifacts` |
| `src/project/lifecycle.rs` | Scan for `pre-commit-failure-*` / `impl-pre-commit-response-*` artifacts during reconstruction; populate `pending_pre_commit_feedback`; update `infer_phase_iteration` for Implementing with pending pre-commit feedback |
| `src/validate/tests_pre_commit_checks.rs` | **New file** — validate conformance tests for pre-commit check feedback loop |
| `src/validate/mod.rs` | Register `tests_pre_commit_checks::tests()` in `register_tests()` |

## Testing Strategy

### 1. Unit tests in `src/workflow/pre_commit_checks.rs`

- **All checks disabled**: `run_pre_commit_checks(path, false, false, false, false)` returns `passed: true`, empty feedback.
- **No `Cargo.toml`**: With fmt/clippy enabled but no `Cargo.toml` at `repo_root`, returns `passed: true` (cargo checks skipped). Nix build still runs if enabled.
- **Error-to-feedback conversion**: Mock a command path that doesn't exist (e.g. `/nonexistent/cargo`). Verify the function returns `passed: false` with an error message in feedback, not `Err`.
- **Timeout-to-feedback conversion**: Use a script that sleeps longer than the timeout. Verify `passed: false` with timeout message.
- **Feedback aggregation**: When multiple checks fail, verify all failures appear in the feedback string with section headers.

### 2. Validate conformance tests in `src/validate/tests_pre_commit_checks.rs`

Register in `src/validate/mod.rs` as `tests.extend(tests_pre_commit_checks::tests())`.

Tests use the existing `RalphHarness` pattern:

- **pre-commit failure feedback loop**: Configure a mock backend script that produces code failing `cargo fmt --check` (e.g. write a misformatted `.rs` file to the worktree). Run orchestration. Assert:
  - After reviewer approval, the orchestrator routes back to `Phase::Implementing` (not `Phase::Committing`).
  - A `pre-commit-failure-*.md` artifact is written under the loop directory.
  - The implementer is re-invoked with pre-commit failure content in its prompt.
  - The approval artifact is cleared (re-review is required after fix).
- **pre-commit passes**: Configure a mock backend script that produces correctly formatted code. Assert the normal `Reviewing → Committing` flow proceeds without routing back.
- **checks disabled**: Set `pre_commit_fmt = false`, `pre_commit_clippy = false` in project config. Assert no check commands are spawned and approval proceeds directly to Committing.

### 3. Config resolution tests

Add to existing config test locations:

- Project-level `pre_commit_fmt = false` overrides global `pre_commit_fmt = true`.
- `pre_commit_nix_build` defaults to `false` when absent from both global and project config.
- `pre_commit_fmt` and `pre_commit_clippy` default to `true`.
- `pre_commit_fmt_auto_fix` defaults to `false`.
- `ralph config get workflow.pre_commit_fmt` returns the resolved value.
- `ralph config set workflow.pre_commit_nix_build true` round-trips correctly.

### 4. Existing test suite compatibility

Existing orchestrator and quick-dev tests use mock backends that do not produce real Rust source files. With the `Cargo.toml` guard in `run_pre_commit_checks`, cargo checks are skipped when no `Cargo.toml` exists, so **existing tests pass without modification**. No test config changes are required.

### 5. Resume/reconstruction tests

- Write a `pre-commit-failure-002.md` artifact without a corresponding `impl-pre-commit-response-002.md`. Run `reconstruct_project_state_from_project_dir`. Assert `pending_pre_commit_feedback` is `Some(...)` and `infer_phase_iteration` returns the correct iteration.
- Write both `pre-commit-failure-002.md` and `impl-pre-commit-response-002.md`. Assert `pending_pre_commit_feedback` is `None` (the failure was already responded to).

## Out of Scope

- Configuring `core.hooksPath` in daemon worktrees
- Adding a dedicated "lint" orchestration phase (checks run inline within the existing reviewing/approval flow)
- Running checks as a post-commit gate
- Making the check commands themselves configurable (hardcoded to `cargo fmt`, `cargo clippy`, `nix build` for now)
- Extending this to the PRD pipeline or non-Rust projects
- Running pre-commit checks at the quick-dev `CodexReview → FinalReview` transition (only the final `FinalReview → Completing` exit is gated)