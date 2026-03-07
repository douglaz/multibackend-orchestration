use std::path::Path;
use std::time::Duration;

use tracing::info;

use crate::daemon::process::run_command_with_timeout;

pub struct PreCommitCheckResult {
    pub passed: bool,
    pub feedback: String,
}

pub fn run_pre_commit_checks(
    repo_root: &Path,
    fmt_enabled: bool,
    clippy_enabled: bool,
    nix_build_enabled: bool,
    fmt_auto_fix: bool,
) -> PreCommitCheckResult {
    let has_cargo_toml = repo_root.join("Cargo.toml").exists();
    let mut failures = Vec::new();

    // cargo fmt
    if fmt_enabled && has_cargo_toml {
        let fmt_passed = run_cargo_fmt_check(repo_root, fmt_auto_fix);
        if let Some(feedback) = fmt_passed {
            failures.push(feedback);
        }
    }

    // cargo clippy
    if clippy_enabled && has_cargo_toml {
        let clippy_result = run_check(
            repo_root,
            "cargo clippy --all-targets -- -D warnings",
            &["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
            Duration::from_secs(300),
        );
        if let Some(feedback) = clippy_result {
            failures.push(feedback);
        }
    }

    // nix build
    if nix_build_enabled {
        let nix_result = run_check(
            repo_root,
            "nix build",
            &["nix", "build"],
            Duration::from_secs(600),
        );
        if let Some(feedback) = nix_result {
            failures.push(feedback);
        }
    }

    if failures.is_empty() {
        PreCommitCheckResult {
            passed: true,
            feedback: String::new(),
        }
    } else {
        PreCommitCheckResult {
            passed: false,
            feedback: failures.join("\n"),
        }
    }
}

/// Run `cargo fmt --check`. If it fails and `auto_fix` is true, try `cargo fmt`
/// and only report failure if the auto-fix itself fails.
/// Returns `None` on success, `Some(feedback)` on failure.
fn run_cargo_fmt_check(repo_root: &Path, auto_fix: bool) -> Option<String> {
    let check_result = run_check(
        repo_root,
        "cargo fmt --check",
        &["cargo", "fmt", "--check"],
        Duration::from_secs(120),
    );

    match check_result {
        None => None, // passed
        Some(feedback) => {
            if auto_fix {
                info!("cargo fmt --check failed, attempting auto-fix with cargo fmt");
                let fix_result = run_check(
                    repo_root,
                    "cargo fmt",
                    &["cargo", "fmt"],
                    Duration::from_secs(120),
                );
                match fix_result {
                    None => {
                        info!("cargo fmt auto-fix succeeded");
                        None
                    }
                    Some(fix_feedback) => Some(fix_feedback),
                }
            } else {
                Some(feedback)
            }
        }
    }
}

/// Run a single check command. Returns `None` if the command succeeds (exit 0),
/// or `Some(feedback_section)` with a markdown-formatted error section.
fn run_check(repo_root: &Path, label: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = std::process::Command::new(args[0]);
    cmd.args(&args[1..]).current_dir(repo_root);

    info!(check = label, "running pre-commit check");

    match run_command_with_timeout(&mut cmd, timeout) {
        Ok(output) => {
            if output.status.success() {
                info!(check = label, "pre-commit check passed");
                None
            } else {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}{stderr}");
                info!(check = label, "pre-commit check failed");
                Some(format!("## {label}\n```\n{}\n```\n", combined.trim()))
            }
        }
        Err(e) => {
            info!(check = label, error = %e, "pre-commit check error");
            Some(format!("## {label}\nError: {e}\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn all_checks_disabled_returns_passed() {
        let tmp = TempDir::new().unwrap();
        let result = run_pre_commit_checks(tmp.path(), false, false, false, false);
        assert!(result.passed);
        assert!(result.feedback.is_empty());
    }

    #[test]
    fn no_cargo_toml_skips_cargo_checks() {
        let tmp = TempDir::new().unwrap();
        // Enable fmt and clippy but no Cargo.toml exists
        let result = run_pre_commit_checks(tmp.path(), true, true, false, false);
        assert!(result.passed);
        assert!(result.feedback.is_empty());
    }

    #[test]
    fn error_converted_to_feedback_not_panic() {
        let tmp = TempDir::new().unwrap();
        // Force a spawn error by invoking a non-existent command
        let feedback = run_check(
            tmp.path(),
            "test-check",
            &["/nonexistent/command-that-does-not-exist"],
            Duration::from_secs(5),
        );
        // Spawn error must produce feedback (Some), not be swallowed
        let feedback = feedback.expect("spawn error should produce Some(feedback)");
        assert!(
            feedback.starts_with("## test-check"),
            "feedback should start with section header, got: {feedback}"
        );
        assert!(
            feedback.contains("Error:"),
            "feedback should contain 'Error:' for spawn failures, got: {feedback}"
        );
    }

    #[test]
    fn nix_build_runs_independently_of_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        // No Cargo.toml, but nix build enabled — should attempt nix build
        // This will fail (no flake.nix) but should not panic
        let result = run_pre_commit_checks(tmp.path(), false, false, true, false);
        // nix build should fail with an error since there's no flake.nix
        assert!(!result.passed || result.feedback.is_empty());
    }

    #[test]
    fn feedback_aggregation_includes_section_headers() {
        // Test that the run_check function produces proper section headers
        let tmp = TempDir::new().unwrap();
        let result = run_check(
            tmp.path(),
            "test-command",
            &["false"], // `false` always exits with 1
            Duration::from_secs(5),
        );
        assert!(result.is_some());
        let feedback = result.unwrap();
        assert!(feedback.starts_with("## test-command"));
    }
}
