Now I have all the context I need. Let me produce the updated specification addressing all three review issues.

Here is the revised specification:

---

## Summary

Add a `--version` / `-V` flag to the `ralph` CLI by applying Clap's built-in `#[command(version)]` attribute to the existing `Cli` struct. This is a one-line change. Clap's derive macro will automatically pull the version string from `Cargo.toml` via the `CARGO_PKG_VERSION` environment variable set at compile time.

## Acceptance Criteria

- `ralph --version` prints `ralph 0.1.0` (or current `Cargo.toml` version) and exits with code 0
- `ralph -V` works identically as the short form, producing the same output as `--version`
- Version string is sourced from `Cargo.toml` package metadata at compile time — no hardcoded strings
- Implementation uses Clap's built-in `#[command(version)]` attribute
- `--version` and `-V` work without an initialized workspace or active project (global flag behavior; Clap exits before command dispatch)

## Technical Approach

Add a single `#[command(version)]` attribute to the `Cli` struct in `src/cli/mod.rs`:

```rust
#[derive(Debug, Parser)]
#[command(name = "ralph")]
#[command(about = "AI backend orchestration tool")]
#[command(version)]            // <-- add this line
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}
```

**How it works:** When `#[command(version)]` is present with no explicit value, Clap's derive macro expands to use the `env!("CARGO_PKG_VERSION")` compile-time constant, which Cargo populates from the `version` field in `Cargo.toml`. Clap automatically registers `--version` / `-V` and handles printing + early exit before subcommand parsing or workspace resolution. No other code changes are needed.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/mod.rs:23-25` | Add `#[command(version)]` attribute to `Cli` struct |
| `src/validate/tests_commands.rs` | Add `version_long_flag`, `version_short_flag`, and `version_no_workspace` conformance tests |
| `src/validate/assertions.rs` | Add `assert_stdout_eq` helper for exact-match output assertions |

No new files, no new dependencies, no changes to `Cargo.toml`.

## Testing Strategy

All version-flag tests are conformance tests in `src/validate/tests_commands.rs`, registered via `tests_commands::tests()` which is already wired into `register_tests()` in `src/validate/mod.rs`. No registration change in `mod.rs` is needed — only the `tests()` vec in `tests_commands.rs` gains new entries.

### 1. New assertion helper

Add `assert_stdout_eq` in `src/validate/assertions.rs` to compare trimmed stdout output for exact equality (not substring), preventing false positives from partial matches:

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

### 2. Conformance tests

Three new entries in the `tests()` vec inside `tests_commands.rs`:

```rust
ConformanceTest {
    name: "commands::version_long_flag",
    func: version_long_flag,
},
ConformanceTest {
    name: "commands::version_short_flag",
    func: version_short_flag,
},
ConformanceTest {
    name: "commands::version_no_workspace",
    func: version_no_workspace,
},
```

**Test implementations** (all use the existing `run_case` / `panic_message` pattern from `tests_commands.rs`):

```rust
/// `ralph --version` exits 0 and prints exactly `ralph <CARGO_PKG_VERSION>`.
fn version_long_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let output = h.ralph(["--version"]).expect("ralph --version should run");
        assert_exit_code(&output, 0);
        let expected = format!("ralph {}", env!("CARGO_PKG_VERSION"));
        assert_stdout_eq(&output, &expected);
    })
}

/// `ralph -V` exits 0 and prints the same output as `--version`.
fn version_short_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let long = h.ralph(["--version"]).expect("ralph --version should run");
        let short = h.ralph(["-V"]).expect("ralph -V should run");
        assert_exit_code(&short, 0);
        // Assert exact parity: both forms must produce identical output.
        let long_out = String::from_utf8_lossy(&long.stdout).trim().to_owned();
        let short_out = String::from_utf8_lossy(&short.stdout).trim().to_owned();
        assert_eq!(long_out, short_out, "-V and --version output must match");
    })
}

/// `ralph --version` succeeds even without an initialized workspace,
/// verifying it is a global flag handled by Clap before command dispatch.
fn version_no_workspace(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Do NOT call h.init_workspace() — the harness provides a bare
        // git repo with no .ralph/ directory, which is exactly the
        // uninitialised-workspace scenario.
        let output = h.ralph(["--version"]).expect("ralph --version should run");
        assert_exit_code(&output, 0);
        let expected = format!("ralph {}", env!("CARGO_PKG_VERSION"));
        assert_stdout_eq(&output, &expected);
    })
}
```

**Key design decisions:**

- `version_long_flag` uses `assert_stdout_eq` (exact equality, not `contains`) against `"ralph {CARGO_PKG_VERSION}"` — this ensures the output format is precisely correct and prevents false positives.
- `version_short_flag` asserts exit code 0 **and** parity with `--version` output, fully satisfying the acceptance criterion that both forms work identically.
- `version_no_workspace` deliberately skips `init_workspace()` to prove the flag works without a `.ralph/` directory. The `RalphHarness` sets `current_dir` to a bare git repo, so this faithfully simulates a user running `ralph --version` outside any project.
- All tests use `env!("CARGO_PKG_VERSION")` so the expected version is always derived from `Cargo.toml` at compile time — no hardcoded version strings in tests.

### 3. Build verification

Run `cargo build` to confirm the new attribute compiles cleanly.

## Out of Scope

- Changing the version value in `Cargo.toml` (that's a release concern)
- Adding build metadata, git SHA, or extended version info (e.g., `--version verbose`)
- Modifying the workspace config version (`workspace.version`) — that's an unrelated concept
- Propagating `--version` to individual subcommands