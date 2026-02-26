The revised specification has been written. Here's a summary of how each review issue was addressed:

### Issue 1: Path Derivation (loop_slug unavailability)
Log paths now use only `loop_number` and `role` — **not** `loop_slug` or backend name. The planner phase (where slug doesn't yet exist) uses `loops/001/agent-output-planner.log`. The prompt-reviewer (no loop number at all) uses `{project_dir}/agent-output-prompt-reviewer.log`. This makes paths deterministic and available at all 9 call sites.

### Issue 2: Retry Attribution
Logs are keyed on `(loop_number, role)`, and the file is opened in **append mode**. All retry attempts within `execute_with_parse_retries` — including reformatter fallbacks to the opposite backend — append to the same file with attempt separators showing which backend produced each section. No overwriting or misattribution.

### Issue 3: Streaming Semantics
Replaced line-by-line `read_line()` with chunk-based `read_buf()` using `BytesMut`. This preserves exact bytes including partial lines, carriage returns, and progress indicators. In-memory accumulation uses `Vec<u8>` with `String::from_utf8_lossy()` applied only at the end, matching current behavior. Explicit acceptance criterion added that returned output is byte-identical to `wait_with_output()`.

### Issue 4: Timeout and Process Cleanup
Spec now requires explicit `child.kill()` + `child.wait()` on timeout (and I/O errors). The streaming loop function takes stdout/stderr handles as parameters so the `child` handle stays in the outer scope for kill/reap. A `Timed out` footer is written to the log before returning `BackendTimeout`. A dedicated test (`cli_backend_timeout_kills_child_and_writes_footer`) verifies the child process is actually dead.

### Issue 5: File Path Safety and I/O Policy
Added `sanitize_for_filename()` utility for backend names in log headers/separators (parentheses, slashes stripped). Log filenames themselves use only the role string which is always safe ASCII. Log I/O failures are best-effort: `tracing::warn!` logged, file handle set to `None`, execution continues unaffected. Explicit acceptance criteria and a test case for write-failure resilience.

### Issue 6: Test Coverage Completeness
Added 4 conformance tests under `src/validate/tests_streaming.rs` including:
- **Mid-execution visibility test** (log file readable while backend still running)
- **Retry/append behavior test** (retry appends with separators)
- **Timeout cleanup test** (timed-out footer + partial output)
- **Prompt-reviewer path test** (correct placement at project root)

Also added unit tests for CR/partial-line preservation, log write failure resilience, sanitization, and child process reaping verification.