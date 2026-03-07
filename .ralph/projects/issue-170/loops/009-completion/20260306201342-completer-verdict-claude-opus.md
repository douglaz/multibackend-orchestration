---
artifact: completer-verdict
loop: 9
project: issue-170
backend: claude(opus)
role: completer
created_at: 2026-03-06T20:13:42Z
---

# Verdict: COMPLETE

The project satisfies all requirements:

- **Harness addition (`write_stable_mock_script`)**: Implemented at `harness.rs:219-230`, writes inner bash script + POSIX `#!/bin/sh` wrapper that `exec bash "<inner>"`, returns wrapper path — exactly as specified.

- **Mock helpers (6 functions in `mock_scripts.rs`)**: All 6 present — `prd_mock_response_body` (line 10), `backend_exec_echo_script` (line 138), `openrouter_arg_logging_script` (line 147), `prd_invocation_counting_script` (line 165), `prd_stdin_capturing_script` (line 200), `prompt_mutating_mock_script` (line 226).

- **4 new test modules registered in `mod.rs`**: `tests_backend_exec` (line 15/137), `tests_openrouter` (line 26/138), `tests_validate_flags` (line 43/139), `tests_quick_prd` (line 42/140).

- **23 new conformance tests verified in source**:
  1. `backend_exec::happy_path_echo` — echo mock, stdin echoed, `tokens_in` in stderr
  2. `backend_exec::unknown_backend` — non-zero exit, stderr contains `unknown`
  3. `backend_exec::raw_suppresses_metrics` — `--raw` mode, no `tokens_in` in stderr
  4. `backend_exec::prompt_from_file` — `--prompt <file>` reads and echoes content
  5. `openrouter::model_injection` — `openrouter(test-model)` logs `--model` and `test-model`
  6. `openrouter::disabled_default_backend` — failure contains `unavailable`, log file absent (no spawn)
  7. `validate_flags::list_prints_names` — `--list` prints known test names
  8. `validate_flags::filter_nonexistent_zero` — `--filter nonexistent_prefix_zzz` reports 0 tests
  9. `validate_flags::single_job_filter` — `-j 1 --filter` reports 1 test with jobs=1
  10. `quick_prd::non_interactive_happy_path` — succeeds, writes spec artifact in workspace
  11. `quick_prd::dry_run_no_artifact` — succeeds, shows idea text, no spec artifact written
  12. `quick_prd::backend_override_proof` — poisons codex, overrides with `--writer-backend claude --reviewer-backend claude`, succeeds
  13. `commands::rollback_dry_run` — prints `dry-run`, HEAD unchanged, loop directories intact
  14. `commands::rollback_with_completion_attempts` — completion attempt state removed, git reset verified
  15. `commands::rollback_force_push` — three-way assertion: remote differs from target, after rollback local==target==remote, remote changed
  16. `commands::config_edit_no_editor` — nonexistent EDITOR, VISUAL unset, non-zero exit, stderr contains `failed to launch editor`
  17. `tail::follow_flag_accepted` — spawns child, `try_wait()` returns `Ok(None)` after 500ms, kills, no unknown flag errors
  18. `project::new_from_parent` — creates parent+child, `project show --json` includes `parent_project`
  19. `prd::prd_resume_fewer_invocations` — invocation-counting mock proves second run has fewer invocations
  20. `prd::prd_answers_ingested` — YAML answers file ingested, captured stdin contains sentinel value
  21. `run::on_prompt_change_flag_accepted` — `--on-prompt-change abort --loops 1` succeeds
  22. `run::on_prompt_change_abort_triggers` — prompt mutated during planner phase via `prompt_mutating_mock_script`, non-zero exit, stderr contains `prompt changed`, mutation sentinel file proves non-vacuous
  23. `run::workspace_root_uses_alternate_path` — branch-agnostic alternate workspace, `--workspace-root` flag succeeds

- **Behavioral acceptance criteria 1–15**: All covered by the tests above with correct assertion patterns (contains-based, non-exact).

- **Non-vacuous proof requirements**: Workspace initialized before post-discovery tests; `tail --follow` liveness asserted before kill; disabled OpenRouter checks both error text and absent log file; backend override poisons default path; resume/answers use observable artifacts (counter deltas / captured stdin); prompt mutation uses sentinel file to prove non-vacuous.

- **Nix-sandbox-safe mocks**: All use `#!/bin/sh` POSIX scripts via `write_mock_script`, `setup_mock_backends_stable`, or `write_stable_mock_script`.

- **No production behavior changes**: All changes confined to `src/validate/` test and harness code.

---
