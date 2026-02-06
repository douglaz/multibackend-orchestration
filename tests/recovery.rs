//! Integration tests for state corruption recovery.

use std::fs;
use std::path::Path;
use std::process::Command;

use ralph::error::RalphError;
use ralph::project::lifecycle::{
    create_project, load_project_state, CreateProjectOptions, PromptSource,
};
use ralph::workspace::Workspace;
use tempfile::TempDir;

fn git_ok(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git command should execute");
    assert!(
        status.success(),
        "git command failed: git {}",
        args.join(" ")
    );
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git command failed: git {}",
        args.join(" ")
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn setup_project() -> (TempDir, std::path::PathBuf, String) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);

    fs::write(repo_root.join("README.md"), "# test\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo\n").expect("write prompt");

    create_project(
        &mut workspace,
        CreateProjectOptions {
            id: "01-poc".to_owned(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, "01-poc".to_owned())
}

#[test]
fn recovers_corrupted_state_from_git_head_when_tracked() {
    let (_temp, workspace_root, project_id) = setup_project();
    let repo_root = workspace_root
        .parent()
        .expect("workspace should be inside repo root");

    // Ensure state.json is recoverable from git HEAD.
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "snapshot state"]);

    let project_dir = workspace_root.join("projects").join(&project_id);
    let state_path = project_dir.join("state.json");
    fs::write(&state_path, "{ not valid json").expect("corrupt state");

    let recovered = load_project_state(&project_dir).expect("state should recover from git");
    assert_eq!(recovered.project_id, project_id);

    let persisted = fs::read_to_string(&state_path).expect("read recovered state");
    assert!(
        serde_json::from_str::<serde_json::Value>(&persisted).is_ok(),
        "recovered state file should be valid JSON"
    );
}

#[test]
fn returns_corrupted_state_error_when_recovery_unavailable() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);
    let state_path = project_dir.join("state.json");
    fs::write(&state_path, "{ definitely invalid").expect("corrupt state");

    let err = load_project_state(&project_dir).expect_err("expected corrupted state error");
    match err {
        RalphError::CorruptedState { path, reason } => {
            assert!(path.ends_with("state.json"));
            assert!(
                reason.contains("recovery failed") || reason.contains("git recovery failed"),
                "unexpected recovery error reason: {reason}"
            );
        }
        other => panic!("expected CorruptedState error, got: {other}"),
    }
}
