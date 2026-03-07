---
artifact: completer-verdict
loop: 7
project: issue-170
backend: claude(opus)
role: completer
created_at: 2026-03-06T19:22:33Z
---

All mock scripts are properly implemented. Now let me do a final count verification of the 23 new tests.

**New test modules (12 tests):**
- `tests_backend_exec.rs`: 4 tests (happy_path_echo, unknown_backend, raw_suppresses_metrics, prompt_from_file)
- `tests_openrouter.rs`: 2 tests (model_injection, disabled_default_backend)
- `tests_validate_flags.rs`: 3 tests (list_prints_names, filter_nonexistent_zero, single_job_filter)
- `tests_quick_prd.rs`: 3 tests (non_interactive_happy_path, dry_run_no_artifact, backend_override_proof)

**Extensions to existing modules (11 tests):**
- `tests_commands.rs`: 4 tests (rollback_dry_run, rollback_with_completion_attempts, rollback_force_push, config_edit_no_editor)
- `tests_tail.rs`: 1 test (follow_flag_accepted)
- `tests_project.rs`: 1 test (new_from_parent)
- `tests_prd.rs`: 2 tests (prd_resume_fewer_invocations, prd_answers_ingested)
- `tests_run.rs`: 3 tests (on_prompt_change_flag_accepted, on_prompt_change_abort_triggers, workspace_root_uses_alternate_path)

**Total: 23 new tests** — matches the requirement exactly.

# Verdict: COMPLETE

The project satisfies all requirements:

- **23 new conformance tests**: All 23 are implemented and registered in `mod.rs` — 4 new modules (12 tests) + 5 extended modules (11 tests).
- **`write_stable_mock_script` harness method** (`harness.rs:219`): Writes inner bash script + POSIX wrapper with `#!/bin/sh\nexec bash`.
- **6 mock script helpers** (`mock_scripts.rs`): `prd_mock_response_body`, `backend_exec_echo_script`, `openrouter_arg_logging_script`, `prd_invocation_counting_script`, `prd_stdin_capturing_script`, `prompt_mutating_mock_script` — all present with correct signatures.
- **Backend exec tests** (criteria 1): Happy path asserts stdin echo + `tokens_in` in stderr; unknown backend asserts non-zero + `unknown`; `--raw` asserts no `tokens_in`; `--prompt` reads from file.
- **Config edit test** (criteria 2): Sets `EDITOR` to nonexistent, unsets `VISUAL`, asserts non-zero + `failed to launch editor`.
- **Quick-PRD tests** (criteria 3): Non-interactive writes artifact; `--dry-run` shows idea, writes no artifact; override proof poisons codex then succeeds with `--writer-backend claude --reviewer-backend claude`.
- **Rollback `--dry-run`** (criteria 4): Asserts `dry-run` in output, HEAD unchanged, loop directories preserved.
- **Rollback with completion attempts** (criteria 5): Creates completion attempt, rollbacks, verifies state removed.
- **`tail --follow`** (criteria 6): Spawns child, `try_wait()` returns `None`, kills, no unrecognized flag errors.
- **`project new --from`** (criteria 7): Creates parent/child, `project show --json` includes `parent_project`.
- **`prd --resume`** (criteria 8): Invocation-counting mock proves second run has fewer invocations.
- **`prd --answers`** (criteria 9): YAML answers ingested, captured stdin contains sentinel value.
- **`--on-prompt-change`** (criteria 10): Parsing test succeeds; abort test mutates prompt and asserts non-zero + `prompt changed`.
- **OpenRouter model injection** (criteria 11): Logs `--model` + `test-model`; disabled backend asserts `unavailable` + no log file.
- **`--workspace-root`** (criteria 12): Moves `.ralph` to alternate path, verifies failure without flag, success with `--workspace-root`. Branch-agnostic via dynamic `rev-parse`.
- **Force-push rollback** (criteria 13): Three-way assertion: remote differs from target before, local==target after, remote==local after, remote changed.
- **Validate self-flags** (criteria 14): `--list` prints test names; `--filter nonexistent` shows 0 tests; `-j 1 --filter` shows 1 test with jobs=1.
- **Non-vacuous proofs**: All tests initialize workspace before post-discovery errors, assert liveness before kill, check both error text and log file absence, poison default paths, and use observable artifacts (counter deltas, captured stdin).
- **Nix-sandbox-safe**: All mocks use POSIX `#!/bin/sh` scripts or `write_stable_mock_script` wrappers.
- **No production behavior changes**: All changes are in `src/validate/`.

---
