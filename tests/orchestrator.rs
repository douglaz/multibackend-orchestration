//! Integration tests for orchestration flows.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ralph::project::lifecycle::{
    create_project, load_project_state, CreateProjectOptions, PromptSource,
};
use ralph::project::state::{CompletionVerdict, LoopStatus, Phase, ProjectStatus};
use ralph::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_reviewer_template,
};
use ralph::workflow::orchestrator::{Orchestrator, RunOptions};
use ralph::workspace::Workspace;
use regex::Regex;
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

fn setup_workspace_with_project(
    planner_mode: &str,
    completer_mode: &str,
) -> (TempDir, PathBuf, String, PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);

    fs::write(repo_root.join("README.md"), "# demo\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    let script_path = repo_root.join("mock_backend.sh");
    write_backend_script(&script_path);
    git_ok(repo_root, &["add", "mock_backend.sh"]);
    git_ok(repo_root, &["commit", "-m", "test: add backend mock"]);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    fs::write(
        workspace_root.join("templates/planner.md"),
        default_planner_template(),
    )
    .expect("write planner template");
    fs::write(
        workspace_root.join("templates/implementer.md"),
        default_implementer_template(),
    )
    .expect("write implementer template");
    fs::write(
        workspace_root.join("templates/reviewer.md"),
        default_reviewer_template(),
    )
    .expect("write reviewer template");
    fs::write(
        workspace_root.join("templates/completer.md"),
        default_completer_template(),
    )
    .expect("write completer template");

    let mut env = BTreeMap::new();
    env.insert("PLANNER_MODE".to_owned(), planner_mode.to_owned());
    env.insert("REVIEW_MODE".to_owned(), "approved".to_owned());
    env.insert("COMPLETER_MODE".to_owned(), completer_mode.to_owned());

    workspace.config.backends.claude.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = env.clone();

    workspace.config.backends.codex.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = env;

    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo system\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "01-poc".to_owned();
    create_project(
        &mut workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id, script_path)
}

fn write_backend_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  if [[ "${PLANNER_MODE:-feature}" == "completion" ]]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
All project requirements have been satisfied.

## Summary of Work
- Built and validated the required project behavior.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: Demo Feature

## Description
Implement a minimal demo feature.

## Acceptance Criteria
- [ ] Demo behavior exists

## Files to Modify/Create
- `README.md` - Document the demo feature

## Dependencies
- Requires: none
- Blocks: none
EOF
  fi
elif [[ "$prompt" == *"You are a software developer implementing a feature specification."* ]]; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Kept implementation minimal to satisfy the spec.

## Spec Deviations
- None

## Testing
- cargo test
EOF
elif [[ "$prompt" == *"You are a code reviewer ensuring implementations match specifications."* ]]; then
  if [[ "${REVIEW_MODE:-approved}" == "suggestions" ]]; then
    cat <<'EOF'
# Review: SUGGESTIONS

## Required Changes
1. **Demo check**: tighten behavior
   - Current: loose
   - Expected: strict
   - Reference: spec
EOF
  else
    cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Demo behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: demo feature
EOF
  fi
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  if [[ "${COMPLETER_MODE:-complete}" == "continue" ]]; then
    cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional requirement remains.

## Recommended Next Features
1. Implement remaining behavior.
EOF
  else
    cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Demo requirement: satisfied by Demo Feature
EOF
  fi
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn run_options(project_id: &str) -> RunOptions {
    RunOptions {
        project: Some(project_id.to_owned()),
        loops: Some(1),
        until_review: false,
        until_complete: false,
        dry_run: false,
        backend: None,
        on_prompt_change: None,
        skip_commit: false,
    }
}

fn assert_timestamped_artifact(rel_path: &str, suffix: &str) {
    let suffix = regex::escape(suffix);
    let pattern = format!(r"^loops/\d{{3}}-[a-z0-9-]+/\d{{14}}-{suffix}$");
    let re = Regex::new(&pattern).expect("valid regex");
    assert!(
        re.is_match(rel_path),
        "artifact path should be timestamp-prefixed: {rel_path}"
    );
}

#[tokio::test]
async fn runs_full_feature_loop_and_commits() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("feature", "complete");

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("orchestration should succeed");

    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.loops[0].status, LoopStatus::Completed);
    assert!(state.loops[0].commit.is_some());
    assert_eq!(state.current_phase, Phase::Planning);
    assert_timestamped_artifact(&state.loops[0].artifacts.spec, "spec.md");
    assert_timestamped_artifact(
        state.loops[0]
            .artifacts
            .impl_notes
            .as_deref()
            .expect("impl-notes artifact should exist"),
        "impl-notes.md",
    );
    assert_timestamped_artifact(
        state.loops[0]
            .artifacts
            .approval
            .as_deref()
            .expect("approval artifact should exist"),
        "review-approved.md",
    );

    let repo_root = workspace_root.parent().expect("repo root");
    let tag = git_output(repo_root, &["tag", "--list", "ralph/01-poc/loop-1"]);
    assert_eq!(tag.trim(), "ralph/01-poc/loop-1");
}

#[tokio::test]
async fn supports_interrupt_and_resume_from_commit_phase() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("feature", "complete");

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.until_review = true;
    options.loops = None;

    orchestrator
        .run(options)
        .await
        .expect("orchestration until-review should succeed");

    let state_after_review = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert_eq!(state_after_review.current_phase, Phase::Committing);
    assert!(state_after_review.loops[0].commit.is_none());

    let workspace = Workspace::load(workspace_root.clone()).expect("reload workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("resume run should commit");

    let final_state = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert!(final_state.loops[0].commit.is_some());
    assert_eq!(final_state.loops[0].status, LoopStatus::Completed);
}

#[tokio::test]
async fn executes_completion_flow_until_complete() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("completion", "complete");

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.loops = None;
    options.until_complete = true;

    orchestrator
        .run(options)
        .await
        .expect("completion flow should succeed");

    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert_eq!(state.status, ProjectStatus::Completed);
    assert_eq!(state.completion_attempts.len(), 1);
    assert_eq!(
        state.completion_attempts[0].verdict,
        Some(CompletionVerdict::Complete)
    );
    assert_timestamped_artifact(
        &state.completion_attempts[0].artifacts.termination_request,
        "termination-request.md",
    );
    assert_timestamped_artifact(
        state.completion_attempts[0]
            .artifacts
            .verdict
            .as_deref()
            .expect("completion verdict artifact should exist"),
        "completer-verdict.md",
    );
}

#[tokio::test]
async fn dry_run_does_not_checkout_project_branch() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("feature", "complete");
    let repo_root = workspace_root.parent().expect("repo root");
    let branch_before = git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.dry_run = true;

    orchestrator
        .run(options)
        .await
        .expect("dry-run should succeed");

    let branch_after = git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(branch_before, branch_after);

    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert_eq!(state.current_loop, 0);
    assert_eq!(state.status, ProjectStatus::Pending);
}

#[tokio::test]
async fn refuses_new_loop_when_non_workspace_changes_are_dirty() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("feature", "complete");
    let repo_root = workspace_root.parent().expect("repo root");

    fs::write(repo_root.join("scratch.txt"), "dirty\n").expect("write dirty file");

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let result = orchestrator.run(run_options(&project_id)).await;

    assert!(
        result.is_err(),
        "run should fail when starting from dirty tree"
    );
    let err_message = result.unwrap_err().to_string();
    assert!(
        err_message.contains("cannot start a new loop with uncommitted changes outside `.ralph/`"),
        "unexpected error message: {err_message}"
    );
    assert!(
        err_message.contains("scratch.txt"),
        "error should list changed path: {err_message}"
    );

    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert_eq!(state.current_loop, 0);
    assert_eq!(state.status, ProjectStatus::Pending);
}
