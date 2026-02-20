//! Integration tests for checkpoint-derived state reconstruction.
//!
//! These tests verify that `reconstruct_project_state_from_project_dir` derives
//! workflow position exclusively from checkpoint commits on the project branch.
//! When no checkpoint commit exists, the default position is `loop=1`,
//! `phase=planning` regardless of which loop artifacts are present on disk.

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
fn no_checkpoint_defaults_to_loop_1_planning() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction should succeed for fresh project");

    assert_eq!(state.project_id, project_id);
    // No checkpoint commit exists, so position defaults to loop=1, planning.
    assert_eq!(state.current_loop, 1);
    assert_eq!(state.current_phase, Phase::Planning);
    assert!(state.loops.is_empty());
    assert!(state.completion_attempts.is_empty());
}

#[test]
fn stale_state_json_does_not_affect_checkpoint_position() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    // Write a stale/corrupt state.json — checkpoint derivation ignores it
    let state_path = project_dir.join("state.json");
    fs::write(&state_path, "{ definitely invalid }").expect("write corrupt state.json");

    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction should succeed even with corrupt state.json on disk");

    assert_eq!(state.project_id, project_id);
    // No checkpoint commit → defaults to loop=1, planning despite state.json
    assert_eq!(state.current_loop, 1);
    assert_eq!(state.current_phase, Phase::Planning);
}

#[test]
fn artifacts_without_checkpoint_default_to_loop_1_planning() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);

    // Create a loop directory with a spec artifact — but no checkpoint commit
    let loop_dir = project_dir.join("loops/001-demo-feature");
    fs::create_dir_all(&loop_dir).expect("create loop dir");
    fs::write(
        loop_dir.join("20260219120000-spec.md"),
        "---\nartifact: spec\nloop: 1\n---\n# Feature: Demo Feature\n\n## Description\nDemo\n",
    )
    .expect("write spec");

    let state = reconstruct_project_state_from_project_dir(&project_dir)
        .expect("reconstruction should succeed with spec artifact");

    // Loop artifacts are collected but position is checkpoint-derived.
    // No checkpoint commit → default loop=1, planning.
    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.loops[0].loop_number, 1);
    assert_eq!(state.current_loop, 1);
    assert_eq!(state.current_phase, Phase::Planning);
}
