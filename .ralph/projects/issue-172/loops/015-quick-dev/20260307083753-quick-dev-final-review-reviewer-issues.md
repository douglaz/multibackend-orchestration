---
artifact: quick-dev-final-review
loop: 15
project: issue-172
backend: codex
role: reviewer
created_at: 2026-03-07T08:37:53Z
---

# Final Review: AMENDMENTS

## Amendment: A1

### Problem
The branch currently fails the exact checks introduced by this feature, so the new pre-commit gate will reject commits in this repo immediately.

- `cargo fmt --check` fails on newly added test code (for example around [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/project/lifecycle.rs):1572 and [`tests/state.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/tests/state.rs):58).
- `cargo clippy --all-targets -- -D warnings` fails on [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/project/lifecycle.rs):1474 (`clippy::too_many_arguments` on `write_loop_artifact`).

### Proposed Change
- Format the new test code with rustfmt.
- Refactor `write_loop_artifact` test helper to reduce argument count (or use a scoped `#[allow(clippy::too_many_arguments)]` if project policy allows).
- Re-run:
  - `cargo fmt --check`
  - `cargo clippy --all-targets -- -D warnings`

### Affected Files
- `src/project/lifecycle.rs` - formatting fixes and clippy-clean test helper signature.
- `tests/state.rs` - formatting fix.

## Amendment: A2

### Problem
`pre_commit_checks::disabled_skips_checks` does not prove what it claims.

- The test setup does not create `Cargo.toml` (see [`src/validate/tests_pre_commit_checks.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs):370 and :391), so cargo checks are skipped regardless of enable/disable flags.
- Therefore the assertions in [`src/validate/tests_pre_commit_checks.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs):94 can pass even if config gating is broken.

### Proposed Change
- Make the disabled-path test run in a Rust project (`Cargo.toml` present) with intentionally failing fmt/clippy inputs.
- Assert that with checks disabled, no pre-commit failure artifact is generated and no pre-commit reloop occurs.
- Keep the enabled-path counterpart to prove the opposite behavior on the same setup.

### Affected Files
- `src/validate/tests_pre_commit_checks.rs` - strengthen assertions so the test actually validates disable-flag behavior.

## Amendment: A3

### Problem
Quick-dev pre-commit failure artifacts are written with iteration `0` on first failure, which can propagate into phase reconstruction as `phase_iteration = 0` in degraded resume scenarios.

- Iteration source is `final_review_attempts` (starts at `0`) in [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/workflow/quick_dev_orchestrator.rs):243 and :798.
- `infer_phase_iteration` accepts parsed `0` from `pending_pre_commit_feedback` path in [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/project/lifecycle.rs):1015 and parse logic at :1063.

### Proposed Change
- Make pre-commit failure iteration 1-based in quick-dev (`final_review_attempts + 1`), and/or
- Clamp parsed pre-commit iteration to minimum `1` in `infer_phase_iteration`.

### Affected Files
- `src/workflow/quick_dev_orchestrator.rs` - 1-based iteration for pre-commit failure artifact.
- `src/project/lifecycle.rs` - defensive clamp for reconstructed pre-commit iteration.

---
