---
artifact: completer-verdict
loop: 5
project: issue-170
backend: claude(opus)
role: completer
created_at: 2026-03-06T18:43:19Z
---

All 6 required mock script functions are present and implemented correctly. Let me now do a final count verification.

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

**Total: 23 tests** — matches the requirement exactly.

---

# Verdict: COMPLETE

The project satisfies all requirements:
- **23 new conformance tests**: 12 in 4 new modules + 11 extensions across 5 existing modules — all verified in source.
- **`write_stable_mock_script`** in `harness.rs:219`: creates inner bash + POSIX sh wrapper as specified.
- **6 mock script helpers** in `mock_scripts.rs`: `prd_mock_response_body` (line 10), `backend_exec_echo_script` (line 138), `openrouter_arg_logging_script` (line 147), `prd_invocation_counting_script` (line 165), `prd_stdin_capturing_script` (line 200), `prompt_mutating_mock_script` (line 226).
- **Module registration**: All 4 new modules registered in `mod.rs` (lines 15, 26, 42-43) with `tests()` calls in `register_tests()` (lines 137-140).
- **Behavioral acceptance criteria 1** (backend exec): Happy path echoes stdin with `tokens_in` metrics; unknown backend asserts non-zero + "unknown"; `--raw` suppresses `tokens_in`; `--prompt` reads from file.
- **Behavioral acceptance criteria 2** (config edit): Sets nonexistent `EDITOR`, unsets `VISUAL`, asserts non-zero + "failed to launch editor".
- **Behavioral acceptance criteria 3** (quick-prd): Non-interactive writes spec artifact; `--dry-run` shows idea text, no artifact written; backend override poisons codex, succeeds with `--writer-backend claude --reviewer-backend claude`.
- **Behavioral acceptance criteria 4** (rollback --dry-run): Prints "dry-run", HEAD and loop dirs unchanged.
- **Behavioral acceptance criteria 5** (rollback with completion): Creates completion attempt, rolls back, verifies completion_attempts cleared and git reset.
- **Behavioral acceptance criteria 6** (tail --follow): Spawns child, verifies `try_wait()` is `None` after 500ms, kills, no unrecognized flag errors.
- **Behavioral acceptance criteria 7** (project new --from): Creates parent/child, verifies `parent_project` in JSON output.
- **Behavioral acceptance criteria 8** (prd --resume): Invocation-counting mock proves second run invokes fewer times.
- **Behavioral acceptance criteria 9** (prd --answers): YAML answers ingested; captured stdin contains sentinel value.
- **Behavioral acceptance criteria 10** (--on-prompt-change): Flag parsing succeeds; abort mode with prompt mutation asserts non-zero + "prompt changed".
- **Behavioral acceptance criteria 11** (OpenRouter): Model injection logs `--model` and `test-model`; disabled backend asserts "unavailable" and no log file.
- **Behavioral acceptance criteria 12** (--workspace-root): Moves `.ralph`, verifies failure without flag, success with `--workspace-root`.
- **Behavioral acceptance criteria 13** (force-push rollback): Three-way assertion on remote/local heads before and after rollback.
- **Behavioral acceptance criteria 14** (validate self-flags): `--list` prints test names; `--filter nonexistent_prefix_zzz` reports 0 tests; `-j 1 --filter` reports 1 test with jobs=1.
- **Non-vacuous proof requirements**: All satisfied — workspace init before error tests, liveness assertion before kill, dual assertions (error text + no log file) for disabled OpenRouter, poison-then-override pattern for backend override, observable artifacts (counter deltas / captured stdin) for resume/answers.
- **Nix sandbox safety**: Mock scripts use `#!/bin/sh` wrappers via `write_stable_mock_script` and `setup_mock_backends_stable`.
- **No production behavior changes**: All changes are in `src/validate/`.

---
