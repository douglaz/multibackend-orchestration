# Implementation Notes

## Decisions Made
- Added `api user` handler returning `ralph-bot\n` to the `pickup_and_question_posting` conformance test's inline GH mock script. This aligns the mock with the runtime contract where `Pending -> AwaitingAnswers` requires bot-login resolution via `gh api user -q .login` for bot-scoped marker idempotency.
- Added `api user` handler to the shared `daemon_mock_gh_script()` in `mock_scripts.rs` to prevent future contract drift when new conformance/integration tests use this shared mock. Placed it before the `label)` case with an error fallthrough for unhandled api subcommands.
- Used issue number 300 for the Pending-stage conformance regression test and 130 for the integration test to avoid collision with existing test issue numbers.
- The Pending-stage bot-login failure test does NOT pre-seed a state file — the daemon creates a fresh `InteractivePrdState` in Pending state on first encounter, matching the real pickup flow.

## Spec Deviations
- None. All acceptance criteria are addressed exactly as specified.

## Testing
- `nix develop -c cargo check` passes cleanly.
- `nix develop -c cargo test --lib` passes (668 tests).
- `nix develop -c cargo test --test daemon_interactive_prd` passes (26 tests, including new `pending_bot_login_failure_exhaustion_transitions_to_failed`).
- The `pickup_and_question_posting` conformance test now handles `gh api user` and should pass in `nix build -L` checkPhase.
- The new `bot_login_failure_exhaustion_pending` conformance test verifies that `error_count` increments across 3 ticks in Pending state, then transitions to `Failed` with `ralph:prd-failed` label.
