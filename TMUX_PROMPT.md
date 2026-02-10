# Observable Backend Execution via tmux

## Overview

Add the ability to run AI backend commands (claude, codex) inside tmux windows so users can watch agents work in real-time, while ralph still captures their output for parsing and orchestration.

## Current Behavior

Backend commands are executed as subprocesses with piped stdin/stdout/stderr (`src/backend/mod.rs`). All output is captured programmatically but invisible to the user during execution. The only visibility is ralph's INFO-level log lines ("invoking implementer...").

## Desired Behavior

When tmux mode is enabled:

1. Ralph creates (or reuses) a named tmux session (e.g., `ralph`)
2. Each backend invocation runs in a dedicated tmux window within that session
3. The user can `tmux attach -t ralph` to watch agents work live — they see the full interactive terminal output (thinking, tool calls, progress)
4. Ralph still captures the final response (stdout) for parsing, exactly as it does today
5. Windows are labeled with context (e.g., `L3-impl-codex` for loop 3 implementer on codex)
6. Completed windows are kept briefly for inspection, then cleaned up

## Technical Approach

### TmuxBackend wrapper

Create a `TmuxBackend` struct that wraps a `CliBackend` and implements the `Backend` trait. Instead of spawning the command directly with piped stdio, it:

1. Writes the prompt to a temp file (e.g., `/tmp/ralph-<session>-<id>-prompt.txt`)
2. Creates a tmux window running a shell command like:
   ```
   cat /tmp/ralph-...-prompt.txt | <backend-command> <args> > /tmp/ralph-...-output.txt 2>&1; echo $? > /tmp/ralph-...-exit.txt
   ```
   Note: stderr should go to the terminal (not redirected) so it's visible in the tmux pane. Only stdout is captured to the output file. The actual command should be:
   ```
   cat /tmp/ralph-...-prompt.txt | <backend-command> <args> > /tmp/ralph-...-output.txt; echo $? > /tmp/ralph-...-exit.txt
   ```
3. Polls for the exit file to appear (indicating the command finished)
4. Reads the output file and returns it as the response
5. Respects the configured timeout

### Session management

- On first backend invocation, create the tmux session if it doesn't exist (`tmux new-session -d -s ralph`)
- Use `tmux new-window -t ralph -n <label>` for each invocation
- After reading output, optionally keep the window for N seconds before killing it
- Use `tmux has-session` to check session existence

### Configuration

In `ralph.toml`:
```toml
[workspace]
tmux = false              # enable/disable tmux mode
tmux_session = "ralph"    # session name
```

CLI override:
```
ralph run --tmux          # enable for this run
ralph run --no-tmux       # disable for this run
```

### Backend registry integration

In the orchestrator's backend setup code (`build_backend_registry` or equivalent), when tmux mode is enabled, wrap each `CliBackend` in a `TmuxBackend` before registering it. This keeps the change minimal — the rest of the orchestrator is unaware of tmux.

## Features (suggested loop decomposition)

### Feature 1: tmux session and window management
- Add a `tmux` module (`src/tmux.rs` or `src/backend/tmux.rs`)
- Functions: `ensure_session(name)`, `create_window(session, label, command)`, `wait_for_exit(exit_file, timeout)`, `kill_window(session, window_id)`
- Check tmux availability, fail gracefully with clear error if not installed
- Unit tests using mock/stub approach (don't require actual tmux in test environment)

### Feature 2: TmuxBackend implementation
- `TmuxBackend` struct wrapping `CliBackend`, implementing `Backend` trait
- Handles prompt writing, tmux window creation, output reading, cleanup
- Temp file management with proper cleanup on success and failure
- Integration with existing timeout mechanism

### Feature 3: Configuration and CLI wiring
- Add `tmux` and `tmux_session` fields to workspace config (`src/config.rs`)
- Add `--tmux` / `--no-tmux` CLI flags (`src/cli.rs` or equivalent)
- In the orchestrator, wrap backends in `TmuxBackend` when tmux mode is active
- Ensure non-tmux mode is completely unchanged (no regressions)

### Feature 4: Window labeling and UX polish
- Generate descriptive window names: `L{loop}-{role}-{backend}` (e.g., `L3-impl-codex`)
- Keep completed windows for a configurable duration before cleanup
- Handle edge cases: tmux session killed externally, window closed mid-execution
- Add `ralph tail --tmux` or similar to auto-attach to the ralph tmux session

## Constraints

- Must not break existing non-tmux execution — tmux mode is strictly opt-in
- If tmux is not installed and tmux mode is requested, fail with a clear error message
- All existing tests must continue to pass without modification
- New tests should not require tmux to be installed (use mocks/stubs)
- The `Backend` trait interface must not change
- Temp files must be cleaned up reliably (use RAII or cleanup-on-drop patterns)

## Language and Stack

- Rust (this is a Rust project using tokio for async)
- The tmux interaction is via shell commands (`tmux` CLI), not a library
- Tests use `#[tokio::test]` and are run via `nix build` in a sandbox
