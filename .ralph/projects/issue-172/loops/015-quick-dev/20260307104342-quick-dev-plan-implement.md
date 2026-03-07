---
artifact: quick-dev-plan-implement
loop: 15
project: issue-172
backend: claude
role: implementer
created_at: 2026-03-07T10:43:42Z
---

# Implementation: Address Final Review Amendments A1 & A2

## Changes Made

### A1: Add rustfmt/clippy to nix check inputs (`flake.nix`)

The `nix build` check phase was failing because the conformance test `pre_commit_checks::fmt_failure_triggers_reloop` runs `cargo fmt --check`, but `nativeCheckInputs` only included `pkgs.bash`. Added `pkgs.rustfmt` and `pkgs.clippy` to ensure the formatter and linter are available during `nix build` checks.

**File**: `flake.nix:28-31`

### A2: Fix 0-based pre-commit failure artifact iteration

Three changes to enforce the 1-based iteration convention:

1. **`src/workflow/quick_dev_orchestrator.rs:798`** — Changed `iteration: final_review_attempts` to `iteration: final_review_attempts + 1` so the first pre-commit failure artifact is `pre-commit-failure-001.md` instead of `pre-commit-failure-000.md`.

2. **`src/project/lifecycle.rs:1016`** — Added `.map(|i| i.max(1))` clamp after `parse_iteration_from_path` in the `pending_pre_commit_feedback` branch of `infer_phase_iteration`, ensuring even a legacy `pre-commit-failure-000.md` artifact produces iteration >= 1.

3. **`src/project/lifecycle.rs:1728`** — Added regression test `infer_phase_iteration_clamps_zero_based_pre_commit_failure` that constructs a state with `pre-commit-failure-000.md` and verifies the inferred iteration is >= 1.

## Verification

- `cargo build` — compiles cleanly
- `cargo fmt --check` — no formatting issues
- `cargo clippy --all-targets -- -D warnings` — no warnings
- `cargo test` — all tests pass (968+ unit tests, 3 validate CLI tests, full integration suite)
