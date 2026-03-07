## Objective
Expand `ralph validate` conformance coverage for currently untested CLI surfaces, backend families (`claude`, `codex`, `openrouter`), and key behavioral gaps.

This is **test-only** work in `src/validate/` plus harness/mock helpers. Do not change production behavior unless required to support test harness utilities.

## Scope
Implement **23 new conformance tests** and required harness/mock support.

## Mandatory Constraints
- Use validate harness patterns only (`ConformanceTest`, `RalphHarness`, `run_case`).
- All backend mocks must be Nix-sandbox-safe:
1. POSIX `#!/bin/sh` scripts via `write_mock_script`, or
2. `setup_mock_backends_stable(...)`, or
3. New helper `write_stable_mock_script(...)` that wraps bash scripts with `#!/bin/sh` + `exec bash`.
- Use contains-based assertions for error text; do not assert exact full error strings or exact exit codes beyond zero/non-zero.
- No network access, no real API keys, no external services.
- Interactive paths remain excluded.

## Explicit Exclusions
- `prd --dry-run` (currently non-functional in production flow).
- Interactive CLI behavior (`quick-prd --interactive`, real editor workflows).
- Distinct semantics for `rollback --hard` (flag currently defined but behavior not distinct in production).
- New CI gating or long-term coverage matrix maintenance.

## Required Harness/Mock Additions
1. Add to `src/validate/harness.rs`:
   `write_stable_mock_script(&self, name: &str, bash_content: &str) -> Result<PathBuf>`
   Behavior: write inner bash script, write POSIX wrapper `#!/bin/sh\nexec bash "<inner>"\n`, return wrapper path.
2. Add to `src/validate/mock_scripts.rs`:
   - `prd_mock_response_body() -> String`
   - `backend_exec_echo_script() -> String` (POSIX cat echo)
   - `openrouter_arg_logging_script(log_path: &Path) -> String` (logs args, consumes stdin, returns success)
   - `prd_invocation_counting_script(counter_path: &Path) -> String` (bash)
   - `prd_stdin_capturing_script(output_dir: &Path) -> String` (bash)
   - `prompt_mutating_mock_script(prompt_path: &Path) -> String` (bash; mutate once when planner prompt detected)

## Required New Test Modules
- `src/validate/tests_backend_exec.rs` (4 tests)
- `src/validate/tests_openrouter.rs` (2 tests)
- `src/validate/tests_validate_flags.rs` (3 tests)
- `src/validate/tests_quick_prd.rs` (3 tests)

Register them in `src/validate/mod.rs`.

## Required Extensions to Existing Modules
- `tests_commands.rs`: `rollback_dry_run`, `rollback_with_completion_attempts`, `rollback_force_push`, `config_edit_no_editor`
- `tests_tail.rs`: `follow_flag_accepted`
- `tests_project.rs`: `new_from_parent`
- `tests_prd.rs`: `prd_resume_fewer_invocations`, `prd_answers_ingested` (and refactor PRD mock to shared helper)
- `tests_run.rs`: `on_prompt_change_flag_accepted`, `on_prompt_change_abort_triggers`, `workspace_root_uses_alternate_path`

## Behavioral Acceptance Criteria
1. `backend exec`:
   - Happy path with echo mock: stdin echoed to stdout, metrics block contains `tokens_in`.
   - Unknown backend path must initialize workspace first; assert non-zero and stderr contains `unknown`.
   - `--raw` suppresses normalized metrics (no `tokens_in` in stderr).
   - `--prompt <file>` reads prompt from file and succeeds.
2. `config edit`:
   - Initialize workspace first.
   - Set `EDITOR` to nonexistent binary and unset `VISUAL`.
   - Assert non-zero and stderr contains `failed to launch editor`.
3. `quick-prd` standalone:
   - Non-interactive happy path succeeds and writes expected output artifact.
   - `--dry-run` succeeds, shows rendered idea text, writes no spec artifact.
   - Override proof: poison default codex command; run with `--writer-backend claude --reviewer-backend claude`; assert success.
4. `rollback --dry-run`:
   - Prints planned action (`dry-run:`) and does not change HEAD or loop directories.
5. `rollback` with completion attempts:
   - Create completion attempt, rollback to earlier feature loop, verify completion-attempt state removed and git reset.
6. `tail --follow`:
   - Spawn child, verify `try_wait()` is `Ok(None)` after short delay, then kill.
   - Assert no unrecognized/unknown flag errors.
7. `project new --from`:
   - Create parent and child project; `project show --json` for child includes `parent_project` referencing parent.
8. `prd --resume`:
   - Invocation-counting mock proves second run invokes backend fewer times than first run.
9. `prd --answers`:
   - YAML answers file is ingested; captured stdin contains sentinel answer value.
10. `--on-prompt-change`:
   - Parsing test: `run --on-prompt-change abort --loops 1` succeeds.
   - Runtime abort test: mutate prompt during planner phase; run with abort mode and assert non-zero + stderr contains `prompt changed`.
11. OpenRouter:
   - Model injection via `backend exec "openrouter(test-model)"` logs `--model` and `test-model`.
   - Disabled backend as default: set default backend to openrouter + disabled flag; assert failure contains `unavailable` and openrouter mock log file is absent (proves no process spawn).
12. `--workspace-root`:
   - No workspace at repo root; create alternate workspace and run command with `--workspace-root`; assert success.
13. Force-push after rollback:
   - Three-way assertion: remote head before rollback differs from target; after rollback local==target and remote==local and remote changed from prior head.
14. `ralph validate` self-flags:
   - `--list` prints known test names.
   - `--filter nonexistent_prefix_zzz` reports running 0 tests.
   - `-j 1 --filter run::single_feature_loop` reports running 1 test with jobs=1.
15. All tests remain non-vacuous and deterministic under validate harness.

## Non-Vacuous Proof Requirements
- Initialize workspace before tests that must exercise post-discovery errors.
- For long-running `tail --follow`, assert liveness before kill.
- For disabled OpenRouter, require both error text and no log file.
- For backend override tests, force failure of default path so success proves override took effect.
- For resume/answers tests, require observable artifacts (counter deltas / captured stdin).

## Verification Commands
Run in order:
```bash
nix develop -c cargo check
nix develop -c cargo test
nix build -L
./result/bin/ralph validate --bin ./result/bin/ralph --filter backend_exec
./result/bin/ralph validate --bin ./result/bin/ralph --filter openrouter
./result/bin/ralph validate --bin ./result/bin/ralph --filter quick_prd
./result/bin/ralph validate --bin ./result/bin/ralph --filter validate_flags
./result/bin/ralph validate --bin ./result/bin/ralph
```

## Done Criteria
1. 23 new validate tests implemented and registered.
2. Harness/helper and mock additions implemented as specified.
3. All targeted filters pass.
4. Full validate suite passes.
5. No production behavior changes outside test support utilities.