The revised specification has been written to `SPEC.md`. Here's a summary of how each review issue was addressed:

**Issue 1 — Same-loop rollback semantics**: Added explicit handling for `marker_loop == checkpoint_loop` with non-Planning phase. Acceptance criteria #3 now has three sub-bullets covering `<`, `==` (non-Planning), and `>=` (no-op) cases. The technical approach §2 includes the `else if marker_loop == checkpoint_loop && checkpoint_phase != Phase::Planning` branch.

**Issue 2 — Malformed marker robustness**: Acceptance criteria #4 explicitly requires malformed markers to be non-fatal (warn via `eprintln!`, ignore). The technical approach §2 shows the `Err(_)` arm logging a warning and continuing.

**Issue 3 — `original_head` capture order**: Acceptance criteria #7 specifies the anchor is captured "**after** switching to the project branch". Technical approach §4 step 3 explicitly captures `original_head` after the checkout in step 2, with a note explaining why (so the anchor refers to the project branch HEAD, not some other branch).

**Issue 4 — Push-failure exit code and contract**: Acceptance criteria #7 specifies non-zero exit code and the exact user-facing warning message. Technical approach §4 shows the `push_failed` flag pattern with `return Err(RalphError::Orchestration(...))` to ensure non-zero exit. The error message is deterministic and actionable.

**Issue 5 — Marker clearing timing**: Acceptance criteria #11 explicitly states the marker is cleared "only after a successful `commit_and_push_phase_transition`". Technical approach §6 places the `fs::remove_file` after the `?` propagation point on `commit_and_push_phase_transition`, with a comment explaining that failed commits/pushes propagate the error and never reach the deletion line.

**Issue 6 — Branch existence edge case**: Acceptance criteria #6 adds a new requirement: create from remote if local missing, fail fast if neither exists. Technical approach §3 replaces the silent skip with a three-way branch: local exists → checkout; remote exists → create+checkout; neither → error before any destructive operation.

**Issue 7 — Validate conformance test coverage**: Testing strategy now specifies 8 new conformance tests in `src/validate/tests_commands.rs` (not unit tests), following the established `ConformanceTest` / `run_case` / `RalphHarness` pattern. Tests cover: soft rollback no-git-mutation, marker writing, marker deletion on hard, dry-run messages (both modes), marker-based reconstruction capping, and push-failure recovery with revert.