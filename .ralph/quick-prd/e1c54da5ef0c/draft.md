## Summary

Add a new conformance test module `tests_e2e.rs` to the `ralph validate` suite that exercises full daemon-orchestrator integration: from task submission through planning, implementation, QA, review, commit, completion, and PR creation. The existing test infrastructure covers daemon lifecycle (`tests_daemon.rs`), orchestrator phases (`tests_run.rs`), and QA gates (`tests_qa.rs`) independently, but no conformance tests validate the **complete daemon → orchestrator → PR** pipeline as a single flow. Additionally, the existing suites lack systematic coverage for backend failure modes (empty output, timeouts, parse-retry exhaustion), artifact parsing failures with malformed responses, and error propagation from the orchestrator back to the daemon task state.

## Acceptance Criteria

- [ ] New `tests_e2e` conformance module registered in `src/validate/mod.rs`
- [ ] `e2e::daemon_full_cycle_happy_path` — daemon start with `--single-iteration` → task claimed → child process (mock ralph via `RALPH_DAEMON_BIN`) invokes real `ralph auto` (via `h.ralph_bin` absolute path) with `auto_mock_script()` backends → quick-PRD → planning → implementation → QA pass → review approved → commit → completion → PR created → task state `completed` in `tasks.json` with `pr_url` set; mock `gh` log file captures `pr create` invocation with title, body-file path, `--head`, and `--repo` flags; `state.json` verified after completion shows `status: "completed"`, loops array has 1 entry with all artifacts populated
- [ ] `e2e::daemon_full_cycle_multi_loop` — daemon dispatches task via mock ralph that invokes real `ralph auto` (absolute path `h.ralph_bin`), counter-file-based `auto_mock_script()` variant returns `CONTINUE` on first completion call and `COMPLETE` on second, verifies two loop directories exist with correct artifacts, backends alternate between loops, and final state shows `status: "completed"` with 2 loops in `state.json`
- [ ] `e2e::backend_empty_output_fails_task` — mock ralph invokes real `ralph auto` with a backend that returns empty/whitespace-only output, task transitions to `failed` in `tasks.json`, worktree directory is preserved, no `pr_url` set, no PR created
- [ ] `e2e::backend_exit_nonzero_fails_task` — mock ralph invokes real `ralph auto` with a backend that exits non-zero, task transitions to `failed` in `tasks.json`
- [ ] `e2e::backend_timeout_exhausted_fails_task` — mock ralph invokes real `ralph auto` with a backend that sleeps beyond a short configured `backends.<name>.timeout_seconds` (e.g. 2s), verifying that `BackendTimeoutExhausted` propagates as non-zero exit from `ralph auto`, task transitions to `failed`, and stderr/log contains "timeout"
- [ ] `e2e::qa_rejection_iteration_limit_fails` — mock ralph invokes real `ralph auto` with QA-always-fail backend variant (`auto_mock_script()` with QA branch returning FAIL) and `max_qa_iterations=1`, `--loops 1`; rollback removes in-progress loop and its artifacts from `state.json`, orchestrator returns non-zero exit (no loops left to run), task transitions to `failed`
- [ ] `e2e::review_rejection_iteration_limit_fails` — mock ralph invokes real `ralph auto` with `always_reject_review` backend variant and `max_review_iterations=1`, `--loops 1`; rollback removes loop, orchestrator returns non-zero, task transitions to `failed`
- [ ] `e2e::malformed_planner_output_fails_gracefully` — `ralph run` invoked with a backend whose planner returns garbage without `# Feature:` or `# Project Completion Request` heading, verifies non-zero exit code and stderr contains a parse error diagnostic
- [ ] `e2e::malformed_reviewer_output_parse_retry` — separate backends: primary reviewer returns garbage, opposite backend (reformatter) returns valid `Review: APPROVED` output; verifies parse-retry pipeline succeeds, loop completes, review-approved artifact content originates from the reformatter backend
- [ ] `e2e::state_json_consistency_after_completion` — run a single loop to completion via `ralph run`, load `state.json`, verify: `project_id`, `project_name`, `current_loop`, `current_phase`, `status` = `"completed"`, `loops[0].status` = `"completed"`, `loops[0].artifacts.spec` (non-empty string), `loops[0].artifacts.impl_notes`, `loops[0].artifacts.approval`, `loops[0].commit` (40-char hex), `loops[0].backends.planner`, `loops[0].backends.implementer`, `loops[0].backends.reviewer`, `loops[0].started_at`, `loops[0].completed_at`
- [ ] `e2e::artifact_directory_structure` — verify `.ralph/projects/<id>/loops/001-<slug>/` contains timestamp-prefixed `*-spec.md`, `*-impl-notes.md`, `*-review-approved.md` files matching `^\d{14}-[a-z0-9-]+\.md$`, each with valid YAML frontmatter between `---` delimiters
- [ ] `e2e::impl_response_artifact_on_review_feedback` — run with a backend where reviewer returns `SUGGESTIONS` on first iteration then `APPROVED` on second, verify `*-impl-response-001.md` artifact exists with correct YAML frontmatter (`artifact: impl-response`, `iteration: 1`, `role: implementer`) and body content from implementer's response
- [ ] `e2e::pr_metadata_verification` — daemon happy-path test additionally parses mock `gh` invocation log to verify: `gh pr create` called with `--title` containing `ralph:` prefix, `--body-file` pointing to a temp file, `--head` matching the daemon branch, `--repo` matching `owner/repo`; body file contents include `Closes #<issue_number>`, diff stat section, and project ref
- [ ] All tests pass via `ralph validate --bin <path> --filter e2e` without external API keys or network access
- [ ] Tests documented with doc comments explaining the workflow being validated
- [ ] `docs/validate-e2e.md` created with test architecture overview, test-to-requirement mapping, and instructions for running the e2e validation suite

## Technical Approach

### Architecture

Create a new conformance test module `src/validate/tests_e2e.rs` following the established pattern:
- Each test is a `fn(h: &RalphHarness) -> TestResult` using `run_case(|| { ... })`
- Tests use `RalphHarness` for temp repo setup, `setup_mock_backends_stable()` for backend mocking
- Daemon tests use `ralph daemon start --single-iteration` with mock `gh` and mock `ralph` scripts via `PATH`/`RALPH_DAEMON_BIN` environment variables

### Mock ralph script design — avoiding recursion (Review Issue #8)

The mock `RALPH_DAEMON_BIN` script must invoke the real `ralph` binary by its absolute path to prevent recursive self-invocation. The mock script receives `h.ralph_bin` (the test harness's `--bin` path) baked in at generation time:

```bash
#!/bin/sh
case "$1" in
  auto)
    # Use absolute path to real ralph binary — NEVER resolve from $PATH
    exec /absolute/path/to/ralph auto "$@"
    ;;
  *)
    echo "mock ralph: unhandled: $1" >&2
    exit 1
    ;;
esac
```

A new helper `e2e_mock_ralph_script(ralph_bin: &Path)` generates this script, embedding the absolute path. The mock script:
1. Receives `auto --idea <text>` from the daemon's `spawn_ralph_auto`
2. Forwards the call to the real `ralph auto` binary (absolute path)
3. The real binary picks up mock backends from `.ralph/config.toml` (written by `setup_mock_backends_stable()`)
4. The real binary picks up mock `gh` from `PATH` (prepended by the daemon test environment)

This is distinct from the existing `daemon_mock_ralph_with_commit_script()` pattern, which fakes the entire `ralph auto` execution with direct git operations. The e2e tests need the real orchestrator to run so they can validate state transitions and artifact generation.

### Happy-path tests: daemon → orchestrator → PR pipeline

**`daemon_full_cycle_happy_path`** (Review Issues #1, #8, #9):

Uses `auto_mock_script()` (not `standard_mock_script()`) since the daemon invokes `ralph auto`, which requires quick-PRD prompt handling (spec writer, spec reviewer, spec reviser) before entering the orchestration loop. The mock ralph script invokes the real binary via absolute path (`h.ralph_bin`).

After `--single-iteration` completes:
1. Load `tasks.json`, verify task state is `completed` with `pr_url` set
2. Parse mock `gh` invocation log to verify `pr create` was called with correct arguments:
   - `--title` contains `ralph:` prefix and task ID
   - `--body-file` path exists and body content includes `Closes #<issue>`, diff stat, and project ref
   - `--head` matches daemon branch name
   - `--repo` matches `owner/repo`
3. Load `state.json`, verify `status: "completed"`, loops array has 1 entry with all artifacts populated
4. Verify loop artifact directory contains timestamp-prefixed spec, impl-notes, review-approved files with YAML frontmatter
5. Verify git tag `ralph/<project-id>/loop-1` exists

The mock `gh` script is enhanced from `daemon_mock_gh_script()` to log all invocation arguments and `--body-file` contents to a known file path for post-test assertion.

**`daemon_full_cycle_multi_loop`**: Uses a counter-file-based `auto_mock_script()` variant. The completion validator checks a counter file: first invocation returns `CONTINUE`, second returns `COMPLETE`. Mock ralph invokes real binary via absolute path. Verifies two loop directories exist, backends alternate (loop 1: claude, loop 2: codex by default), and final state shows `status: "completed"` with 2 loops.

### Failure-path tests

**Backend failures** (Review Issue #4 — reformatter scope clarification):

The reformatter fallback is **parse-error-driven**, not a generic backend execution failure fallback. When the primary backend returns output that fails parsing, the orchestrator tries the opposite backend as a reformatter (up to 3 attempts). This is distinct from:
- **Empty output**: Triggers a same-backend retry first (output < 20 chars), then if still empty/unparseable, goes to reformatter
- **Non-zero exit**: Raises `BackendCommandFailed` immediately — no reformatter fallback, the error propagates up
- **Timeout**: Raises `BackendTimeout`, retried 3 times with exponential backoff via `execute_with_timeout_retries()`, then `BackendTimeoutExhausted` propagates up — no reformatter fallback

Tests are designed to match these actual semantics:
- `backend_empty_output_fails_task`: Backend returns empty string. After same-backend retry and reformatter attempts all fail (both backends return empty), orchestrator errors out. Task fails.
- `backend_exit_nonzero_fails_task`: Backend `exit 1`. Immediate `BackendCommandFailed` error. Task fails.
- `backend_timeout_exhausted_fails_task`: Backend `sleep 30` with `backends.<name>.timeout_seconds` set to 2. After 3 timeout retries, `BackendTimeoutExhausted` propagates. Task fails.

**QA/review rejection limits** (Review Issue #2 — corrected rollback semantics):

Rollback removes the in-progress loop entirely: `state.remove_loop()` deletes it from `state.loops`, the loop directory is deleted from disk, and `current_loop` resets to `last_loop_number()`. This means:
- After rollback, the rolled-back loop does **not** persist in `state.json`
- With `--loops 1`, after rollback there are 0 completed loops and no more loops to run, so the orchestrator returns an error
- With `auto --idea` (which uses `until_complete`), the daemon would loop indefinitely since rollback resets to planning

Tests use the daemon pathway with `--loops 1` semantics by having the mock ralph script pass `--loops 1` to the real binary. This creates a bounded scenario:
- `qa_rejection_iteration_limit_fails`: QA always fails → iteration limit hit → rollback → 0 loops remain → orchestrator exits non-zero → daemon marks task `failed`
- `review_rejection_iteration_limit_fails`: Reviewer always returns SUGGESTIONS → iteration limit hit → rollback → 0 loops remain → orchestrator exits non-zero → daemon marks task `failed`

Key assertions: task state is `failed`, `state.json` shows `loops` array is empty (rolled-back loop was removed), loop directory does not exist on disk.

**Timeout test** (Review Issue #3):

`backend_timeout_exhausted_fails_task` uses a mock backend script containing `sleep 30` and configures `backends.claude.timeout_seconds` = `2` and `backends.codex.timeout_seconds` = `2` via `ralph config set`. The `execute_with_timeout_retries()` function retries 3 times with backoff (1s, 2s, 4s), then returns `BackendTimeoutExhausted`. Assertions: task `failed`, log/stderr contains "timeout".

**Malformed output / parse-retry** (Review Issue #4):

- `malformed_planner_output_fails_gracefully`: Uses `ralph run` directly (not daemon pathway) for simplicity, with a backend whose planner returns `"random garbage"`. Parser fails, reformatter also fails (same garbage from both backends since `setup_mock_backends_stable` sets both), orchestrator errors. Asserts non-zero exit and stderr diagnostic.
- `malformed_reviewer_output_parse_retry`: Uses `ralph run` with `setup_separate_mock_backends()` — primary backend's reviewer returns garbage, opposite backend returns valid `Review: APPROVED`. Parse-retry pipeline calls reformatter (opposite backend), which succeeds. Loop completes. Review-approved artifact content matches the reformatter's output.

### State validation strategy (Review Issue #5)

Final-state validation is the primary mechanism since conformance tests run the binary as a subprocess and cannot hook into intermediate phase transitions. The `state_json_consistency_after_completion` test performs comprehensive field-by-field verification of the final `state.json` as a regression guard.

For intermediate-state validation, the `daemon_full_cycle_happy_path` test inspects `state.json` **after** the daemon completes (which is after all child processes finish). Since the daemon runs `--single-iteration`, the child `ralph auto` runs to completion synchronously before the daemon exits. The state at daemon exit reflects the final orchestrator state. True mid-phase snapshots would require instrumenting the orchestrator binary, which is out of scope for conformance tests — the existing `tests_run.rs` unit tests cover phase-by-phase state transitions.

### Implementation response artifact coverage (Review Issue #6)

`impl_response_artifact_on_review_feedback` uses `ralph run` with a backend where:
- Reviewer returns `SUGGESTIONS` with feedback on iteration 1
- Implementer responds to feedback (produces `ImplementerDecision::Response`)
- Reviewer returns `APPROVED` on iteration 2

After completion, verify:
- `*-impl-response-001.md` exists in the loop directory
- YAML frontmatter contains `artifact: impl-response`, `iteration: 1`, `role: implementer`
- Body contains the implementation response content
- `*-review-approved.md` also exists (from the final approval)

This uses `setup_separate_mock_backends()` with a stateful mock script that tracks review iteration via a counter file.

### PR metadata verification (Review Issue #9)

The mock `gh` script is enhanced to capture full invocation details. A new `e2e_mock_gh_logging_script()` function generates a mock that:
1. Handles all standard `gh` subcommands (issue list/edit/view/comment, pr list/create)
2. On `pr create`: logs the full argument list to `$MOCK_GH_LOG` file, copies the `--body-file` content to `$MOCK_GH_BODY_LOG`
3. Returns the standard mock PR URL

Post-test assertions parse the log files to verify:
- `--title` argument contains `ralph:` prefix
- `--body-file` content includes `Closes #<N>`, a diff stat section, and a project ref line
- `--head` argument matches the expected daemon branch
- `--repo` argument matches `owner/repo` from the mock issue

### Mock script composition

New mock scripts are defined as functions in `tests_e2e.rs` (private to the module):
- `e2e_mock_ralph_script(ralph_bin: &Path) -> String` — wrapper that invokes real binary via absolute path
- `e2e_mock_ralph_run_script(ralph_bin: &Path, extra_args: &[&str]) -> String` — variant that passes `--loops 1` or other flags
- `e2e_mock_gh_logging_script() -> String` — enhanced mock gh with argument/body logging
- `e2e_auto_mock_qa_always_fail() -> String` — `auto_mock_script()` variant with QA returning FAIL
- `e2e_auto_mock_review_always_reject() -> String` — `auto_mock_script()` variant with reviewer returning SUGGESTIONS
- `e2e_auto_mock_multi_loop() -> String` — counter-file completion validator variant
- `e2e_timeout_backend_script() -> String` — `sleep 30` script for timeout testing
- `e2e_garbage_planner_script() -> String` — returns unparseable output
- `e2e_reviewer_garbage_primary_script() -> String` / `e2e_reviewer_valid_reformatter_script() -> String` — split scripts for parse-retry test
- `e2e_review_then_approve_script() -> String` — stateful reviewer for impl-response test

Reuse from `mock_scripts.rs`: `auto_mock_script()`, `daemon_mock_gh_script()` (as base for logging variant), `standard_mock_script()` where `ralph run` tests don't need quick-PRD handling.

## Files & Modules

| File | Action | Purpose |
|------|--------|---------|
| `src/validate/tests_e2e.rs` | **Create** | New conformance test module with ~14 tests |
| `src/validate/mod.rs` | **Edit** | Add `mod tests_e2e;` declaration and `tests.extend(tests_e2e::tests())` in `register_tests()` |
| `docs/validate-e2e.md` | **Create** | Test architecture overview, test-to-requirement mapping table, instructions for running e2e suite, troubleshooting guide |

No other source files need modification. All new tests use the existing harness, assertion, and mock_scripts infrastructure.

## Testing Strategy

### Running the new tests

```bash
# Build and run only e2e tests
cargo build --release
./target/release/ralph validate --bin ./target/release/ralph --filter e2e

# Run with verbose output for debugging
./target/release/ralph validate --bin ./target/release/ralph --filter e2e --verbose

# List e2e tests without executing
./target/release/ralph validate --bin ./target/release/ralph --filter e2e --list
```

### CI integration

The tests require no external API keys, network access, or running services. They use:
- `tempfile::TempDir` for isolated temp directories
- Bash mock scripts for `gh`, `ralph`, and backend CLIs
- Environment variable injection (`PATH`, `RALPH_DAEMON_BIN`) for executable substitution
- Backend timeout configured to 2s for the timeout test (default 7200s would cause CI hangs)

Existing CI that runs `ralph validate --bin <path>` will automatically pick up the new tests via `register_tests()`.

### Test isolation guarantees

- Each test gets a fresh `RalphHarness` (new temp dir, fresh git repo)
- Mock scripts are written per-test into the temp dir
- No shared state between tests
- Counter files (for stateful mock behavior) are created in the test's `temp_dir`
- Mock `gh` log files are written to test-specific paths within `temp_dir`

### Verification approach per test

| Test | Key assertions |
|------|---------------|
| `daemon_full_cycle_happy_path` | task state `completed`, `pr_url` set, `state.json` status `completed`, loop artifacts exist with YAML frontmatter, git tag exists, mock gh log shows `pr create` with correct `--title`/`--head`/`--repo` |
| `daemon_full_cycle_multi_loop` | 2 loops in state, backend alternation (claude/codex), both loop dirs populated, PR created |
| `backend_empty_output_fails_task` | task state `failed`, no `pr_url`, worktree directory exists |
| `backend_exit_nonzero_fails_task` | task state `failed` |
| `backend_timeout_exhausted_fails_task` | task state `failed`, log contains "timeout" |
| `qa_rejection_iteration_limit_fails` | task state `failed`, `state.json` loops array empty (rolled-back), loop directory absent |
| `review_rejection_iteration_limit_fails` | task state `failed`, `state.json` loops array empty (rolled-back) |
| `malformed_planner_output_fails_gracefully` | `ralph run` exits non-zero, stderr contains parse error |
| `malformed_reviewer_output_parse_retry` | loop completes, review-approved artifact content matches reformatter output |
| `state_json_consistency_after_completion` | All `ProjectState` fields present and correctly typed: `project_id`, `project_name`, `current_loop`, `current_phase`, `status`, `loops[0].*` (status, artifacts, commit, backends, timestamps) |
| `artifact_directory_structure` | Directory naming `NNN-slug`, file naming matches `^\d{14}-[a-z0-9-]+\.md$`, YAML frontmatter valid |
| `impl_response_artifact_on_review_feedback` | `*-impl-response-001.md` exists, YAML frontmatter has `artifact: impl-response`, `iteration: 1`, `role: implementer` |
| `pr_metadata_verification` | Mock gh log: `--title` has `ralph:` prefix, body-file includes `Closes #N`, diff stat, project ref; `--head` and `--repo` match expected values |

### Documentation (Review Issue #7)

`docs/validate-e2e.md` provides:
1. **Architecture overview**: Diagram of daemon → mock ralph → real binary → mock backends → mock gh pipeline
2. **Test-to-requirement mapping**: Table linking each test to its acceptance criteria and the original feature requirements
3. **Running instructions**: Build commands, filter syntax, verbose mode, listing tests
4. **Troubleshooting guide**: Common failure modes (mock script permission issues, PATH ordering, timeout flakiness), debugging with `--verbose` and log file inspection
5. **Adding new tests**: Template and conventions for adding e2e tests

## Out of Scope

- **Performance benchmarking** — tests validate correctness, not execution time
- **Real backend integration** — all backends are mocked; real Claude/Codex API testing is not part of the validate suite
- **Network-dependent scenarios** — no tests for real GitHub API failures, rate limiting, or auth issues
- **Parallel/concurrent daemon dispatch** — tests use `--single-iteration` mode; multi-task concurrent dispatch is not covered
- **Mid-phase state snapshots** — intermediate `state.json` inspection during orchestrator execution requires binary instrumentation; only final state is validated (existing `tests_run.rs` covers per-phase transitions)
- **Tmux integration** — tmux-based backend execution is tested separately in `tests_tail.rs`
- **MCP handler testing** — already covered by `tests_mcp.rs`
- **Prompt review workflow** — already covered by `tests_prompt_review.rs`
- **Auto-rebase logic** — already covered by the rebase tests in `tests_daemon.rs`
- **Modifications to existing test modules** — no changes to `tests_daemon.rs`, `tests_run.rs`, `tests_qa.rs`, or other existing modules
- **Changes to the `mock_scripts.rs` shared module** — new mock scripts are scoped to `tests_e2e.rs`
- **Generic backend failure reformatter fallback** — the reformatter is parse-error-driven only; backend execution failures (non-zero exit, timeout) propagate directly without reformatter involvement, matching current orchestrator semantics
