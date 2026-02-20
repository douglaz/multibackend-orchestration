//! Integration tests for state corruption recovery.

use std::fs;
use std::path::Path;
use std::process::Command;

use ralph::error::RalphError;
use ralph::project::lifecycle::{
    create_project, load_project_state, CreateProjectOptions, PromptSource,
};
use ralph::project::state::{
    FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType, Phase,
    ProjectState, ProjectStatus,
};
use ralph::workspace::Workspace;
use tempfile::TempDir;
use chrono::Utc;

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
            id: "01-poc".to_owned(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, "01-poc".to_owned())
}

fn synthetic_two_loop_state(project_id: &str) -> ProjectState {
    let mut state = ProjectState::new(project_id, "Proof of Concept", "prompt-hash", None);
    let now = Utc::now();

    state.current_loop = 2;
    state.current_phase = Phase::Reviewing;
    state.phase_iteration = 2;
    state.status = ProjectStatus::InProgress;
    state.loops = vec![
        FeatureLoopState {
            loop_number: 1,
            slug: "first".to_owned(),
            feature_name: "First".to_owned(),
            loop_type: LoopType::Feature,
            status: LoopStatus::Completed,
            backends: FeatureLoopBackends {
                planner: "claude".to_owned(),
                implementer: "codex".to_owned(),
                reviewer: "claude".to_owned(),
                qa: "claude".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: "loops/001-first/spec.md".to_owned(),
                impl_notes: Some("loops/001-first/impl-notes.md".to_owned()),
                reviews: vec![],
                approval: Some("loops/001-first/review-approved.md".to_owned()),
                qa_results: vec![],
                pending_qa_feedback: None,
            },
            commit: Some("abc123".to_owned()),
            started_at: now,
            completed_at: Some(now),
        },
        FeatureLoopState {
            loop_number: 2,
            slug: "second".to_owned(),
            feature_name: "Second".to_owned(),
            loop_type: LoopType::Feature,
            status: LoopStatus::InProgress,
            backends: FeatureLoopBackends {
                planner: "claude".to_owned(),
                implementer: "codex".to_owned(),
                reviewer: "claude".to_owned(),
                qa: "claude".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: "loops/002-second/spec.md".to_owned(),
                impl_notes: Some("loops/002-second/impl-notes.md".to_owned()),
                reviews: vec![],
                approval: None,
                qa_results: vec![],
                pending_qa_feedback: None,
            },
            commit: None,
            started_at: now,
            completed_at: None,
        },
    ];

    state
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

#[test]
fn reconstruction_clamps_to_rollback_marker_boundary() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);
    let state_path = project_dir.join("state.json");
    let marker_path = project_dir.join(".rollback-target");

    let state = synthetic_two_loop_state(&project_id);
    state.save(&state_path).expect("save synthetic state");
    fs::write(&marker_path, "1\n").expect("write marker");

    let loaded = load_project_state(&project_dir).expect("load state");
    assert_eq!(loaded.current_loop, 1);
    assert_eq!(loaded.current_phase, Phase::Planning);
    assert_eq!(loaded.phase_iteration, 1);
    assert_eq!(loaded.loops.len(), 1);
    assert_eq!(loaded.loops[0].loop_number, 1);
}

#[test]
fn reconstruction_without_marker_remains_unchanged() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);
    let state_path = project_dir.join("state.json");

    let state = synthetic_two_loop_state(&project_id);
    state.save(&state_path).expect("save synthetic state");

    let loaded = load_project_state(&project_dir).expect("load state");
    assert_eq!(loaded.current_loop, 2);
    assert_eq!(loaded.current_phase, Phase::Reviewing);
    assert_eq!(loaded.loops.len(), 2);
}

#[test]
fn malformed_rollback_marker_is_ignored() {
    let (_temp, workspace_root, project_id) = setup_project();
    let project_dir = workspace_root.join("projects").join(&project_id);
    let state_path = project_dir.join("state.json");
    let marker_path = project_dir.join(".rollback-target");

    let state = synthetic_two_loop_state(&project_id);
    state.save(&state_path).expect("save synthetic state");
    fs::write(&marker_path, "not-a-number\n").expect("write malformed marker");

    let loaded = load_project_state(&project_dir).expect("load state");
    assert_eq!(loaded.current_loop, 2);
    assert_eq!(loaded.current_phase, Phase::Reviewing);
    assert_eq!(loaded.loops.len(), 2);
}
