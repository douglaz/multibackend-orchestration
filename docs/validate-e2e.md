# Validate E2E Conformance

## Architecture overview
The validate E2E conformance tests run the compiled `ralph` binary in isolated temporary git repositories via `RalphHarness`, with deterministic mock backends and mock `gh` scripts. Tests exercise real CLI entry points (`run`, `auto`, `daemon start`) and assert persisted state (`.ralph/projects/*/state.json`, `.ralph/daemon/tasks.json`), loop artifacts, and command/PR side effects.

## Test-to-requirement mapping
| Normative requirement | Conformance test(s) |
| --- | --- |
| 1. `ralph auto` happy-path tests must use `auto_mock_script()` | `e2e_conformance::pr_metadata_verification` (daemon-dispatched real `ralph auto` via `e2e_mock_ralph_script()` and `auto_mock_script()` backends), `auto_init::*` tests in `tests_auto_init.rs` |
| 2. QA/review rollback failure removes loop and fails with `--loops 1` | `qa::iteration_limit_fails`, `run::review_limit_fails` |
| 3. Timeout surfaces `BackendTimeoutExhausted` and task/project fails | `e2e_conformance::backend_timeout_exhausted_fails_task` |
| 4. Reformatter boundaries (parse-error only; non-zero/timeout no reformatter; empty output retry then reformatter) | `e2e_conformance::backend_command_failed_no_reformatter`, `e2e_conformance::backend_timeout_exhausted_fails_task`, `e2e_conformance::empty_output_retries_then_reformatter` |
| 5. Review-feedback path writes `*-impl-response-001.md` with required frontmatter/body | `run::impl_response_artifact_on_review_feedback` |
| 6. PR metadata includes `ralph:` title prefix, `--body-file` body requirements, `--head`, `--repo` | `e2e_conformance::pr_metadata_verification` |

## Running instructions
1. Build and type-check:
   `nix develop -c cargo check`
2. Run unit/integration tests:
   `nix develop -c cargo test`
3. Build release binary:
   `nix build -L`
4. Run full conformance suite:
   `./result/bin/ralph validate --bin ./result/bin/ralph`

## Troubleshooting
- If daemon PR tests do not create a PR, verify the test sets an `origin` remote and that mock `gh` is first in `PATH`.
- If backend scripts fail only in Nix environments, prefer `setup_mock_backends_stable()` so wrappers use `/bin/sh` + `bash` and clear default backend args.
- If reformatter-semantics assertions fail, inspect stderr for `requesting reformat via` and inspect mock backend invocation logs (`RALPH_VALIDATE_BACKEND_LOG`).
- If tests appear stuck, confirm the mock auto backend can reach completion deterministically (for E2E, set `RALPH_COMPLETE=yes` and `RALPH_E2E_FORCE_FEATURE=yes`).

## Adding-new-tests template
1. Add a `ConformanceTest` entry in the relevant `src/validate/tests_*.rs` module `tests()` list.
2. Implement `fn <name>(h: &RalphHarness) -> TestResult` wrapped with `run_case(|| { ... })`.
3. Use harness helpers to initialize workspace, configure mock backends/scripts, and execute real CLI commands.
4. Assert user-visible behavior (exit code, stderr/stdout), persisted state files, and artifact naming/content.
5. If external-process behavior is required, add a deterministic helper in `src/validate/mock_scripts.rs` and log invocations for explicit assertions.
