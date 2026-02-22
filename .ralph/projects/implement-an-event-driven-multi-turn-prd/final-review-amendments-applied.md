# Final Review Amendments Applied

## Round 1

### Amendment: PRD-AMEND-20260222-01

### Problem
Errors while resolving bot identity are not counted toward the PRD workflow retry/failure state machine.

`advance_issue` resolves bot login with `?` before entering transition wrappers (`src/daemon/interactive_prd.rs:375`, `src/daemon/interactive_prd.rs:379`), but retry accounting happens inside `finish_transition`/`apply_transition_result` (`src/daemon/interactive_prd.rs:920`, `src/daemon/interactive_prd.rs:936`, `src/daemon/interactive_prd.rs:944`).  
Result: repeated `gh api user` failures never increment `error_count`, never persist `last_error`, and never transition to `Failed`, violating the retry-exhaustion rule.

### Proposed Change
Route bot-login resolution through transition error handling so failures are wrapped by `finish_transition` and persisted:

1. Move `get_or_fetch_bot_login(...)` into the transition wrapper result path (or compose it into the `result` passed to `finish_transition`).
2. Ensure login-resolution failures increment `error_count` and can trigger `Failed` after 3 consecutive failures.
3. Add regression tests for repeated bot-login failure in `AwaitingAnswers` and `AwaitingFeedback`.

### Affected Files
- `src/daemon/interactive_prd.rs` - include bot-login failures in transition retry accounting.
- `src/validate/tests_interactive_prd.rs` - add conformance case for repeated `gh api user` failure reaching `Failed`.
- `tests/daemon_interactive_prd.rs` - add integration coverage for login-failure retry exhaustion.

### Reviewer
codex

### Amendment: PRD-AMEND-20260222-02

### Problem
Approval label updates can orphan an issue from polling when partial GitHub failure occurs.

`do_approval_transition` removes `ralph:prd-active` before adding `ralph:prd-done` (`src/daemon/interactive_prd.rs:800`, `src/daemon/interactive_prd.rs:807`).  
Polling only scans `ralph:prd` and `ralph:prd-active` (`src/daemon/interactive_prd.rs:322`, `src/daemon/interactive_prd.rs:338`).

If `--add-label ralph:prd-done` fails after `--remove-label ralph:prd-active` succeeds, state stays non-terminal (`AwaitingFeedback` + incremented error) but the issue no longer has a polled label, so it will not be retried. This breaks restart/retry safety.

### Proposed Change
Make approval label mutation boundary-safe:

1. Add `ralph:prd-done` first.
2. Remove `ralph:prd-active` second.
3. Keep retry semantics so partial failures remain pollable (`ralph:prd-active` still present) and can recover on next tick.
4. Add regression tests for partial failure during approval label swap.

### Affected Files
- `src/daemon/interactive_prd.rs` - reorder approval label operations to preserve poll visibility on partial failure.
- `src/validate/tests_interactive_prd.rs` - add conformance test for approval label-swap partial failure recovery.
- `tests/daemon_interactive_prd.rs` - add integration test where add/remove label failure is injected mid-approval.

### Reviewer
codex


## Round 2

### Amendment: PRD-REV-001

### Problem
`nix build -L` currently fails in checkPhase because the new integration helper cannot locate the test binary outside a local debug layout. `tests/daemon_interactive_prd.rs:2038` reads runtime `CARGO_BIN_EXE_ralph`; in Nix this is unset, fallback only checks `target/debug/ralph` at `tests/daemon_interactive_prd.rs:2044`, then panics at `tests/daemon_interactive_prd.rs:2051`. This causes the interactive PRD integration tests to fail during package build.

### Proposed Change
Make `ralph_bin_absolute()` robust across cargo/nix layouts:
1. Use compile-time `option_env!("CARGO_BIN_EXE_ralph")` first.
2. Add fallback search for `target/{debug,release}/ralph`, `target/<triple>/{debug,release}/ralph`, and `CARGO_TARGET_DIR` roots (same pattern used in `tests/validate_cli.rs:14`).
3. Keep a clear panic message listing searched locations.

### Affected Files
- `tests/daemon_interactive_prd.rs` - replace `ralph_bin_absolute()` with a layout-agnostic resolver.

### Reviewer
codex

### Amendment: PRD-REV-002

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

### Reviewer
codex

