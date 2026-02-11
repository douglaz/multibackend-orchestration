use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

pub fn assert_exit_code(output: &Output, expected: i32) {
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(
        code,
        expected,
        "unexpected exit code.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn assert_json_field(value: &Value, field: &str, expected: &Value) {
    let mut current = value;
    for segment in field.split('.') {
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("missing JSON field segment '{segment}' in '{field}'"));
    }

    assert_eq!(
        current, expected,
        "unexpected value for JSON field '{field}'"
    );
}

pub fn assert_file_exists(path: &Path) {
    assert!(
        path.exists(),
        "expected file to exist: {}",
        path.to_string_lossy()
    );
}

pub fn assert_file_contains(path: &Path, needle: &str) {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.to_string_lossy()));
    assert!(
        content.contains(needle),
        "expected '{}' to contain '{}'",
        path.to_string_lossy(),
        needle
    );
}

pub fn assert_stdout_contains(output: &Output, needle: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(needle),
        "expected stdout to contain '{}', got:\n{}",
        needle,
        stdout
    );
}

pub fn assert_git_branch_exists(repo_root: &Path, branch: &str) {
    let branch_ref = format!("refs/heads/{branch}");
    let status = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &branch_ref])
        .current_dir(repo_root)
        .status()
        .expect("git should execute");
    assert!(
        status.success(),
        "expected git branch '{branch}' to exist in {}",
        repo_root.to_string_lossy()
    );
}

pub fn assert_git_tag_exists(repo_root: &Path, tag: &str) {
    let tag_ref = format!("refs/tags/{tag}");
    let status = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &tag_ref])
        .current_dir(repo_root)
        .status()
        .expect("git should execute");
    assert!(
        status.success(),
        "expected git tag '{tag}' to exist in {}",
        repo_root.to_string_lossy()
    );
}
