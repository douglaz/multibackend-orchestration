## Summary

Expand `ralph validate` coverage to exercise untested CLI surfaces, all three backend families (claude, codex, openrouter), and key gaps in existing test modules. An audit of the 290+ existing validate tests against the full CLI/config surface reveals ~17 distinct gap areas: untested commands (`backend exec`, `config edit`, `quick-prd` as standalone CLI), untested flags (`rollback --dry-run`, `tail --follow`, `project new --from`, `--workspace-root`, `prd --resume`, `prd --answers`, `quick-prd --dry-run`, `backend exec --raw`, `backend exec --prompt`, `quick-prd --writer-backend`/`--reviewer-backend`), missing backend family coverage (openrouter lacks `backend exec` model-injection tests and direct disabled-as-default-backend tests), under-tested areas (`--on-prompt-change` as a CLI flag, force-push behavior post-rollback), and self-test coverage for `ralph validate` flags (`--list`, `--filter`, `-j`).

The `prd --dry-run` flag is excluded because it is defined but non-functional (the pipeline unconditionally writes `PRD.md`). Interactive CLI paths (`quick-prd --interactive`, `config edit` with a real editor) are excluded because validate tests run non-interactively. The `--hard` flag on rollback is defined but unused in production code (`src/cli/rollback.rs` never reads `args.hard` — rollback always performs git reset + force-push); the spec tests rollback with completion attempts but does not rely on `--hard` having distinct behavior. This is a one-time expansion; no ongoing coverage matrix or CI gating is required.

All mock backend commands use Nix-sandbox-safe execution: either pure POSIX `#!/bin/sh` scripts (written via `write_mock_script`) or bash scripts wrapped via `setup_mock_backends_stable` / the new `write_stable_mock_script` helper, which creates a `#!/bin/sh` wrapper that calls `exec bash <script>`. This avoids `/usr/bin/env` shebang failures in Nix sandboxes, per `docs/validate-e2e.md` troubleshooting guidance.

## Acceptance Criteria

1. **`backend exec` command**: Four tests. Happy-path: set the claude backend to a dedicated echo mock (`#!/bin/sh\ncat`) via `set_config_fast`, invoke `backend exec claude` with stdin containing a test prompt. Verify exit 0, stdout contains the echoed text, and stderr contains `tokens_in` (from the normalized metrics block at `src/cli/backend.rs:158`). Error-path: call `h.init_workspace()` first (ensuring `Workspace::discover()` at `src/cli/backend.rs:47` succeeds), then invoke with `"unknown_backend_xyz"`; verify non-zero exit, stderr contains "unknown" (from `src/cli/backend.rs:59`). Without workspace init, the test could fail at workspace discovery instead of validating unknown-backend handling. Raw-flag: invoke `backend exec claude --raw`; verify exit 0, stdout contains echoed text, stderr does NOT contain `tokens_in` (the `--raw` flag at `src/cli/backend.rs:147` bypasses normalization). Prompt-file: write a prompt to a temp file, invoke `backend exec claude --prompt <path>`; verify exit 0 and stdout contains the file's prompt text.

2. **`config edit` command**: One test. Call `h.init_workspace()` first (ensuring workspace discovery succeeds). Set `EDITOR=nonexistent-binary-ralph-test-xxxxx` and remove `VISUAL`. Verify non-zero exit and stderr contains "failed to launch editor" (the error from `src/cli/config.rs:350`). Do NOT rely on `EDITOR` being unset — the code at `src/cli/config.rs:334` falls back to `vi` which may be present.

3. **`quick-prd` standalone invocation**: Three tests. Happy-path: invoke `ralph quick-prd --idea "test idea" --non-interactive` with writer/reviewer mocks configured via `setup_mock_backends_stable`; verify exit 0 and output file exists. Dry-run: invoke `ralph quick-prd --idea "test idea" --dry-run`; verify exit 0, stdout contains rendered prompt text (the `{{idea}}` placeholder is replaced at `src/cli/quick_prd.rs:40`), and no spec file is written. Backend-override (non-vacuous): configure the claude mock backend via `setup_mock_backends_stable`, explicitly poison `backends.codex.command` to `nonexistent-codex-binary-ralph-test-xxxxx` via `set_config_fast`, invoke with `--writer-backend claude --reviewer-backend claude`. Verify exit 0 — success proves the override was effective because the default reviewer backend is codex (from `default_daemon_prd_reviewer_backend()` at `src/config/global.rs:908`), which would fail with the poisoned command.

4. **`rollback --dry-run`**: One test confirming dry-run prints planned actions (stdout contains "dry-run:" per `src/cli/rollback.rs:74`) but does not modify loop directories or git HEAD.

5. **`rollback` with completion attempts**: One test where a project reaches a completion loop via `RALPH_COMPLETE=yes`, then rollback to a prior feature loop verifies completion-attempt state is removed and git is reset. Note: `--hard` is unused in production code (never read from `args`), so this test uses plain `rollback <N>`.

6. **`tail --follow`**: One test with explicit child-process lifecycle control. Set up workspace via `h.init_workspace()`, configure mocks via `setup_mock_backends_stable`, create project, run `--loops 1` to generate state. Spawn `ralph tail --follow --project <id> --poll-interval-ms 50` via `std::process::Command`. Wait briefly, assert `child.try_wait()` returns `Ok(None)` (proving it entered the polling loop at `src/cli/tail.rs:145-179` and did not exit early). Kill child. Assert stderr does not contain "unknown" or "unrecognized". The `try_wait()` assertion makes this non-vacuous.

7. **`project new --from`**: One test creating project A, then `project new --id child --name Child --from A`, then `project show child --json`; verify JSON state contains `parent_project` field (from `src/project/state.rs:20`) referencing A.

8. **`prd --resume`**: One test with observable proof of cache reuse. First run: `prd --idea "test" --non-interactive` with invocation-counting mock (bash script installed via `write_stable_mock_script` for Nix safety). Record count. Truncate counter file. Second run: `prd --resume --idea "test" --non-interactive`. Assert second count < first count (proving stages were skipped via cache).

9. **`prd --answers`**: One test with observable proof of answer ingestion. Write a YAML answers file. Invoke `prd --idea "test" --non-interactive --answers <path>` with a stdin-capturing mock (bash script installed via `write_stable_mock_script` for Nix safety). Assert at least one captured stdin contains the answer value.

10. **`--on-prompt-change` CLI flag**: Two tests. (a) Flag-parsing: `run --on-prompt-change abort --loops 1` with mocks configured via `setup_mock_backends_stable` completes exit 0. (b) Runtime abort: disable prompt review via `set_config_fast("workflow.prompt_review_enabled", "false")` so planning is the first orchestrator phase. Configure a prompt-mutating mock (bash script installed via `write_stable_mock_script`) that modifies `prompt.md` during the planner invocation. Run with `--on-prompt-change abort --loops 2`. Assert non-zero exit and stderr contains "prompt changed" (from `src/workflow/orchestrator.rs:2763-2765`).

11. **OpenRouter backend**: Two mock-based tests. Model injection: configure openrouter via `set_config_fast`, setting `backends.openrouter.command` to a POSIX `#!/bin/sh` arg-logging mock (written via `write_mock_script`). Invoke `backend exec "openrouter(test-model)"`, verify logged args contain `--model` and `test-model` (injected at `src/backend/openrouter.rs:24-30`). Disabled error: set `backends.openrouter.enabled` to `"false"`, set `backends.openrouter.command` to a POSIX arg-logging mock with a distinct log path, set `workspace.default_backend` to `"openrouter"`, configure claude/codex with `setup_mock_backends_stable`. Create a project and run with `--loops 1`. Assert non-zero exit, stderr contains "unavailable" (from `RalphError::BackendUnavailable` at `src/error.rs:58`), AND the openrouter mock log file does NOT exist (proving the disabled flag prevented command spawning, not just that the error message appeared). This two-part assertion is non-vacuous: `stderr` containing "unavailable" proves the error was raised, and the absent log file proves the disabled-backend short-circuit prevented the process from being spawned at all.

12. **`--workspace-root` flag**: One test. Do NOT init workspace at `repo_root`. Create workspace at `h.repo_root.join("alt_ws")` via `crate::cli::init::create_workspace`. Configure mocks on alternate workspace via `Workspace::load` + `set_global_config_value` + `save_config()`. Run `ralph run --workspace-root <alt_ws> --project <id> --loops 1`. Assert exit 0. Non-vacuous: without `--workspace-root`, `Workspace::discover()` would fail.

13. **Force-push after rollback**: One test with three-way comparison. Run `--loops 2`, capture `remote_head_before` and rollback target. Assert `remote_head_before != rollback_target`. Execute rollback. Assert `local_head_after == rollback_target`, `remote_head_after == local_head_after`, `remote_head_after != remote_head_before`. This proves the force-push at `src/cli/rollback.rs:114-118` moved the remote.

14. **`ralph validate` self-test flags**: Three tests. `--list`: verify exit 0 and stdout contains a known test name. `--filter nonexistent_prefix_zzz`: verify output contains "running 0 tests". `-j 1 --filter run::single_feature_loop`: verify stdout contains "running 1 tests (jobs: 1)" (matching format at `src/validate/runner.rs:50`).

15. All new tests run within the existing `ralph validate` harness using `ConformanceTest` structs and `RalphHarness`. No external services or real API keys required.

16. Tests assert non-exact error text (contains-based checks), not exact exit codes or full message strings.

## Technical Approach

### Nix-sandbox-safe mock execution

Per `docs/validate-e2e.md` troubleshooting: "If backend scripts fail only in Nix environments, prefer `setup_mock_backends_stable()` so wrappers use `/bin/sh` + `bash` and clear default backend args." All mock scripts configured as backend commands MUST use one of these three approaches:

1. **POSIX `#!/bin/sh` scripts** written via `write_mock_script` — safe to set directly via `set_config_fast` or any config method. Suitable for simple scripts that avoid bashisms (`<<<`, `[[ ]]`, `set -o pipefail`).
2. **`setup_mock_backends_stable(script)`** — wraps a bash script in a `#!/bin/sh` → `exec bash <script>` wrapper and sets it for claude+codex. Use for tests that configure both standard backends together.
3. **`write_stable_mock_script(name, bash_content)`** — new harness helper (see below). Wraps a bash script for individual backend config via `set_config_fast`. Use for bash scripts that need to be set on a single backend.

Classification of new mock scripts:
- **POSIX-safe** (use `write_mock_script`): `backend_exec_echo_script()`, `openrouter_arg_logging_script()`
- **Bash-requiring** (use `write_stable_mock_script` or `setup_mock_backends_stable`): `prd_invocation_counting_script()`, `prd_stdin_capturing_script()`, `prompt_mutating_mock_script()` — all compose `prd_mock_response_body()` or `standard_mock_script()` patterns that use bash here-strings (`<<<`) and `set -euo pipefail`

### New harness helper: `write_stable_mock_script`

Add to `RalphHarness` in `harness.rs`:

```rust
pub fn write_stable_mock_script(&self, name: &str, bash_content: &str) -> Result<PathBuf> {
    let inner = self.write_mock_script(name, bash_content)?;
    let wrapper_name = format!("{name}-stable.sh");
    let inner_str = inner.to_string_lossy();
    let wrapper_content = format!("#!/bin/sh\nexec bash \"{inner_str}\"\n");
    self.write_mock_script(&wrapper_name, &wrapper_content)
}
```

This mirrors the wrapping logic from `setup_mock_backends_stable` (line 253 of `harness.rs`) but returns the wrapper path for use with `set_config_fast` on individual backends. The wrapper does NOT pass `"$@"` through to bash, matching the existing stable wrapper behavior — mock backend scripts read from stdin and do not process positional args (model flags are injected by the backend registry but are irrelevant to mock behavior).

### OpenRouter configuration via `set_config_fast`

The Gemini removal PR (ccb9a80) replaced all `backends.gemini.*` branches in `set_global_config_value` with `backends.openrouter.*` branches. All openrouter keys are supported in `src/config/global.rs`: `command` (line 1527), `timeout_seconds` (lines 1534–1535), `enabled` (lines 1543–1544), `role_timeouts.*` (lines 1554–1560), `args` (lines 1564–1565), `models.*` (lines 1575–1577), and `env.*` (lines 1595–1601). OpenRouter tests use `set_config_fast` — the same mechanism used by claude and codex tests — with no new harness infrastructure needed.

### `--hard` flag observation

The `--hard` flag on `RollbackArgs` (`src/cli/mod.rs:206`) is defined but never referenced in `src/cli/rollback.rs` — `args.hard` is never read. Rollback always computes a git reset ref (line 54) and always performs `reset_hard` + force-push (lines 86–118). The existing test `commands::rollback_hard` passes only because rollback is always hard. New tests do not depend on `--hard` having distinct behavior; the completion-attempt rollback test uses plain `rollback <N>`.

### New test modules

- **`src/validate/tests_backend_exec.rs`** — Four tests for `backend exec`. Happy-path uses `backend_exec_echo_script()` (a POSIX `#!/bin/sh\ncat` script), written via `write_mock_script` and set as claude command via `set_config_fast("backends.claude.command", ...)` with `set_config_fast("backends.claude.args", "[]")`. Invokes `h.ralph_with_stdin(["backend", "exec", "claude"], "hello from test")`. Asserts exit 0, `assert_stdout_contains("hello from test")`, `assert_stderr_contains("tokens_in")`. Error test calls `h.init_workspace()` first (precondition for `Workspace::discover()` at `src/cli/backend.rs:47`), then invokes with `"unknown_backend_xyz"`, asserts non-zero + stderr "unknown". Raw test adds `--raw`, asserts stdout contains text but stderr does NOT contain `tokens_in`. Prompt-file test writes to a temp file, invokes with `--prompt <path>`, no stdin needed.

- **`src/validate/tests_openrouter.rs`** — Two tests. Model-injection: writes `openrouter_arg_logging_script(log_path)` (POSIX `#!/bin/sh`) via `write_mock_script`, configures openrouter via `set_config_fast("backends.openrouter.enabled", "true")`, `set_config_fast("backends.openrouter.command", <mock_path>)`, `set_config_fast("backends.openrouter.args", "[]")`. Invokes `h.ralph_with_stdin(["backend", "exec", "openrouter(test-model)"], "test prompt")`. Asserts log file contains `--model` and `test-model`. Disabled-error: configures claude/codex with `setup_mock_backends_stable(&standard_mock_script)` (Nix-safe wrapping for the bash-based standard mock), then sets `set_config_fast("backends.openrouter.enabled", "false")`, `set_config_fast("backends.openrouter.command", <arg_logging_mock>)` (POSIX script, distinct log path), and `set_config_fast("workspace.default_backend", "openrouter")`. Creates project, runs with `--loops 1`. Asserts non-zero exit, stderr contains "unavailable", AND the openrouter mock log file does NOT exist (proving the disabled flag prevented command spawning at the `BackendRegistry` level before any process was started).

- **`src/validate/tests_validate_flags.rs`** — Three self-tests. `--list`: invoke `ralph validate --bin <bin> --list`, verify exit 0 and stdout contains "run::". `--filter nonexistent_prefix_zzz`: verify output contains "running 0 tests". `-j 1 --filter run::single_feature_loop`: verify stdout contains "running 1 tests (jobs: 1)".

- **`src/validate/tests_quick_prd.rs`** — Three tests. Standalone happy-path: configure mocks via `setup_mock_backends_stable(&auto_mock_script)` (Nix-safe wrapping for the bash-based auto mock which handles quick-PRD writer/reviewer prompts at `mock_scripts.rs:295-319`), invoke `quick-prd --idea "test" --non-interactive`, assert exit 0. Dry-run: invoke `quick-prd --idea "test" --dry-run`, assert stdout contains "test" (the rendered idea), assert no spec file. Backend-override: configure claude mock via `setup_mock_backends_stable`, poison codex with `set_config_fast("backends.codex.command", "nonexistent-codex-binary-ralph-test-xxxxx")`, invoke with `--writer-backend claude --reviewer-backend claude`, assert exit 0.

### Extensions to existing test modules

- **`tests_commands.rs`**: Add `rollback_dry_run` (run `--loops 2`, invoke `rollback --dry-run 1`, assert stdout contains "dry-run:", verify HEAD unchanged and loop dirs intact), `rollback_with_completion_attempts` (run loops with `RALPH_COMPLETE=yes` to trigger completion, rollback to feature loop, verify `completion_attempts` cleared in state), `rollback_force_push` (three-way remote/local/target comparison), `config_edit_no_editor` (init workspace first, set EDITOR to nonexistent binary, remove VISUAL). All tests that run loops use `setup_mock_backends_stable` for Nix-safe mock execution, consistent with the existing `rollback_hard` test in this module.

- **`tests_tail.rs`**: Add `follow_flag_accepted` with workspace/project setup. Configure mock backends via `setup_mock_backends_stable` (not `setup_mock_backends_fast`, ensuring Nix safety for the bash-based standard mock). Run `--loops 1` to generate state. Spawn `tail --follow`, assert liveness via `try_wait()`, kill, check output.

- **`tests_project.rs`**: Add `new_from_parent`. Create project A with prompt, then `project new --id child --name Child --from A`, then `project show child --json` and assert `parent_project` field.

- **`tests_prd.rs`**: Add `prd_resume_fewer_invocations` and `prd_answers_ingested`. Both use PRD-specific mock scripts from `mock_scripts.rs` that compose the shared `prd_mock_response_body()` helper. These are bash scripts, so they are installed via `write_stable_mock_script` and their wrapper paths are set via `set_config_fast` for the relevant backends. Update existing `prd_mock_script()` to delegate to the shared helper.

- **`tests_run.rs`**: Add `on_prompt_change_flag_accepted` (exit 0 with abort flag + 1 loop; mocks via `setup_mock_backends_stable`), `on_prompt_change_abort_triggers` (prompt review disabled; `prompt_mutating_mock_script` installed via `write_stable_mock_script`, set as both backend commands via `set_config_fast`; runtime abort proof), `workspace_root_uses_alternate_path` (alternate workspace inside repo_root git worktree, no default .ralph; mocks configured on alternate workspace via `Workspace::load` + `set_global_config_value` using stable-wrapped script paths).

### Mock infrastructure

Five new mock script helpers added to `mock_scripts.rs`, plus one shared extraction:

- **`prd_mock_response_body() -> String`**: Extracted from the existing `prd_mock_script()` in `tests_prd.rs`. Returns the shell `if/elif/fi` block matching all PRD prompt families. All PRD mock variants share this single source of truth. Uses bash syntax (`<<<`, `set -euo pipefail`); callers must install via `write_stable_mock_script` or `setup_mock_backends_stable`.

- **`backend_exec_echo_script() -> String`**: Returns `"#!/bin/sh\ncat\n"`. POSIX-safe. Echoes stdin to stdout, ignoring args. Install via `write_mock_script`.

- **`openrouter_arg_logging_script(log_path: &Path) -> String`**: POSIX `#!/bin/sh` script. Writes `"$@"` to `log_path` via `printf '%s\n' "$@"`, consumes stdin via `cat > /dev/null`, prints mock response, exits 0. Install via `write_mock_script`.

- **`prd_invocation_counting_script(counter_path: &Path) -> String`**: Bash script. Appends a line to counter file per invocation, then delegates to `prd_mock_response_body()`. Install via `write_stable_mock_script`.

- **`prd_stdin_capturing_script(output_dir: &Path) -> String`**: Bash script. Writes stdin to unique file per invocation (`$$` PID), then delegates to `prd_mock_response_body()`. Install via `write_stable_mock_script`.

- **`prompt_mutating_mock_script(prompt_path: &Path) -> String`**: Bash script. Wraps `standard_mock_script()` behavior but when planner prompt pattern detected in stdin (matching "You are a software architect planning"), appends `"\n# Modified by mock"` to `prompt_path`. Uses marker file to prevent repeated mutations. Install via `write_stable_mock_script`.

### Registration

Each new module gets `pub fn tests() -> Vec<ConformanceTest>`. Add `mod tests_backend_exec; mod tests_openrouter; mod tests_validate_flags; mod tests_quick_prd;` to `src/validate/mod.rs` and extend `register_tests()`.

## Files & Modules

| File | Action |
|---|---|
| `src/validate/mod.rs` | Add `mod tests_backend_exec; mod tests_openrouter; mod tests_validate_flags; mod tests_quick_prd;` and register in `register_tests()` |
| `src/validate/harness.rs` | Add `write_stable_mock_script(name, bash_content) -> Result<PathBuf>` helper — writes bash script and creates `#!/bin/sh` wrapper returning wrapper path (mirrors `setup_mock_backends_stable` wrapping at line 253) |
| `src/validate/tests_backend_exec.rs` | **New** — 4 tests: happy-path (POSIX echo mock + metrics), unknown-backend error (with workspace init precondition), `--raw` (suppresses metrics), `--prompt` (reads from file) |
| `src/validate/tests_openrouter.rs` | **New** — 2 tests: model injection (POSIX arg-logging mock via `write_mock_script`), disabled-error (`setup_mock_backends_stable` for claude/codex + log file non-existence assertion) |
| `src/validate/tests_validate_flags.rs` | **New** — 3 self-tests: `--list`, `--filter`, `-j 1` |
| `src/validate/tests_quick_prd.rs` | **New** — 3 tests: standalone invocation (`setup_mock_backends_stable`), `--dry-run`, `--writer-backend`/`--reviewer-backend` override (codex poisoned) |
| `src/validate/tests_commands.rs` | Add `rollback_dry_run`, `rollback_with_completion_attempts`, `rollback_force_push`, `config_edit_no_editor` — all loop-running tests use `setup_mock_backends_stable` |
| `src/validate/tests_tail.rs` | Add `follow_flag_accepted` — uses `setup_mock_backends_stable` |
| `src/validate/tests_project.rs` | Add `new_from_parent` |
| `src/validate/tests_prd.rs` | Add `prd_resume_fewer_invocations`, `prd_answers_ingested` (both use `write_stable_mock_script` for bash PRD mocks); update `prd_mock_script()` to delegate to `mock_scripts::prd_mock_response_body()` |
| `src/validate/tests_run.rs` | Add `on_prompt_change_flag_accepted` (`setup_mock_backends_stable`), `on_prompt_change_abort_triggers` (`write_stable_mock_script` for mutation mock), `workspace_root_uses_alternate_path` (stable-wrapped scripts on alternate workspace) |
| `src/validate/mock_scripts.rs` | Add `prd_mock_response_body()` (bash), `backend_exec_echo_script()` (POSIX), `openrouter_arg_logging_script()` (POSIX), `prd_invocation_counting_script()` (bash), `prd_stdin_capturing_script()` (bash), `prompt_mutating_mock_script()` (bash) |

## Testing Strategy

All new tests are validate-suite conformance tests executed via `ralph validate --bin <path>`. They follow established patterns:

1. **Harness setup**: `RalphHarness::new(bin)` creates an isolated temp dir with a fresh git repo (including bare origin remote at `origin.git`). Each test gets its own `TempDir` — no shared state.

2. **Nix-sandbox-safe mock execution**: All mock scripts configured as backend commands use one of three Nix-safe patterns, per `docs/validate-e2e.md` troubleshooting guidance:
   - **POSIX `#!/bin/sh` scripts** (e.g., `backend_exec_echo_script`, `openrouter_arg_logging_script`) → written via `write_mock_script`, safe to set directly via `set_config_fast`.
   - **`setup_mock_backends_stable(script)`** → for tests that configure both claude+codex together with bash scripts (e.g., `standard_mock_script()`, `auto_mock_script()`). Creates a `#!/bin/sh` → `exec bash` wrapper and sets it for both backends.
   - **`write_stable_mock_script(name, bash_content)`** → for tests that configure individual backends via `set_config_fast` with bash scripts (e.g., PRD counting/capturing mocks, prompt mutation mock). Creates the same `#!/bin/sh` → `exec bash` wrapper and returns the wrapper path.
   
   No test sets a `#!/usr/bin/env bash` script directly as a backend command via `set_config_fast` — this pattern is forbidden because `/usr/bin/env` is absent in Nix sandboxes.

3. **Mock backends**: `setup_mock_backends_stable()` for standard orchestration tests. `set_config_fast` for backend-specific configuration (openrouter keys fully supported since ccb9a80). PRD tests use counting/capturing mocks composing the shared `prd_mock_response_body()`.

4. **Backend exec echo mock**: The `backend exec` happy-path uses `backend_exec_echo_script()` — a POSIX `#!/bin/sh\ncat` script — instead of `standard_mock_script()`, which rejects unrecognized prompt patterns with `exit 1`.

5. **Shared PRD mock helper**: `prd_mock_response_body()` extracted to `mock_scripts.rs` as the single source of truth for PRD prompt-family matching. All PRD mock variants compose it.

6. **Assertions**: Use `assert_exit_code`, `assert_stdout_contains`, `assert_stderr_contains`, `assert_file_exists`, `assert_file_contains`, `assert_json_field` from `src/validate/assertions.rs`. Error assertions use contains-based checks. Each test function is wrapped in `run_case()` for panic-safe `TestResult` conversion.

7. **Process lifecycle for `tail --follow`**: Spawns child process, asserts liveness via `try_wait()` returning `Ok(None)`, kills process, asserts on collected output. No reliance on self-termination; avoids suite hangs.

8. **Observable assertions for stateful commands**: `prd --resume` proves cache reuse via per-run invocation-count comparison (counter file truncated between runs). `prd --answers` proves ingestion via stdin-capture. Force-push uses three-way comparison with preconditions.

9. **Non-vacuous coverage strategies**:
   - `--workspace-root`: no `.ralph` at `repo_root`; alternate workspace inside git worktree
   - `tail --follow`: `try_wait().is_none()` proves active polling loop
   - OpenRouter disabled-error: `default_backend = "openrouter"` forces orchestrator to target it; log file non-existence proves command was never spawned (not just that the error message appeared)
   - `backend exec` error-path: workspace initialized first so the error originates from unknown-backend handling (`src/cli/backend.rs:58-63`), not from `Workspace::discover()` (`src/cli/backend.rs:47`)
   - `config edit`: workspace initialized first; error is from editor launch, not workspace discovery
   - `quick-prd` backend override: codex poisoned with nonexistent binary; success proves override worked
   - `--on-prompt-change abort`: prompt review disabled; planner-pattern mutation; runtime abort detected

10. **Verification**: Build and run using Nix-based project conventions:
    ```bash
    nix develop -c cargo build
    nix develop -c cargo test
    nix build -L
    ./result/bin/ralph validate --bin ./result/bin/ralph --filter <new_test_prefix>
    ```
    Then run full suite to verify no regressions.

No external dependencies, API keys, or network access. All backend interactions are mocked.

## Out of Scope

- **`prd --dry-run`**: Defined but non-functional — pipeline unconditionally writes `PRD.md` at `src/prd/pipeline.rs`. Testing requires production fix.
- **Interactive CLI paths**: `quick-prd --interactive`, `config edit` with real editor. No TTY in validate harness.
- **`backend exec --role`**: Model selection via role mappings already covered by unit tests in `src/backend/mod.rs`.
- **`quick-prd --max-revisions`**: Exercised by default value; non-default testing adds complexity without meaningful coverage beyond clap parsing.
- **`--hard` flag behavior**: `args.hard` is defined on `RollbackArgs` (`src/cli/mod.rs:206`) but never read in `src/cli/rollback.rs`. Rollback always performs git reset + force-push. Testing `--hard` as distinct from default rollback is vacuous until production code implements the distinction.
- **Maintained coverage matrix or CI gating**: One-time expansion only per user direction.
- **Exact exit-code and error-message contracts**: Contains-based assertions only.
- **OpenRouter integration tests**: No real Goose CLI or external API calls.
- **Production code changes**: Test-only spec. `set_global_config_value` already supports all openrouter keys.
- **Unit tests in `tests/` directory**: All new coverage goes into `src/validate/` conformance suite.