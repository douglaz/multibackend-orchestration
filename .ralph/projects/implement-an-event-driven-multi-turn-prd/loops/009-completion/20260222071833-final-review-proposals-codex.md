---
artifact: final-review-proposals
loop: 9
project: implement-an-event-driven-multi-turn-prd
backend: codex
role: final_reviewer
created_at: 2026-02-22T07:18:33Z
---

# Final Review: AMENDMENTS

## Amendment: PRD-REV-001

### Problem
`nix build -L` currently fails in checkPhase because the new integration helper cannot locate the test binary outside a local debug layout. `tests/daemon_interactive_prd.rs:2038` reads runtime `CARGO_BIN_EXE_ralph`; in Nix this is unset, fallback only checks `target/debug/ralph` at `tests/daemon_interactive_prd.rs:2044`, then panics at `tests/daemon_interactive_prd.rs:2051`. This causes the interactive PRD integration tests to fail during package build.

### Proposed Change
Make `ralph_bin_absolute()` robust across cargo/nix layouts:
1. Use compile-time `option_env!("CARGO_BIN_EXE_ralph")` first.
2. Add fallback search for `target/{debug,release}/ralph`, `target/<triple>/{debug,release}/ralph`, and `CARGO_TARGET_DIR` roots (same pattern used in `tests/validate_cli.rs:14`).
3. Keep a clear panic message listing searched locations.

### Affected Files
- `tests/daemon_interactive_prd.rs` - replace `ralph_bin_absolute()` with a layout-agnostic resolver.

## Amendment: PRD-REV-002

### Problem
Section validation is not strict enough to guarantee the required 6-section spec output. `run_draft_with_section_retry_sync` can return drafts with missing sections on final retry (`src/daemon/interactive_prd.rs:1087`), review loops can accept and return those drafts on reviewer approval (`src/daemon/interactive_prd.rs:1056`, `src/daemon/interactive_prd.rs:899`), and revision updates currently accept output when any section exists (`missing.len() < 6`) (`src/daemon/interactive_prd.rs:914`). This can post incomplete specs despite the 6-section requirement.

### Proposed Change
Enforce full section completeness before accepting/posting drafts:
1. Require `missing.is_empty()` for accepted draft/revision content.
2. If sections remain missing after configured retries, return `InteractivePrdFailed` with missing section names (so retry/failure semantics handle it).
3. Add regression coverage for reviewer-approved but section-incomplete outputs to ensure they are rejected.

### Affected Files
- `src/daemon/interactive_prd.rs` - tighten draft/revision acceptance to require all six sections.
- `src/daemon/interactive_prd.rs` - add unit tests for section-incomplete approval cases.
- `src/validate/tests_interactive_prd.rs` - add/adjust conformance coverage enforcing 6-section draft/revision output.
