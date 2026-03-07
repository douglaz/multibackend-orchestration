---
artifact: quick-dev-final-review
loop: 15
project: issue-172
backend: codex
role: reviewer
created_at: 2026-03-07T10:37:51Z
---

# Final Review: AMENDMENTS

## Amendment: A1

### Problem
`nix build -L` is currently failing in check phase on `pre_commit_checks::fmt_failure_triggers_reloop`. The new conformance path invokes real `cargo fmt --check`, but the build check environment only declares `bash` in check inputs, so formatter/linter tool availability is not guaranteed.

Evidence:
- [`flake.nix`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/flake.nix):28
- [`flake.nix`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/flake.nix):45
- [`tests_pre_commit_checks.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs):203

### Proposed Change
Add formatter/linter tooling to Nix check inputs used by `buildRustPackage` (at least `rustfmt`; ideally `clippy` too). Optionally add a small capability guard in the validate test to skip with a clear reason if `cargo fmt --check` is unavailable.

### Affected Files
- [`flake.nix`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/flake.nix) - add check-time tool dependencies required by new pre-commit conformance behavior.
- [`src/validate/tests_pre_commit_checks.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/validate/tests_pre_commit_checks.rs) - optional guard to avoid environment-dependent hard failure.

## Amendment: A2

### Problem
Quick-dev pre-commit failure artifacts are written with a zero-based counter, so the first artifact is `pre-commit-failure-000.md`. Reconstruction logic can then infer `phase_iteration = 0` from that filename, which breaks the 1-based iteration convention and weakens crash/resume robustness.

Evidence:
- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/workflow/quick_dev_orchestrator.rs):798
- [`quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/workflow/quick_dev_orchestrator.rs):243
- [`lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/project/lifecycle.rs):1015

### Proposed Change
Use a 1-based iteration when writing quick-dev pre-commit failure artifacts (`final_review_attempts + 1`) and clamp reconstructed pending pre-commit iteration to at least `1` in `infer_phase_iteration`.

### Affected Files
- [`src/workflow/quick_dev_orchestrator.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/workflow/quick_dev_orchestrator.rs) - make pre-commit failure artifact iteration 1-based.
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/project/lifecycle.rs) - enforce minimum iteration of 1 during reconstruction.
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-172/src/project/lifecycle.rs) - add a regression test for `pre-commit-failure-000.md` handling.

---
