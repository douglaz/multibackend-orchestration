---
artifact: prompt-review
project: summary-run-cargo-clippy-and-resolve-all
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-21T20:51:29Z
---

# Prompt Review

## Issues Found
- The clippy invocation is not fixed (`cargo clippy` vs `--all-targets --all-features`), so “zero warnings” is ambiguous and non-reproducible.
- The prompt assumes exactly 20 warnings without requiring a baseline check; if warnings drift, implementers won’t know whether to follow clippy output or the static list.
- Several fixes are line-number anchored; line numbers are brittle and can fail after nearby edits.
- One deref fix note is internally conflicting (`let _ = log_writer;` vs `let _ = &mut log_writer;`), which can cause compile/borrow errors.
- “No public API changes” is required but not operationalized (what counts, how to verify).
- The testing section does not require a strict failing-on-warning clippy run (`-D warnings`), so warning-free status is not enforced.
- Dead code deletion is specified, but no fallback is defined if those symbols are already removed/renamed.
- Scope boundaries are mostly clear, but there is no required final report format for validating that all warning categories were resolved.

## Refined Prompt
# Clippy Remediation Spec (No Behavior Changes)

## Objective
Resolve all current `cargo clippy` warnings in this repository using mechanical, localized edits only.  
Do not change runtime behavior or public API surface.

## Required Environment
Run commands from repo root using the project toolchain:

1. `nix develop -c cargo clippy --all-targets --all-features`
2. `nix develop -c cargo build`
3. `nix develop -c cargo test`

For final verification, clippy must pass with warnings denied:

4. `nix develop -c cargo clippy --all-targets --all-features -- -D warnings`

## Scope and Constraints
- In scope: warning fixes listed below and any directly required compile fixes caused by those edits.
- Out of scope: refactors, feature work, pedantic lint cleanup, documentation-only updates.
- Dead code items listed below must be deleted, not suppressed.
- `too_many_arguments` warnings must be suppressed only on the listed functions.
- If line numbers drift, identify targets by symbol name and lint message.
- If baseline warning count is no longer 20, still fix all warnings from the required clippy invocation above.

## Required Changes by Category

### 1) Unused import (1)
- `src/backend/mod.rs`
- Remove `use std::os::unix::process::CommandExt`.

### 2) Dead code (3) — delete entirely
- `src/cli/history.rs`: delete function `verdict_label`.
- `src/cli/history.rs`: delete enum `HistoryEntry`.
- `src/cli/history.rs`: delete impl block containing `loop_number`.

### 3) Style lints (5)
- `src/backend/output_normalizer.rs`: replace `let...else { return None; }` on `content.as_array()` with `?`.
- `src/daemon/process.rs`: remove needless `return;`.
- `src/daemon/rebase_agent.rs`: replace single-arm `match` with `if` for `RebaseAgentBackend::None`.
- `src/validate/tests_prd.rs`: remove `-> ()` from `setup_prd_mock`.
- `src/workflow/orchestrator.rs`: collapse `else { if ... }` into `else if ...`.

### 4) `needless_option_as_deref` (3)
- `src/backend/tmux_backend.rs`
- `src/backend/mod.rs` (2 sites)
- Replace no-op `.as_deref_mut()` usages with the minimal equivalent that preserves borrow/mutability semantics and passes clippy.

### 5) Simplifiable `map_or` (4)
- `src/validate/tests_commands.rs`: `map_or(false, ...)` -> `is_some_and(...)`
- `src/validate/tests_run.rs` (2 sites): `map_or(false, ...)` -> `is_some_and(...)`
- `src/validate/tests_streaming.rs`: `map_or(true, ...)` -> `is_none_or(...)`

### 6) `too_many_arguments` suppressions (4)
Add `#[allow(clippy::too_many_arguments)]` directly above:
- `src/daemon/runtime.rs`: `post_artifact_comments_with_client`
- `src/daemon/runtime.rs`: `sweep_artifact_comments`
- `src/daemon/runtime.rs`: `try_post_artifact_comment`
- `src/workflow/orchestrator.rs`: `execute_with_parse_retries`

## Acceptance Criteria
- `nix develop -c cargo clippy --all-targets --all-features -- -D warnings` succeeds.
- Dead code symbols above are removed (not `#[allow(dead_code)]`).
- Only the four listed functions receive `#[allow(clippy::too_many_arguments)]`.
- `nix develop -c cargo build` succeeds.
- `nix develop -c cargo test` passes.
- No public API changes: do not modify signatures/visibility of public items.

## Deliverable Format
Provide:
1. A per-file summary of edits made.
2. Final status of the four required commands (pass/fail).
3. Any deviations from this spec (if none, explicitly state “No deviations”).
