//! Regression test for `ralph validate --bin` path resolution.
//!
//! Verifies that relative `--bin` paths are resolved to absolute paths before
//! test execution, so per-test harness cwd changes do not break binary lookup.

use std::env;
use std::io::Write;
use std::path::PathBuf;

use ralph::validate::{execute, ValidateArgs};

/// Locate the built `ralph` binary via the CARGO_BIN_EXE mechanism or
/// fall back to probing common target directory layouts.
fn ralph_bin_absolute() -> PathBuf {
    // cargo sets CARGO_BIN_EXE_ralph when running integration tests after build
    if let Ok(p) = env::var("CARGO_BIN_EXE_ralph") {
        return PathBuf::from(p);
    }

    // Fallback: try common target directory layouts (debug, release, Nix store)
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set");
    let manifest_path = PathBuf::from(&manifest);

    // Collect target dirs to search: local `target/` and CARGO_TARGET_DIR if set
    let mut target_roots = vec![manifest_path.join("target")];
    if let Ok(target_dir) = env::var("CARGO_TARGET_DIR") {
        target_roots.push(PathBuf::from(target_dir));
    }

    let profiles = &["debug", "release"];

    for target_root in &target_roots {
        // Direct profile layout: target/{debug,release}/ralph
        for profile in profiles {
            let candidate = target_root.join(profile).join("ralph");
            if candidate.exists() {
                return candidate;
            }
        }

        // Target-triple layout: target/<triple>/{debug,release}/ralph
        // This covers Nix builds that use e.g. target/x86_64-unknown-linux-gnu/release/
        if let Ok(entries) = std::fs::read_dir(target_root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Target triples contain at least two hyphens (arch-vendor-os or arch-vendor-os-env)
                if name_str.matches('-').count() >= 2 {
                    for profile in profiles {
                        let candidate = entry.path().join(profile).join("ralph");
                        if candidate.exists() {
                            return candidate;
                        }
                    }
                }
            }
        }
    }

    // Try locating via `which` as a last resort
    if let Ok(output) = std::process::Command::new("which").arg("ralph").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !path.is_empty() {
                return PathBuf::from(path);
            }
        }
    }

    panic!(
        "ralph binary not found; run `cargo build` first. \
         Searched: target/{{debug,release}}/ralph, \
         target/<triple>/{{debug,release}}/ralph, \
         CARGO_TARGET_DIR (same layouts), PATH"
    );
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
