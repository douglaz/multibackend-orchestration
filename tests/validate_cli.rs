//! Regression test for `ralph validate --bin` path resolution.
//!
//! Verifies that relative `--bin` paths are resolved to absolute paths before
//! test execution, so per-test harness cwd changes do not break binary lookup.

use std::env;
use std::io::Write;
use std::path::PathBuf;

use ralph::validate::{ValidateArgs, execute};

/// Locate the built `ralph` binary via the CARGO_BIN_EXE mechanism or
/// fall back to building the path from `CARGO_MANIFEST_DIR`.
fn ralph_bin_absolute() -> PathBuf {
    // cargo sets CARGO_BIN_EXE_ralph when running integration tests after build
    if let Ok(p) = env::var("CARGO_BIN_EXE_ralph") {
        return PathBuf::from(p);
    }

    // Fallback: derive from target directory
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set");
    let target_dir = PathBuf::from(manifest).join("target").join("debug").join("ralph");
    assert!(
        target_dir.exists(),
        "ralph binary not found at {target_dir:?}; run `cargo build` first"
    );
    target_dir
}

/// Build a relative path from `cwd` to `target` by stripping the common prefix.
fn make_relative(target: &PathBuf, cwd: &PathBuf) -> PathBuf {
    // Find common prefix length
    let target_components: Vec<_> = target.components().collect();
    let cwd_components: Vec<_> = cwd.components().collect();

    let common = target_components
        .iter()
        .zip(cwd_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Build relative path: "../" for each remaining cwd component, then the target suffix
    let mut relative = PathBuf::new();
    for _ in common..cwd_components.len() {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component);
    }
    relative
}

#[test]
fn validate_relative_bin_resolves_correctly() {
    let abs_bin = ralph_bin_absolute();
    assert!(
        abs_bin.is_absolute(),
        "expected absolute path for ralph binary, got {abs_bin:?}"
    );

    // Construct a relative path to the same binary from the current directory.
    let cwd = env::current_dir().expect("current_dir should succeed");
    let relative_bin = make_relative(&abs_bin, &cwd);

    assert!(
        !relative_bin.is_absolute(),
        "expected relative path, got {relative_bin:?}"
    );

    // Execute validate with the relative --bin path and a narrow filter so the
    // harness actually runs a test from a different cwd (temp dir).  Using
    // `list: false` ensures TestRunner creates a RalphHarness per test, which
    // changes cwd — this is the exact code path that would break without
    // canonicalization of the --bin path.
    let args = ValidateArgs {
        bin: relative_bin,
        filter: Some("init::creates_workspace_structure".to_string()),
        list: false,
        verbose: false,
    };

    let result = execute(args);
    assert!(
        result.is_ok(),
        "validate with relative --bin should succeed when running a real test (got: {result:?})"
    );
}

#[test]
fn validate_nonexistent_bin_fails_with_error() {
    let args = ValidateArgs {
        bin: PathBuf::from("/nonexistent/path/to/ralph"),
        filter: None,
        list: true,
        verbose: false,
    };

    let result = execute(args);
    assert!(
        result.is_err(),
        "validate with nonexistent --bin should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("does not exist") || err_msg.contains("not accessible"),
        "error should mention path issue, got: {err_msg}"
    );
}

#[test]
fn validate_non_executable_bin_fails_with_error() {
    // Create a real file that exists but is not executable (mode 0o644).
    let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
    // Write some content so the file is non-empty.
    writeln!(tmp.as_file(), "not a binary").unwrap();

    // Ensure the file is NOT executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        std::fs::set_permissions(tmp.path(), perms).expect("failed to set permissions");
    }

    let args = ValidateArgs {
        bin: tmp.path().to_path_buf(),
        filter: None,
        list: true,
        verbose: false,
    };

    let result = execute(args);
    assert!(
        result.is_err(),
        "validate with non-executable --bin should fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not executable"),
        "error should mention 'not executable', got: {err_msg}"
    );
}
