Here is the engineering specification:

---

## Summary

Add a `--version` / `-V` flag to the `ralph` CLI by applying Clap's built-in `#[command(version)]` attribute to the existing `Cli` struct. This is a one-line change. Clap's derive macro will automatically pull the version string from `Cargo.toml` via the `CARGO_PKG_VERSION` environment variable set at compile time.

## Acceptance Criteria

- `ralph --version` prints `ralph 0.1.0` (or current `Cargo.toml` version) and exits with code 0
- `ralph -V` works identically as the short form
- Version string is sourced from `Cargo.toml` package metadata at compile time — no hardcoded strings
- Implementation uses Clap's built-in `#[command(version)]` attribute

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

**How it works:** When `#[command(version)]` is present with no explicit value, Clap's derive macro expands to use the `env!("CARGO_PKG_VERSION")` compile-time constant, which Cargo populates from the `version` field in `Cargo.toml`. Clap automatically registers `--version` / `-V` and handles printing + early exit. No other code changes are needed.

## Files & Modules

| File | Change |
|---|---|
| `src/cli/mod.rs:23-25` | Add `#[command(version)]` attribute to `Cli` struct |

No new files, no new dependencies, no changes to `Cargo.toml`.

## Testing Strategy

1. **Manual smoke test:** Run `cargo run -- --version` and `cargo run -- -V`; verify output matches `ralph <version>` where `<version>` equals the `version` field in `Cargo.toml`.
2. **Build test:** Run `cargo build` to confirm compilation succeeds with the new attribute.
3. **Automated test (optional but recommended):** Add an integration test in `tests/` that invokes the binary with `--version` via `std::process::Command` and asserts:
   - Exit code is 0
   - Stdout contains the version from `env!("CARGO_PKG_VERSION")`

Example:

```rust
#[test]
fn cli_version_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ralph"))
        .arg("--version")
        .output()
        .expect("failed to run ralph");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}
```

## Out of Scope

- Changing the version value in `Cargo.toml` (that's a release concern)
- Adding build metadata, git SHA, or extended version info (e.g., `--version verbose`)
- Modifying the workspace config version (`workspace.version`) — that's an unrelated concept
- Propagating `--version` to individual subcommands