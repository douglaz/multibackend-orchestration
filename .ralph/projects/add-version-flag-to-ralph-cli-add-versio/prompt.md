### Feature
Add global CLI version support to `ralph` using Clap’s built-in version handling.

### Goal
`ralph --version` and `ralph -V` must print `ralph <Cargo.toml version>` and exit with code `0`, without requiring an initialized workspace or active project.

### Required Changes

1. Update CLI metadata in `src/cli/mod.rs`.
- Add `#[command(version)]` to the existing `Cli` derive attributes.
- Do not hardcode a version string.

2. Add conformance tests in `src/validate/tests_commands.rs`.
- Add these entries to `tests()`:
  - `commands::version_long_flag`
  - `commands::version_short_flag`
  - `commands::version_no_workspace`
- Implement all three using existing `run_case` and `RalphHarness` patterns.

3. Add exact stdout assertion helper in `src/validate/assertions.rs`.
- Add:
```rust
pub fn assert_stdout_eq(output: &Output, expected: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        expected.trim(),
        "expected stdout to equal '{}', got:\n{}",
        expected.trim(),
        stdout
    );
}
```
- Import this helper in `tests_commands.rs` and use it for version-output checks.

### Acceptance Criteria

- `ralph --version` exits `0` and prints `ralph <current Cargo.toml package version>`.
- `ralph -V` exits `0` and produces the same output as `--version`.
- Expected output in tests is derived from `format!("ralph {}", env!("CARGO_PKG_VERSION"))`.
- `--version` works when no `.ralph/` workspace exists (global Clap flag behavior).
- Implementation uses Clap `#[command(version)]` on `Cli`.

### Test Cases

1. `version_long_flag`
- Run `h.ralph(["--version"])`.
- Assert exit code `0`.
- Assert stdout equals `format!("ralph {}", env!("CARGO_PKG_VERSION"))` (trimmed).

2. `version_short_flag`
- Run `h.ralph(["--version"])` and `h.ralph(["-V"])`.
- Assert `-V` exit code `0`.
- Assert trimmed stdout from both calls is identical.

3. `version_no_workspace`
- Do not call `h.init_workspace()`.
- Run `h.ralph(["--version"])`.
- Assert exit code `0`.
- Assert stdout equals `format!("ralph {}", env!("CARGO_PKG_VERSION"))` (trimmed).

### Validation Commands

```bash
nix develop -c cargo check
nix build -L
./result/bin/ralph validate --bin ./result/bin/ralph --filter commands::version_
```

### Constraints

- No new dependencies.
- No new files.
- No `Cargo.toml` changes.
- No `src/validate/mod.rs` registration changes required (tests are already sourced via `tests_commands::tests()`).

### Assumption

- The binary passed to `--bin` is built from the same source revision as the tests, so `env!("CARGO_PKG_VERSION")` matches runtime version output.