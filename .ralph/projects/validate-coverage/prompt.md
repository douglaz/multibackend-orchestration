# Expand validate conformance suite to cover missing features

## Goal

Add conformance tests for `ralph validate` covering untested features:
`tail` (one-shot mode), `config show`, `project list` (dedicated), and template fallback resilience.

## Context

The validate test framework lives in `src/validate/`. Tests are registered in `src/validate/mod.rs` via `register_tests()`, which collects `Vec<ConformanceTest>` from each `tests_*.rs` module. Each test receives a `RalphHarness` that provides an isolated git repo and a ralph binary path.

The harness (`src/validate/harness.rs`) provides helpers: `ralph()`, `ralph_ok()`, `ralph_env()`, `init_workspace()`, `create_project()`, `setup_mock_backends()`, `load_state()`, `loop_dir()`, `list_artifacts()`.

Assertions live in `src/validate/assertions.rs`: `assert_exit_code()`, `assert_stdout_contains()`, `assert_file_exists()`, `assert_json_field()`, `assert_json_array()`, etc.

Mock scripts are in `src/validate/mock_scripts.rs`: `standard_mock_script()` and `always_reject_review_script()`.

## New test file: `src/validate/tests_tail.rs`

Create a new test module with these tests. Follow the exact patterns in `tests_commands.rs` — each test function signature is `fn test_name(h: &RalphHarness) -> TestResult`, wrapped in `run_case(|| { ... })`. Include a local `run_case` and `setup_with_standard_mock` helper like the other modules do.

### `tail::one_shot_shows_artifacts`
1. Init workspace, create project, setup standard mock, run 1 loop
2. Run `ralph tail`
3. Assert stdout contains `"--- ["` (the event delimiter format used for both artifact and state events)
4. Assert stdout contains `"artifact="` (metadata line present in artifact events)
5. Assert stdout contains `"started"` (present in "loop N (name) started" state events)

### `tail::json_output_valid`
1. Init workspace, create project, setup standard mock, run 1 loop
2. Run `ralph tail --json`
3. For each non-empty line of stdout, parse as JSON (serde_json::from_str)
4. Assert each JSON object has `project_id`, `event_type`, and `timestamp` fields
5. Assert at least one event has `event_type == "artifact"`
6. Assert at least one event has `event_type == "state"`

### `tail::last_flag_limits_output`
1. Init workspace, create project, setup standard mock, run 1 loop
2. Run `ralph tail` to get full output
3. Run `ralph tail --last 1` to get limited output
4. Assert the `--last 1` output is shorter (fewer bytes) than the full output

### `tail::no_project_fails_gracefully`
1. Init workspace only (do NOT create or activate any project)
2. Run `ralph tail`
3. Assert exit code 2
4. Assert stderr or combined output contains `"active project"` (the error message from `ActiveProjectNotSet`)

## New tests in `src/validate/tests_commands.rs`

Add these to the existing `tests()` function and implement the test functions.

### `commands::config_show_global`
1. Init workspace
2. Run `ralph config show --global`
3. Assert exit code 0
4. Parse stdout as JSON (`serde_json::from_str`)
5. Assert the JSON object has `workspace`, `backends`, `workflow`, and `templates` keys

### `commands::config_show_project`
1. Init workspace, create project (e.g. "config-show-proj")
2. Run `ralph config show --project config-show-proj`
3. Assert exit code 0
4. Parse stdout as JSON
5. Assert JSON has `scope` key, where `scope.type == "project"` and `scope.project == "config-show-proj"`
6. Assert JSON has `workflow`, `templates`, and `backends` keys

### `commands::project_list_empty`
1. Init workspace (no projects created)
2. Run `ralph project list`
3. Assert exit code 0
4. Assert stdout contains `"PROJECTS IN WORKSPACE"` (the header that is always printed)
5. After the header lines, assert no project data rows appear. The simplest approach: the total line count should be small (4 or fewer lines for just headers).

### `commands::project_list_multiple`
1. Init workspace
2. Create two projects: "list-proj-a" and "list-proj-b"
3. Run `ralph project list`
4. Assert exit code 0
5. Assert stdout contains `"list-proj-a"` and `"list-proj-b"`

## New test in `src/validate/tests_run.rs`

### `run::template_fallback_when_file_missing`
1. Init workspace, create project, setup standard mock
2. Enable QA: `ralph config set workflow.qa_enabled true`
3. Delete the QA template file: `std::fs::remove_file(h.repo_root.join(".ralph/templates/qa.md"))` — this simulates a workspace initialized before QA was added
4. Run `ralph run --loops 1`
5. Assert exit code 0 (run succeeds — `render_template_with_fallback` uses the embedded default)
6. Load state and verify `state["loops"][0]["artifacts"]["qa_results"]` is a non-empty array (proving QA phase ran)
7. Verify the loop completed: `state["loops"][0]["status"] == "completed"`

## Registration

1. In `src/validate/mod.rs`:
   - Add `mod tests_tail;` alongside the existing module declarations
   - Add `tests.extend(tests_tail::tests());` to `register_tests()`

2. Wire new tests in existing modules by adding `ConformanceTest` entries to the `tests()` functions in `tests_commands.rs` and `tests_run.rs`.

## Implementation patterns

Follow existing patterns exactly:
- Each test function signature: `fn test_name(h: &RalphHarness) -> TestResult`
- Wrap body in `run_case(|| { ... })` using a local `run_case` helper (each module has its own copy — see the bottom of tests_commands.rs or tests_run.rs)
- Use `setup_with_standard_mock(h, project_id)` helper for standard setup (create one per new module, following the pattern in `tests_commands.rs`)
- Use assertions from `crate::validate::assertions`
- Import mock scripts from `crate::validate::mock_scripts::standard_mock_script`
- Test names use `module::test_name` format (e.g. `"tail::one_shot_shows_artifacts"`)

## Files to create/modify

| File | Action |
|------|--------|
| `src/validate/tests_tail.rs` | **Create** — new module with 4 tests |
| `src/validate/tests_commands.rs` | **Modify** — add 4 new tests (config_show_global, config_show_project, project_list_empty, project_list_multiple) |
| `src/validate/tests_run.rs` | **Modify** — add 1 new test (template_fallback_when_file_missing) |
| `src/validate/mod.rs` | **Modify** — register `tests_tail` module |

## Verification

1. `cargo check` — compiles without errors
2. `cargo test` — all existing tests pass
3. `nix build -L` — clean release build
4. `./result/bin/ralph validate --bin ./result/bin/ralph` — all tests pass including the new ones
