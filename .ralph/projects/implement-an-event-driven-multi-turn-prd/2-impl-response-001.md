# Implementation Response (Iteration 1)

## Changes Made
1. **Fixed PRD state persistence path (Required #1)** — Changed `run_prd_phase` in `src/daemon/runtime.rs:596` to derive `data_dir` by walking two parents up from `repo_root` (i.e., stripping the `{owner}/{repo}` suffix). Previously `repo_root` was passed directly, causing `state_path()` in `interactive_prd.rs:177` to double the `{owner}/{repo}` segment. Now state is written to `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue}.json` exactly once.

2. **Fixed retry semantics and per-tick deduplication (Required #2)** — Two changes:
   - `poll_and_advance_prd` now maintains a `HashSet<u32>` of processed issue numbers across both poll passes (`ralph:prd` and `ralph:prd-active`). An issue seen in the first pass is skipped in the second, enforcing the spec invariant "at most one state transition per issue per tick".
   - `do_pending_to_awaiting` now performs idempotent label operations: it checks current issue labels before attempting `remove_label_with_retry("ralph:prd")` (only if still present) and `add_label_with_retry("ralph:prd-active")` (only if not already present). This prevents failures on retry when labels were already swapped in a prior failed attempt.

3. **Added conformance coverage for runtime behaviors (Required #3)** — Added three new conformance tests to `src/validate/tests_interactive_prd.rs`:
   - `startup_prd_label_ensure`: Runs `daemon start --single-iteration` with mocked `gh`, verifies all 5 PRD labels and 4 standard labels are created (9 total `label create` calls).
   - `prd_ready_conflict_in_claim_path`: Runs daemon with an issue carrying both `ralph:ready` and `ralph:prd`, verifies the normal claim path does NOT claim it (no `ralph:in-progress` label swap).
   - `idempotent_state_reprocessing`: Exercises save/load/save/load cycle verifying state equality survives daemon restart reprocessing.

4. **Fixed existing conformance tests for PRD labels (Required #4)** — Updated three tests in `src/validate/tests_daemon.rs` (`label_ensure_startup`, `label_ensure_already_exists`, `label_ensure_hard_failure`) to assert `github::REQUIRED_LABELS.len() + PRD_LABELS.len()` (9) total label create calls instead of just `REQUIRED_LABELS.len()` (4), accounting for the 5 PRD lifecycle labels now created at startup.

5. **Validate PRD config at daemon startup (Recommended #1)** — Added a call to `validate_interactive_prd_workspace_config` in `src/cli/daemon.rs` during startup (gated by `daemon_cfg.prd_enabled`), so invalid PRD backend specs fail fast during `daemon start`. Made the function `pub` in `src/config/mod.rs`.

## Could Not Address
None

## Pending Changes (Pre-Commit)
- `src/daemon/runtime.rs` — Fixed `data_dir` derivation in `run_prd_phase` to avoid double owner/repo segment
- `src/daemon/interactive_prd.rs` — Added per-tick issue deduplication via `HashSet`; made label operations idempotent in `do_pending_to_awaiting`
- `src/validate/tests_interactive_prd.rs` — Added 3 new conformance tests: `startup_prd_label_ensure`, `prd_ready_conflict_in_claim_path`, `idempotent_state_reprocessing`
- `src/validate/tests_daemon.rs` — Updated 3 label ensure tests to account for 9 total labels (standard + PRD)
- `src/cli/daemon.rs` — Added PRD config validation at startup
- `src/config/mod.rs` — Made `validate_interactive_prd_workspace_config` public
