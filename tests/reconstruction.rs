//! Focused integration tests for state reconstruction edge cases.
//!
//! These tests verify that `reconstruct_project_state_from_project_dir`
//! handles malformed/missing checkpoints and defaults correctly.

use std::fs;
use std::path::Path;
use std::process::Command;

use ralph::project::lifecycle::{
    create_project, reconstruct_project_state_from_project_dir, CreateProjectOptions, PromptSource,
};
use ralph::project::state::Phase;
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
        &workspace,
        CreateProjectOptions {
            id: "issue-1".to_owned(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, "issue-1".to_owned())
}

#[test]
fn no_loop_artifacts_defaults_to_loop_1_planning() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction should succeed");

    // With no artifacts and no checkpoint commits, default to loop=1, phase=planning
    // (current_loop=0 means no loop is active, next loop is 1)
    assert_eq!(state.current_phase, Phase::Planning);
    assert!(state.loops.is_empty());
}

#[test]
fn reconstruction_with_spec_only_infers_implementing_phase() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    // Create loop directory with only a spec artifact
    let loop_dir = project_dir.join("loops/001-auth");
    fs::create_dir_all(&loop_dir).expect("create loop dir");
    fs::write(
        loop_dir.join("20260219120000-spec.md"),
        "---\nartifact: spec\nloop: 1\n---\n# Feature: Auth\n\n## Description\nAuth module.\n",
    )
    .expect("write spec");

    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction with spec should succeed");

    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.current_phase, Phase::Implementing);
}

#[test]
fn reconstruction_with_spec_and_impl_notes_infers_qa_or_reviewing_phase() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    let loop_dir = project_dir.join("loops/001-auth");
    fs::create_dir_all(&loop_dir).expect("create loop dir");
    fs::write(
        loop_dir.join("20260219120000-spec.md"),
        "---\nartifact: spec\nloop: 1\n---\n# Feature: Auth\n\n## Description\nAuth module.\n",
    )
    .expect("write spec");
    fs::write(
        loop_dir.join("20260219120100-impl-notes.md"),
        "---\nartifact: impl-notes\nloop: 1\n---\n# Implementation Notes\n\n## Decisions Made\n- ok\n",
    )
    .expect("write impl-notes");

    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction with spec+impl should succeed");

    assert_eq!(state.loops.len(), 1);
    // With spec + impl-notes but no QA results and no review, should be at QA or Reviewing
    assert!(
        state.current_phase == Phase::QA || state.current_phase == Phase::Reviewing,
        "expected QA or Reviewing, got {:?}",
        state.current_phase
    );
}

#[test]
fn reconstruction_ignores_stale_state_json() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    // Write a state.json that says we're at loop 5, completing
    let state_path = project_dir.join("state.json");
    fs::write(
        &state_path,
        r#"{"project_id":"issue-1","current_loop":5,"current_phase":"completing"}"#,
    )
    .expect("write stale state.json");

    // Reconstruction should derive state from artifacts, not state.json
    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction should succeed ignoring state.json");

    assert_eq!(state.current_phase, Phase::Planning);
    assert!(state.loops.is_empty());
}
