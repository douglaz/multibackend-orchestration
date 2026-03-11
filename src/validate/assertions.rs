use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use regex::Regex;
use serde_json::Value;
use serde_yaml::Value as YamlValue;
use toml::Value as TomlValue;

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

pub fn assert_git_tag_not_exists(repo_root: &Path, tag: &str) {
    let tag_ref = format!("refs/tags/{tag}");
    let status = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", &tag_ref])
        .current_dir(repo_root)
        .status()
        .expect("git should execute");
    assert!(
        !status.success(),
        "expected git tag '{tag}' to NOT exist in {}",
        repo_root.to_string_lossy()
    );
}

pub fn assert_dir_exists(path: &Path) {
    assert!(
        path.is_dir(),
        "expected directory to exist: {}",
        path.to_string_lossy()
    );
}

pub fn assert_file_not_empty(path: &Path) {
    assert_file_exists(path);
    let meta = fs::metadata(path)
        .unwrap_or_else(|err| panic!("failed to stat {}: {err}", path.to_string_lossy()));
    assert!(
        meta.len() > 0,
        "expected file to be non-empty: {}",
        path.to_string_lossy()
    );
}

pub fn load_toml(path: &Path) -> TomlValue {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.to_string_lossy()));
    content
        .parse::<TomlValue>()
        .unwrap_or_else(|err| panic!("failed to parse TOML {}: {err}", path.to_string_lossy()))
}

pub fn assert_toml_field(value: &TomlValue, field: &str, expected: &TomlValue) {
    let mut current = value;
    for segment in field.split('.') {
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("missing TOML field segment '{segment}' in '{field}'"));
    }
    assert_eq!(
        current, expected,
        "unexpected value for TOML field '{field}'"
    );
}

pub fn assert_json_array_len(value: &Value, field: &str, expected_len: usize) {
    let mut current = value;
    for segment in field.split('.') {
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("missing JSON field segment '{segment}' in '{field}'"));
    }
    let arr = current
        .as_array()
        .unwrap_or_else(|| panic!("expected JSON field '{field}' to be an array"));
    assert_eq!(
        arr.len(),
        expected_len,
        "expected JSON array '{field}' to have {expected_len} elements, got {}",
        arr.len()
    );
}

pub fn assert_artifact_timestamp_naming(path: &Path) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("artifact path does not have a valid file name: {path:?}"));
    let re = Regex::new(
        r"^(\d{14}-[a-z0-9-]+\.md|\d{14}-agent-output-[a-z0-9_-]+-\d+\.log|agent-output-[a-z0-9_-]+\.log)$",
    )
    .expect("valid regex");
    assert!(
        re.is_match(name),
        "expected timestamped artifact name, got '{}'",
        name
    );
}

pub fn parse_yaml_frontmatter(path: &Path) -> YamlValue {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.to_string_lossy()));
    let mut lines = content.lines();
    let first = lines.next().unwrap_or_default();
    assert_eq!(
        first.trim(),
        "---",
        "expected '{}' to begin with YAML frontmatter delimiter",
        path.to_string_lossy()
    );

    let mut frontmatter = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    assert!(
        closed,
        "expected '{}' to contain a closing YAML frontmatter delimiter",
        path.to_string_lossy()
    );

    serde_yaml::from_str(&frontmatter).unwrap_or_else(|err| {
        panic!(
            "failed to parse YAML frontmatter from {}: {err}",
            path.to_string_lossy()
        )
    })
}

pub fn assert_no_loop_artifacts(project_dir: &Path) {
    let loops_dir = project_dir.join("loops");
    if !loops_dir.exists() {
        return;
    }
    let mut entries = fs::read_dir(&loops_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", loops_dir.to_string_lossy()));
    assert!(
        entries.next().is_none(),
        "expected no loop artifacts under '{}'",
        loops_dir.to_string_lossy()
    );
}

pub fn assert_stderr_contains(output: &Output, needle: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(needle),
        "expected stderr to contain '{}', got:\n{}",
        needle,
        stderr
    );
}

pub fn assert_path_not_exists(path: &Path) {
    assert!(
        !path.exists(),
        "expected path to not exist: {}",
        path.to_string_lossy()
    );
}

pub fn git_head_commit(repo_root: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .expect("git rev-parse should execute");
    assert!(
        output.status.success(),
        "git rev-parse HEAD failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub fn git_tag_commit(repo_root: &Path, tag: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", tag])
        .current_dir(repo_root)
        .output()
        .expect("git rev-parse should execute");
    assert!(
        output.status.success(),
        "git rev-parse {tag} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Assert that there are no uncommitted `.ralph/` files in the repo.
/// This ensures completion artifacts are auto-committed.
pub fn assert_no_uncommitted_ralph_files(repo_root: &Path) {
    let output = Command::new("git")
        .args(["status", "--porcelain", ".ralph/"])
        .current_dir(repo_root)
        .output()
        .expect("git status should execute");
    assert!(
        output.status.success(),
        "git status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = String::from_utf8_lossy(&output.stdout);
    let uncommitted: Vec<&str> = status.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        uncommitted.is_empty(),
        "expected no uncommitted .ralph/ files after completion, found:\n{}",
        uncommitted.join("\n")
    );
}

/// Assert that the given JSON value is a top-level array, returning a reference
/// to the array. Panics with a descriptive message if the value is not an array.
pub fn assert_json_array(value: &Value) -> &Vec<Value> {
    value.as_array().unwrap_or_else(|| {
        panic!(
            "expected top-level JSON array, got {}",
            match value {
                Value::Object(_) => "object",
                Value::String(_) => "string",
                Value::Number(_) => "number",
                Value::Bool(_) => "bool",
                Value::Null => "null",
                Value::Array(_) => unreachable!(),
            }
        )
    })
}

/// Normalize a backend string by stripping model suffixes.
/// For example: `"claude(sonnet-4)"` → `"claude"`, `"codex(gpt-5.4)"` → `"codex"`.
/// If the string has no parenthesized suffix, it is returned as-is (lowercased).
pub fn normalize_backend(backend: &str) -> String {
    let s = backend.trim();
    if let Some(idx) = s.find('(') {
        s[..idx].trim().to_lowercase()
    } else {
        s.to_lowercase()
    }
}

/// Strip ANSI SGR escape sequences from a string.
///
/// `tracing_subscriber::fmt()` may emit ANSI colour/style codes around
/// structured field names even when stderr is captured into a pipe (depending
/// on version and configuration).  Stripping these codes before substring
/// assertions makes tests robust across environments.
pub fn strip_ansi(s: &str) -> String {
    let re = Regex::new(r"\x1b\[[0-9;]*m").expect("valid ANSI regex");
    re.replace_all(s, "").into_owned()
}
