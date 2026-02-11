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
    let re = Regex::new(r"^\d{14}-[a-z0-9-]+\.md$").expect("valid regex");
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
