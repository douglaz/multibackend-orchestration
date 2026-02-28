//! Integration tests for orchestration flows.

use ralph::error::RalphError;
use ralph::project::lifecycle::{
    create_project, reconstruct_project_state_from_project_dir, CreateProjectOptions, PromptSource,
};
use ralph::project::state::{
    AcceptanceQaResult, CompletionVerdict, LoopStatus, Phase, ProjectStatus,
};
use ralph::prompts::templates::{
    default_completer_template, default_implementer_template, default_planner_template,
    default_qa_template, default_reviewer_template,
};
use ralph::workflow::orchestrator::{Orchestrator, RunOptions};
use ralph::workspace::Workspace;
use regex::Regex;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/// Add a local bare remote to a git repo so `git push origin` works in tests.
/// Must be called after the initial commits so the bare remote has a HEAD.
/// The bare repo lives at `<repo>/.test-remote.git` and is `.gitignore`d to
/// avoid triggering dirty-tree checks.
fn add_local_bare_remote(repo_root: &Path) {
    let bare_dir = repo_root.join(".test-remote.git");
    let bare_str = bare_dir.to_string_lossy().to_string();

    // Gitignore the bare dir so it doesn't show up as untracked
    let gitignore = repo_root.join(".gitignore");
    let mut contents = fs::read_to_string(&gitignore).unwrap_or_default();
    contents.push_str("/.test-remote.git\n");
    fs::write(&gitignore, &contents).expect("write .gitignore");
    git_ok(repo_root, &["add", ".gitignore"]);
    git_ok(repo_root, &["commit", "-m", "chore: gitignore test remote"]);

    Command::new("git")
        .args(["init", "--bare", &bare_str])
        .status()
        .expect("bare init");
    git_ok(repo_root, &["remote", "add", "origin", &bare_str]);
    git_ok(repo_root, &["push", "-u", "origin", "HEAD"]);
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

    add_local_bare_remote(repo_root);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    fs::write(
        workspace_root.join("templates/spec.md"),
        default_planner_template(),
    )
    .expect("write spec template");
    fs::write(
        workspace_root.join("templates/implementation.md"),
        default_implementer_template(),
    )
    .expect("write implementation template");
    fs::write(
        workspace_root.join("templates/review.md"),
        default_reviewer_template(),
    )
    .expect("write review template");
    fs::write(
        workspace_root.join("templates/completion.md"),
        default_completer_template(),
    )
    .expect("write completion template");
    fs::write(
        workspace_root.join("templates/qa.md"),
        default_qa_template(),
    )
    .expect("write qa template");

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
    workspace.config.workflow.final_review_enabled = false;
    workspace.config.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo system\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "issue-1".to_owned();
    create_project(
        &workspace,
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
elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock check: passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
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
        planner_backend: None,
        implementer_backend: None,
        reviewer_backend: None,
        qa_backend: None,
        completer_backend: None,
        tmux: None,
        on_prompt_change: None,
        skip_commit: false,
        skip_prompt_review: false,
        pr_url: None,
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

fn assert_timestamped_acceptance_artifact(rel_path: &str, base: &str) {
    let base = regex::escape(base);
    let pattern = format!(r"^loops/\d{{3}}-[a-z0-9-]+/\d{{14}}-{base}(?:-[a-z0-9-]+)?\.md$");
    let re = Regex::new(&pattern).expect("valid regex");
    assert!(
        re.is_match(rel_path),
        "acceptance artifact path should be timestamp-prefixed: {rel_path}"
    );
}

fn assert_acceptance_results_cover_both_families(results: &[AcceptanceQaResult]) {
    assert_eq!(
        results.len(),
        2,
        "expected exactly two acceptance results, got {}",
        results.len()
    );
    assert_ne!(
        results[0].backend, results[1].backend,
        "acceptance backend entries must be distinct"
    );
    assert!(
        results
            .iter()
            .any(|result| result.backend.starts_with("claude")),
        "expected one acceptance result from claude backend family, got {:?}",
        results
            .iter()
            .map(|result| &result.backend)
            .collect::<Vec<_>>()
    );
    assert!(
        results
            .iter()
            .any(|result| result.backend.starts_with("codex")),
        "expected one acceptance result from codex backend family, got {:?}",
        results
            .iter()
            .map(|result| &result.backend)
            .collect::<Vec<_>>()
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

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
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

    // Verify a structured checkpoint commit was pushed for loop 1
    let repo_root = workspace_root.parent().expect("repo root");
    let log = git_output(
        repo_root,
        &["log", "--oneline", "--all", "--grep=ralph(issue-1): loop 1"],
    );
    assert!(
        !log.is_empty(),
        "expected a Ralph checkpoint commit for loop 1"
    );
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

    let state_after_review = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert_eq!(state_after_review.current_phase, Phase::Committing);
    // With checkpoint commits, the reviewing→committing transition already
    // produces a commit hash; the loop is marked Completed by reconstruction
    // once approval + commit_hash exist.
    assert!(state_after_review.loops[0].commit.is_some());

    let workspace = Workspace::load(workspace_root.clone()).expect("reload workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("resume run should commit");

    let final_state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
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

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
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
    let verdict_path = state.completion_attempts[0]
        .artifacts
        .verdict
        .as_deref()
        .expect("completion verdict artifact should exist");
    assert!(
        verdict_path.contains("completer-verdict"),
        "verdict artifact should contain completer-verdict: {verdict_path}"
    );

    // Completion artifacts should be auto-committed
    let repo_root = workspace_root.parent().expect("repo root");
    let status_output = Command::new("git")
        .args(["status", "--porcelain", ".ralph/"])
        .current_dir(repo_root)
        .output()
        .expect("git status should execute");
    let uncommitted = String::from_utf8_lossy(&status_output.stdout);
    let uncommitted_lines: Vec<&str> = uncommitted
        .lines()
        .filter(|l| !l.is_empty() && *l != "?? .ralph/")
        .collect();
    assert!(
        uncommitted_lines.is_empty(),
        "expected no uncommitted .ralph/ files after completion, found:\n{}",
        uncommitted_lines.join("\n")
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

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    // No checkpoint commits exist after dry-run; reconstruction defaults to loop 1.
    assert!(
        state.loops.is_empty(),
        "loops should be empty after dry-run"
    );
    assert_eq!(state.current_loop, 1);
    assert_eq!(state.current_phase, Phase::Planning);
    assert_eq!(state.phase_iteration, 1);
    assert_eq!(state.status, ProjectStatus::Pending);
}

#[tokio::test]
async fn tmux_mode_fails_early_when_tmux_is_unavailable() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("feature", "complete");

    let mut workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    workspace.config.workspace.tmux = true;
    workspace.config.workspace.tmux_session = "ralph-test".to_owned();

    fn tmux_unavailable() -> ralph::Result<()> {
        Err(RalphError::TmuxUnavailable)
    }

    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator.set_tmux_preflight_checker(tmux_unavailable);
    let result = orchestrator
        .run(run_options(&project_id))
        .await
        .expect_err("run should fail without tmux");
    assert!(
        matches!(result, RalphError::TmuxUnavailable),
        "expected tmux unavailable error, got {result:?}"
    );
}

#[tokio::test]
async fn tmux_mode_dry_run_skips_tmux_availability_check() {
    let (_temp, workspace_root, project_id, _script) =
        setup_workspace_with_project("feature", "complete");

    let mut workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    workspace.config.workspace.tmux = true;
    workspace.config.workspace.tmux_session = "ralph-test".to_owned();

    fn tmux_unavailable() -> ralph::Result<()> {
        Err(RalphError::TmuxUnavailable)
    }

    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator.set_tmux_preflight_checker(tmux_unavailable);
    let mut options = run_options(&project_id);
    options.dry_run = true;

    let result = orchestrator.run(options).await;
    assert!(
        result.is_ok(),
        "dry-run should skip tmux preflight: {result:?}"
    );
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

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    // Dirty-tree rejection prevents any loops; reconstruction defaults to loop 1.
    assert!(
        state.loops.is_empty(),
        "loops should be empty after dirty-tree rejection"
    );
    assert_eq!(state.current_loop, 1);
    assert_eq!(state.current_phase, Phase::Planning);
    assert_eq!(state.phase_iteration, 1);
    assert_eq!(state.status, ProjectStatus::Pending);
}

// ---------------------------------------------------------------------------
// Two-loop happy-path test with separate Claude / Codex mock scripts
// ---------------------------------------------------------------------------

/// Write `mock_claude.sh`.
///
/// Planner behaviour (tracked via `$COUNTER_DIR/claude_planner_count`):
///   - 1st call → `# Feature: Auth Module`
///   - 2nd call → `# Project Completion Request`
///
/// Implementer / reviewer / completer behave identically to the shared mock.
fn write_claude_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  counter_file="${COUNTER_DIR}/claude_planner_count"
  count=0
  if [ -f "$counter_file" ]; then
    count=$(cat "$counter_file")
  fi
  count=$((count + 1))
  echo "$count" > "$counter_file"

  if [ "$count" -ge 2 ]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
All project requirements have been satisfied.

## Summary of Work
- Built Auth Module and API Layer.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: Auth Module

## Description
Implement authentication module.

## Acceptance Criteria
- [ ] Auth behavior exists

## Files to Modify/Create
- `README.md` - Document the auth module

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
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Feature behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: auth module
EOF

elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Auth Module: satisfied
- API Layer: satisfied
EOF

elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock check: passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "claude: unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write claude script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

/// Write `mock_codex.sh`.
///
/// Planner behaviour (tracked via `$COUNTER_DIR/codex_planner_count`):
///   - 1st call → `# Feature: API Layer`
///   - 2nd call → `# Project Completion Request`
///
/// Implementer / reviewer / completer behave identically to the shared mock.
fn write_codex_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  counter_file="${COUNTER_DIR}/codex_planner_count"
  count=0
  if [ -f "$counter_file" ]; then
    count=$(cat "$counter_file")
  fi
  count=$((count + 1))
  echo "$count" > "$counter_file"

  if [ "$count" -ge 2 ]; then
    cat <<'EOF'
# Project Completion Request

## Rationale
All project requirements have been satisfied.

## Summary of Work
- Built Auth Module and API Layer.

## Remaining Items
- None
EOF
  else
    cat <<'EOF'
# Feature: API Layer

## Description
Implement the API layer.

## Acceptance Criteria
- [ ] API behavior exists

## Files to Modify/Create
- `README.md` - Document the API layer

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
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Feature behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: api layer
EOF

elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Auth Module: satisfied
- API Layer: satisfied
EOF

elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock check: passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "codex: unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write codex script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

/// Set up a workspace where `claude` and `codex` backends point at **separate**
/// mock scripts so that backend alternation is exercised with distinct outputs.
fn setup_workspace_with_split_backends() -> (TempDir, PathBuf, String) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    // Initialise git repo
    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);

    fs::write(repo_root.join("README.md"), "# demo\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    // Write separate mock scripts
    let claude_script = repo_root.join("mock_claude.sh");
    write_claude_script(&claude_script);

    let codex_script = repo_root.join("mock_codex.sh");
    write_codex_script(&codex_script);

    git_ok(repo_root, &["add", "mock_claude.sh", "mock_codex.sh"]);
    git_ok(
        repo_root,
        &["commit", "-m", "test: add split backend mocks"],
    );

    add_local_bare_remote(repo_root);

    // Initialise workspace
    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    fs::write(
        workspace_root.join("templates/spec.md"),
        default_planner_template(),
    )
    .expect("write spec template");
    fs::write(
        workspace_root.join("templates/implementation.md"),
        default_implementer_template(),
    )
    .expect("write implementation template");
    fs::write(
        workspace_root.join("templates/review.md"),
        default_reviewer_template(),
    )
    .expect("write review template");
    fs::write(
        workspace_root.join("templates/completion.md"),
        default_completer_template(),
    )
    .expect("write completion template");
    fs::write(
        workspace_root.join("templates/qa.md"),
        default_qa_template(),
    )
    .expect("write qa template");

    // Counter directory – lives inside the temp dir so scripts can track calls
    let counter_dir = repo_root.join("counters");
    fs::create_dir_all(&counter_dir).expect("create counter dir");

    // Configure claude backend → mock_claude.sh
    let mut claude_env = BTreeMap::new();
    claude_env.insert(
        "COUNTER_DIR".to_owned(),
        counter_dir.to_string_lossy().to_string(),
    );
    workspace.config.backends.claude.command = claude_script.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = claude_env;

    // Configure codex backend → mock_codex.sh
    let mut codex_env = BTreeMap::new();
    codex_env.insert(
        "COUNTER_DIR".to_owned(),
        counter_dir.to_string_lossy().to_string(),
    );
    workspace.config.backends.codex.command = codex_script.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = codex_env;

    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.config.workflow.final_review_enabled = false;
    workspace.config.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.save_config().expect("save config");

    // Create project
    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo system\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "issue-1".to_owned();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id)
}

#[tokio::test]
async fn two_loop_happy_path_with_separate_backends() {
    let (_temp, workspace_root, project_id) = setup_workspace_with_split_backends();

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.loops = None;
    options.until_complete = true;

    orchestrator
        .run(options)
        .await
        .expect("two-loop orchestration should succeed");

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");

    // --- 1. Loop count ---
    assert_eq!(
        state.loops.len(),
        2,
        "expected exactly 2 feature loops, got {}",
        state.loops.len()
    );

    // --- 2. Backend alternation (with role-model injection from defaults) ---
    // Loop 1 (odd): planner=claude, implementer=codex, reviewer=claude
    assert_eq!(state.loops[0].backends.planner, "claude(opus)");
    assert_eq!(
        state.loops[0].backends.implementer,
        "codex(gpt-5.3-codex-high)"
    );
    assert_eq!(state.loops[0].backends.reviewer, "claude(opus)");
    // Loop 2 (even): planner=codex, implementer=claude, reviewer=codex
    assert_eq!(
        state.loops[1].backends.planner,
        "codex(gpt-5.3-codex-xhigh)"
    );
    assert_eq!(state.loops[1].backends.implementer, "claude(opus)");
    assert_eq!(
        state.loops[1].backends.reviewer,
        "codex(gpt-5.3-codex-high)"
    );

    // --- 3. Feature names ---
    assert_eq!(state.loops[0].feature_name, "Auth Module");
    assert!(
        state.loops[0].slug.contains("auth-module"),
        "loop 1 slug should contain 'auth-module', got '{}'",
        state.loops[0].slug
    );
    assert_eq!(state.loops[1].feature_name, "API Layer");
    assert!(
        state.loops[1].slug.contains("api-layer"),
        "loop 2 slug should contain 'api-layer', got '{}'",
        state.loops[1].slug
    );

    // --- 4. All loops completed with commit hashes ---
    assert_eq!(state.loops[0].status, LoopStatus::Completed);
    assert!(
        state.loops[0].commit.is_some(),
        "loop 1 should have a commit hash"
    );
    assert_eq!(state.loops[1].status, LoopStatus::Completed);
    assert!(
        state.loops[1].commit.is_some(),
        "loop 2 should have a commit hash"
    );

    // --- 5. Checkpoint commits ---
    let repo_root = workspace_root.parent().expect("repo root");
    let log1 = git_output(
        repo_root,
        &["log", "--oneline", "--all", "--grep=ralph(issue-1): loop 1"],
    );
    assert!(
        !log1.is_empty(),
        "expected a Ralph checkpoint commit for loop 1"
    );
    let log2 = git_output(
        repo_root,
        &["log", "--oneline", "--all", "--grep=ralph(issue-1): loop 2"],
    );
    assert!(
        !log2.is_empty(),
        "expected a Ralph checkpoint commit for loop 2"
    );

    // --- 6. Artifacts ---
    // Loop 1: spec, impl-notes, approval
    assert_timestamped_artifact(&state.loops[0].artifacts.spec, "spec.md");
    assert_timestamped_artifact(
        state.loops[0]
            .artifacts
            .impl_notes
            .as_deref()
            .expect("loop 1 should have impl-notes"),
        "impl-notes.md",
    );
    assert_timestamped_artifact(
        state.loops[0]
            .artifacts
            .approval
            .as_deref()
            .expect("loop 1 should have approval"),
        "review-approved.md",
    );
    // Loop 2: spec, impl-notes, approval
    assert_timestamped_artifact(&state.loops[1].artifacts.spec, "spec.md");
    assert_timestamped_artifact(
        state.loops[1]
            .artifacts
            .impl_notes
            .as_deref()
            .expect("loop 2 should have impl-notes"),
        "impl-notes.md",
    );
    assert_timestamped_artifact(
        state.loops[1]
            .artifacts
            .approval
            .as_deref()
            .expect("loop 2 should have approval"),
        "review-approved.md",
    );

    // --- 7. Completion ---
    assert_eq!(state.status, ProjectStatus::Completed);
    assert_eq!(
        state.completion_attempts.len(),
        1,
        "expected exactly 1 completion attempt"
    );
    assert_eq!(
        state.completion_attempts[0].verdict,
        Some(CompletionVerdict::Complete)
    );
    assert_timestamped_artifact(
        &state.completion_attempts[0].artifacts.termination_request,
        "termination-request.md",
    );
    let verdict_path = state.completion_attempts[0]
        .artifacts
        .verdict
        .as_deref()
        .expect("completion verdict artifact should exist");
    assert!(
        verdict_path.contains("completer-verdict"),
        "verdict artifact should contain completer-verdict: {verdict_path}"
    );
}

// ---------------------------------------------------------------------------
// Review iteration limit rollback test
// ---------------------------------------------------------------------------

fn write_suggestions_backend_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
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
elif [[ "$prompt" == *"You are a software developer implementing a feature specification."* ]]; then
  if [[ -n "${TEST_REPO_ROOT:-}" ]]; then
    cat <<'EOF' > "${TEST_REPO_ROOT}/new_module.rs"
pub fn generated_for_review_limit_test() {}
EOF
  fi
  if [[ "$prompt" == *"Review Feedback (if responding to review)"* && "$prompt" == *"Required Changes"* ]]; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Addressed reviewer feedback

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Kept implementation minimal to satisfy the spec.

## Spec Deviations
- None

## Testing
- cargo test
EOF
  fi
elif [[ "$prompt" == *"You are a code reviewer ensuring implementations match specifications."* ]]; then
  cat <<'EOF'
# Review: SUGGESTIONS

## Required Changes
1. **Demo check**: tighten behavior
   - Current: loose
   - Expected: strict
   - Reference: spec
EOF
elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock check: passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write suggestions backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn setup_workspace_with_always_suggestions() -> (TempDir, PathBuf, String) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);

    fs::write(repo_root.join("README.md"), "# demo\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    let script_path = repo_root.join("mock_backend.sh");
    write_suggestions_backend_script(&script_path);
    git_ok(repo_root, &["add", "mock_backend.sh"]);
    git_ok(repo_root, &["commit", "-m", "test: add backend mock"]);

    add_local_bare_remote(repo_root);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    fs::write(
        workspace_root.join("templates/spec.md"),
        default_planner_template(),
    )
    .expect("write spec template");
    fs::write(
        workspace_root.join("templates/implementation.md"),
        default_implementer_template(),
    )
    .expect("write implementation template");
    fs::write(
        workspace_root.join("templates/review.md"),
        default_reviewer_template(),
    )
    .expect("write review template");
    fs::write(
        workspace_root.join("templates/completion.md"),
        default_completer_template(),
    )
    .expect("write completion template");
    fs::write(
        workspace_root.join("templates/qa.md"),
        default_qa_template(),
    )
    .expect("write qa template");

    let mut env = BTreeMap::new();
    env.insert("PLANNER_MODE".to_owned(), "feature".to_owned());
    env.insert("REVIEW_MODE".to_owned(), "suggestions".to_owned());
    env.insert("COMPLETER_MODE".to_owned(), "complete".to_owned());
    env.insert(
        "TEST_REPO_ROOT".to_owned(),
        repo_root.to_string_lossy().to_string(),
    );

    workspace.config.backends.claude.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = env.clone();

    workspace.config.backends.codex.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = env;

    workspace.config.workflow.max_review_iterations = 1;
    workspace.config.workflow.final_review_enabled = false;

    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.config.workflow.final_review_enabled = false;
    workspace.config.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo system\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "issue-1".to_owned();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id)
}

#[tokio::test]
async fn review_iteration_limit_rollback() {
    let (_temp, workspace_root, project_id) = setup_workspace_with_always_suggestions();

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let options = run_options(&project_id);

    let result = orchestrator.run(options).await;
    assert!(
        result.is_err(),
        "run should fail with ReviewIterationLimitExceeded"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            RalphError::ReviewIterationLimitExceeded {
                loop_number: 1,
                max_iterations: 1,
            }
        ),
        "expected ReviewIterationLimitExceeded, got {err:?}"
    );

    // Verify state was rolled back cleanly
    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert!(
        state.loops.is_empty(),
        "loops should be empty after rollback, got {} loop(s)",
        state.loops.len()
    );
    assert_eq!(
        state.current_phase,
        Phase::Planning,
        "phase should be reset to Planning"
    );
    assert_eq!(
        state.phase_iteration, 1,
        "phase_iteration should be reset to 1"
    );
    assert_eq!(
        state.current_loop, 1,
        "current_loop should be 1 after rollback (no-checkpoint default)"
    );
    // With checkpoint commits, new_module.rs is committed during the
    // implementing→reviewing phase transition and persists in git history
    // after rollback.  The rollback cleans untracked files and resets
    // the working tree, but committed files remain.

    let workspace = Workspace::load(workspace_root.clone()).expect("reload workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let rerun_result = orchestrator.run(run_options(&project_id)).await;
    assert!(
        rerun_result.is_err(),
        "rerun should still fail at review limit with this fixture"
    );
    assert!(
        matches!(
            rerun_result.unwrap_err(),
            RalphError::ReviewIterationLimitExceeded {
                loop_number: 1,
                max_iterations: 1
            }
        ),
        "rerun should not be blocked by dirty-tree validation"
    );
}

fn write_parse_retry_claude_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"CRITICAL: Your previous response could not be parsed."* ]]; then
  counter_file="${COUNTER_DIR}/claude_reformat_count"
  count=0
  if [ -f "$counter_file" ]; then
    count=$(cat "$counter_file")
  fi
  count=$((count + 1))
  echo "$count" > "$counter_file"
  cat <<'EOF'
this response is still not parseable
EOF
elif [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  cat <<'EOF'
planner response without required markdown structure
EOF
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
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Demo behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: parse retry test
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Demo requirement satisfied
EOF
elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock check: passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "claude: unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write parse-retry claude script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn write_parse_retry_codex_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"CRITICAL: Your previous response could not be parsed."* ]]; then
  counter_file="${COUNTER_DIR}/codex_reformat_count"
  count=0
  if [ -f "$counter_file" ]; then
    count=$(cat "$counter_file")
  fi
  count=$((count + 1))
  echo "$count" > "$counter_file"
  args=" $* "
  if [[ "$args" == *' -c model_reasoning_effort="medium" '* && "$args" == *" --model gpt-5.3-codex "* ]]; then
    model_counter_file="${COUNTER_DIR}/codex_reformat_model_count"
    model_count=0
    if [ -f "$model_counter_file" ]; then
      model_count=$(cat "$model_counter_file")
    fi
    model_count=$((model_count + 1))
    echo "$model_count" > "$model_counter_file"
  fi
  cat <<'EOF'
# Feature: Reformat Rescue Feature

## Description
Produce parseable planner output during reformat retry.

## Acceptance Criteria
- [ ] Output is parseable

## Files to Modify/Create
- `README.md` - Document parse retry behavior

## Dependencies
- Requires: none
- Blocks: none
EOF
elif [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  cat <<'EOF'
# Feature: Fallback Feature

## Description
Fallback planner output.

## Acceptance Criteria
- [ ] Output is parseable

## Files to Modify/Create
- `README.md` - Document fallback behavior

## Dependencies
- Requires: none
- Blocks: none
EOF
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
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Demo behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: parse retry test
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Demo requirement satisfied
EOF
elif [[ "$prompt" == *"You are a QA engineer"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock check: passed

## Acceptance Criteria Verification
All acceptance criteria verified.
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "codex: unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write parse-retry codex script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn setup_workspace_for_reformat_backend_test() -> (TempDir, PathBuf, String, PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);

    fs::write(repo_root.join("README.md"), "# demo\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    let claude_script = repo_root.join("mock_claude.sh");
    write_parse_retry_claude_script(&claude_script);
    let codex_script = repo_root.join("mock_codex.sh");
    write_parse_retry_codex_script(&codex_script);
    git_ok(repo_root, &["add", "mock_claude.sh", "mock_codex.sh"]);
    git_ok(
        repo_root,
        &["commit", "-m", "test: add parse retry backend mocks"],
    );

    add_local_bare_remote(repo_root);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    fs::write(
        workspace_root.join("templates/spec.md"),
        default_planner_template(),
    )
    .expect("write spec template");
    fs::write(
        workspace_root.join("templates/implementation.md"),
        default_implementer_template(),
    )
    .expect("write implementation template");
    fs::write(
        workspace_root.join("templates/review.md"),
        default_reviewer_template(),
    )
    .expect("write review template");
    fs::write(
        workspace_root.join("templates/completion.md"),
        default_completer_template(),
    )
    .expect("write completion template");
    fs::write(
        workspace_root.join("templates/qa.md"),
        default_qa_template(),
    )
    .expect("write qa template");

    let counter_dir = repo_root.join("counters");
    fs::create_dir_all(&counter_dir).expect("create counter dir");

    let mut claude_env = BTreeMap::new();
    claude_env.insert(
        "COUNTER_DIR".to_owned(),
        counter_dir.to_string_lossy().to_string(),
    );
    workspace.config.backends.claude.command = claude_script.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = claude_env;

    let mut codex_env = BTreeMap::new();
    codex_env.insert(
        "COUNTER_DIR".to_owned(),
        counter_dir.to_string_lossy().to_string(),
    );
    workspace.config.backends.codex.command = codex_script.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = codex_env;

    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.config.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo system\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "issue-1".to_owned();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id, counter_dir)
}

#[tokio::test]
async fn parse_retry_reformat_uses_opposite_backend() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_reformat_backend_test();

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("orchestration should succeed with opposite-backend reformat");

    let codex_count = fs::read_to_string(counter_dir.join("codex_reformat_count"))
        .expect("codex should receive the reformat attempt");
    assert_eq!(
        codex_count.trim(),
        "1",
        "opposite backend should be used exactly once for planner reformat"
    );
    let codex_model_count = fs::read_to_string(counter_dir.join("codex_reformat_model_count"))
        .expect("codex reformat should use the configured reformatter model");
    assert_eq!(
        codex_model_count.trim(),
        "1",
        "reformat retry should target codex(gpt-5.3-codex-medium), not bare codex"
    );

    let claude_reformat_counter = counter_dir.join("claude_reformat_count");
    assert!(
        !claude_reformat_counter.exists(),
        "original backend should not receive planner reformat attempt"
    );
}

#[tokio::test]
async fn parse_retry_reformat_without_role_model_uses_bare_opposite_backend() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_reformat_backend_test();

    let mut workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    workspace.config.backends.claude.models.reformatter = None;
    workspace.config.backends.codex.models.reformatter = None;

    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("orchestration should succeed with bare opposite-backend reformat");

    let codex_count = fs::read_to_string(counter_dir.join("codex_reformat_count"))
        .expect("codex should receive the reformat attempt");
    assert_eq!(
        codex_count.trim(),
        "1",
        "opposite backend should still be used when reformatter models are unset"
    );

    let codex_model_counter = counter_dir.join("codex_reformat_model_count");
    assert!(
        !codex_model_counter.exists(),
        "bare opposite backend should be used when no reformatter role model is configured"
    );
}

// ---------------------------------------------------------------------------
// QA phase tests
// ---------------------------------------------------------------------------

/// Backend script that supports QA: always passes QA on first try.
fn write_qa_pass_backend_script(path: &Path) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
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
elif [[ "$prompt" == *"You are a QA engineer validating"* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary manually: passed

## Automated Tests
- cargo check: passed
- cargo test: passed

## Acceptance Criteria Verification
All acceptance criteria verified. Build and tests pass.
EOF
elif [[ "$prompt" == *"You are a code reviewer ensuring implementations match specifications."* ]]; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Demo behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: demo feature with QA
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"#;

    fs::write(path, script).expect("write qa-pass backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

/// Backend script that fails QA on first iteration, then passes on second.
fn write_qa_fail_then_pass_backend_script(path: &Path) {
    let script = r##"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
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
elif [[ "$prompt" == *"You are a software developer implementing a feature specification."* ]]; then
  if [[ "$prompt" == *"# QA: FAIL"* ]]; then
    cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Fixed QA failure by addressing reported issues.

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Kept implementation minimal to satisfy the spec.

## Spec Deviations
- None

## Testing
- cargo test
EOF
  fi
elif [[ "$prompt" == *"You are a QA engineer validating"* ]]; then
  counter_file="${COUNTER_DIR}/qa_attempt_count"
  count=0
  if [ -f "$counter_file" ]; then
    count=$(cat "$counter_file")
  fi
  count=$((count + 1))
  echo "$count" > "$counter_file"

  if [ "$count" -le 1 ]; then
    cat <<'EOF'
# QA: FAIL

## Failures
1. cargo test fails: 2 test failures in integration tests

## Suggested Fixes
1. Fix test assertion in tests/integration.rs line 42
EOF
  else
    cat <<'EOF'
# QA: PASS

## Manual Testing
- ran binary manually: passed (after fix)

## Automated Tests
- cargo check: passed
- cargo test: passed (after fix)

## Acceptance Criteria Verification
All acceptance criteria now verified after implementer fix.
EOF
  fi
elif [[ "$prompt" == *"You are a code reviewer ensuring implementations match specifications."* ]]; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Demo behavior exists

## Notes
Implementation satisfies the specification.

## Commit Message
feat: demo feature after QA retry
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"##;

    fs::write(path, script).expect("write qa-fail-then-pass backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

/// Backend script that always fails QA (for limit testing).
fn write_qa_always_fail_backend_script(path: &Path) {
    let script = r##"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
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
elif [[ "$prompt" == *"You are a software developer implementing a feature specification."* ]]; then
  if [[ "$prompt" == *"# QA: FAIL"* ]]; then
    # Read iteration from counter file
    iter_file="${COUNTER_DIR}/qa_response_iteration"
    iteration=1
    if [ -f "$iter_file" ]; then
      iteration=$(cat "$iter_file")
    fi
    next=$((iteration + 1))
    echo "$next" > "$iter_file"
    cat <<EOF
# Implementation Response (Iteration $iteration)

## Changes Made
1. Attempted to fix QA failure.

## Could Not Address
- None
EOF
  else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Kept implementation minimal to satisfy the spec.

## Spec Deviations
- None

## Testing
- cargo test
EOF
  fi
elif [[ "$prompt" == *"You are a QA engineer validating"* ]]; then
  cat <<'EOF'
# QA: FAIL

## Failures
1. cargo test still fails

## Suggested Fixes
1. Fix remaining test failures
EOF
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"##;

    fs::write(path, script).expect("write qa-always-fail backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn setup_workspace_with_qa(
    qa_script_writer: fn(&Path),
    qa_enabled: bool,
    max_qa_iterations: u32,
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
    qa_script_writer(&script_path);
    git_ok(repo_root, &["add", "mock_backend.sh"]);
    git_ok(repo_root, &["commit", "-m", "test: add backend mock"]);

    add_local_bare_remote(repo_root);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");
    fs::write(
        workspace_root.join("templates/spec.md"),
        default_planner_template(),
    )
    .expect("write spec template");
    fs::write(
        workspace_root.join("templates/implementation.md"),
        default_implementer_template(),
    )
    .expect("write implementation template");
    fs::write(
        workspace_root.join("templates/review.md"),
        default_reviewer_template(),
    )
    .expect("write review template");
    fs::write(
        workspace_root.join("templates/completion.md"),
        default_completer_template(),
    )
    .expect("write completion template");
    fs::write(
        workspace_root.join("templates/qa.md"),
        default_qa_template(),
    )
    .expect("write qa template");

    let counter_dir = repo_root.join("counters");
    fs::create_dir_all(&counter_dir).expect("create counter dir");

    let mut env = BTreeMap::new();
    env.insert(
        "COUNTER_DIR".to_owned(),
        counter_dir.to_string_lossy().to_string(),
    );

    workspace.config.backends.claude.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = env.clone();

    workspace.config.backends.codex.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = env;

    workspace.config.workflow.qa_enabled = qa_enabled;
    workspace.config.workflow.max_qa_iterations = max_qa_iterations;
    workspace.config.workflow.final_review_enabled = false;

    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.config.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a demo system\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "issue-1".to_owned();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id, counter_dir)
}

fn write_acceptance_pass_backend_script(path: &Path) {
    let script = r####"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  planner_counter="${COUNTER_DIR}/planner_count"
  planner_count=0
  if [ -f "$planner_counter" ]; then
    planner_count=$(cat "$planner_counter")
  fi
  planner_count=$((planner_count + 1))
  echo "$planner_count" > "$planner_counter"
  printf "%s" "$prompt" > "${COUNTER_DIR}/planner_prompt_${planner_count}.md"

  cat <<'EOF'
# Project Completion Request

## Rationale
All requirements are already satisfied.

## Summary of Work
- Existing completed work satisfies the prompt.

## Remaining Items
- None
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement alpha: satisfied by existing implementation
EOF
elif [[ "$prompt" == *"You are a QA engineer validating overall project acceptance."* ]]; then
  qa_counter="${COUNTER_DIR}/acceptance_qa_count"
  qa_count=0
  if [ -f "$qa_counter" ]; then
    qa_count=$(cat "$qa_counter")
  fi
  qa_count=$((qa_count + 1))
  echo "$qa_count" > "$qa_counter"

  missing=0
  if [[ "$prompt" != *"### Master Prompt"* ]]; then missing=1; fi
  if [[ "$prompt" != *"### Completed Feature Loop Summary"* ]]; then missing=1; fi
  if [[ "$prompt" != *"Use your tools to explore the actual source code"* ]]; then missing=1; fi
  if [[ "$prompt" != *"Verify overall project acceptance, not just a single feature."* ]]; then missing=1; fi

  if [ "$missing" -eq 0 ]; then
    cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance verification: passed manually

## Automated Tests
- acceptance verification checklist: passed

## Acceptance Criteria Verification
Overall project acceptance verified from master prompt, completed-loop summary, and tool-based exploration.
EOF
  else
    cat <<'EOF'
# QA: FAIL

## Failures
1. Acceptance prompt is missing required context sections.

## Suggested Fixes
1. Include master prompt, completed-loop summary, and tool-exploration instruction.
EOF
  fi
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"####;

    fs::write(path, script).expect("write acceptance-pass backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn write_acceptance_fail_then_pass_backend_script(path: &Path) {
    let script = r###"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  planner_counter="${COUNTER_DIR}/planner_count"
  planner_count=0
  if [ -f "$planner_counter" ]; then
    planner_count=$(cat "$planner_counter")
  fi
  planner_count=$((planner_count + 1))
  echo "$planner_count" > "$planner_counter"
  printf "%s" "$prompt" > "${COUNTER_DIR}/planner_prompt_${planner_count}.md"

  cat <<'EOF'
# Project Completion Request

## Rationale
The project appears complete and should be validated for final acceptance.

## Summary of Work
- Completed the planned implementation.

## Remaining Items
- None
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement alpha: satisfied by completion attempt 1
EOF
elif [[ "$prompt" == *"You are a QA engineer validating overall project acceptance."* ]]; then
  qa_counter="${COUNTER_DIR}/acceptance_qa_count"
  qa_count=0
  if [ -f "$qa_counter" ]; then
    qa_count=$(cat "$qa_counter")
  fi
  qa_count=$((qa_count + 1))
  echo "$qa_count" > "$qa_counter"

  if [ "$qa_count" -eq 1 ]; then
    cat <<'EOF'
# QA: FAIL

## Failures
1. Global acceptance check failed: missing end-to-end acceptance evidence.

## Suggested Fixes
1. Re-run completion planning and provide full acceptance evidence.
EOF
  else
    cat <<'EOF'
# QA: PASS

## Manual Testing
- final acceptance verification: passed manually

## Automated Tests
- final acceptance checklist: passed

## Acceptance Criteria Verification
Project-level acceptance now passes after feedback.
EOF
  fi
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###;

    fs::write(path, script).expect("write acceptance-fail-then-pass backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn setup_workspace_for_acceptance_gate(
    script_writer: fn(&Path),
) -> (TempDir, PathBuf, String, PathBuf) {
    let (temp, workspace_root, project_id, _counter_dir) =
        setup_workspace_with_qa(script_writer, true, 3);

    // Keep test counters under `.ralph/` so they are excluded from dirty-tree
    // checks when acceptance failure routes back to planning.
    let counter_dir = workspace_root.join("counters");
    fs::create_dir_all(&counter_dir).expect("create acceptance counter dir");

    let mut workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let counter_dir_string = counter_dir.to_string_lossy().to_string();
    workspace
        .config
        .backends
        .claude
        .env
        .insert("COUNTER_DIR".to_owned(), counter_dir_string.clone());
    workspace
        .config
        .backends
        .codex
        .env
        .insert("COUNTER_DIR".to_owned(), counter_dir_string);
    workspace.save_config().expect("save workspace config");

    (temp, workspace_root, project_id, counter_dir)
}

#[tokio::test]
async fn qa_disabled_skips_phase() {
    let (_temp, workspace_root, project_id, _counter_dir) =
        setup_workspace_with_qa(write_qa_pass_backend_script, false, 3);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("orchestration should succeed");

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.loops[0].status, LoopStatus::Completed);
    // QA results should be empty when QA is disabled
    assert!(
        state.loops[0].artifacts.qa_results.is_empty(),
        "QA results should be empty when QA is disabled"
    );
    assert!(state.loops[0].commit.is_some());
}

#[tokio::test]
async fn qa_pass_proceeds_to_review() {
    let (_temp, workspace_root, project_id, _counter_dir) =
        setup_workspace_with_qa(write_qa_pass_backend_script, true, 3);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("orchestration with QA should succeed");

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.loops[0].status, LoopStatus::Completed);
    assert!(state.loops[0].commit.is_some());

    // QA should have passed with one exchange
    assert_eq!(
        state.loops[0].artifacts.qa_results.len(),
        1,
        "expected exactly 1 QA exchange"
    );
    assert!(
        state.loops[0].artifacts.qa_results[0].passed,
        "QA should have passed"
    );
    assert!(
        state.loops[0].artifacts.pending_qa_feedback.is_none(),
        "pending_qa_feedback should be cleared after pass"
    );
}

#[tokio::test]
async fn qa_fail_retries_implementer_then_passes() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_with_qa(write_qa_fail_then_pass_backend_script, true, 3);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_options(&project_id))
        .await
        .expect("orchestration with QA fail-then-pass should succeed");

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.loops[0].status, LoopStatus::Completed);
    assert!(state.loops[0].commit.is_some());

    // QA should have 2 exchanges: first fail, then pass
    assert_eq!(
        state.loops[0].artifacts.qa_results.len(),
        2,
        "expected 2 QA exchanges (fail + pass)"
    );
    assert!(
        !state.loops[0].artifacts.qa_results[0].passed,
        "first QA should have failed"
    );
    assert!(
        state.loops[0].artifacts.qa_results[0]
            .implementer_response
            .is_some(),
        "first QA failure should have implementer response"
    );
    // Verify the implementer response artifact is specifically an impl-qa-response path.
    let response_path = state.loops[0].artifacts.qa_results[0]
        .implementer_response
        .as_ref()
        .expect("implementer_response should be set");
    assert!(
        response_path.contains("impl-qa-response-"),
        "implementer response artifact should be an impl-qa-response file, got: {response_path}"
    );
    assert!(
        state.loops[0].artifacts.qa_results[1].passed,
        "second QA should have passed"
    );

    // Verify counter dir shows 2 QA attempts
    let qa_count =
        fs::read_to_string(counter_dir.join("qa_attempt_count")).expect("qa counter should exist");
    assert_eq!(
        qa_count.trim(),
        "2",
        "QA should have been invoked exactly twice"
    );
}

#[tokio::test]
async fn qa_limit_exceeded_rolls_back() {
    let (_temp, workspace_root, project_id, _counter_dir) =
        setup_workspace_with_qa(write_qa_always_fail_backend_script, true, 1);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let result = orchestrator.run(run_options(&project_id)).await;

    assert!(
        result.is_err(),
        "run should fail with QaIterationLimitExceeded"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            RalphError::QaIterationLimitExceeded {
                loop_number: 1,
                max_iterations: 1,
            }
        ),
        "expected QaIterationLimitExceeded, got {err:?}"
    );

    // Verify state was rolled back
    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert!(
        state.loops.is_empty(),
        "loops should be empty after QA limit rollback"
    );
    assert_eq!(state.current_phase, Phase::Planning);
    assert_eq!(state.phase_iteration, 1);
    assert_eq!(
        state.current_loop, 1,
        "current_loop should be 1 after QA limit rollback (no-checkpoint default)"
    );
}

// resume_from_phase_qa was removed: it depended on save_project_state to
// manually persist mid-phase state, which is no longer supported.  Phase
// resume is now governed by artifact-based reconstruction in lifecycle.rs.

#[tokio::test]
async fn acceptance_gate_pass_keeps_completed() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_acceptance_gate(write_acceptance_pass_backend_script);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.loops = None;
    options.until_complete = true;

    orchestrator
        .run(options)
        .await
        .expect("orchestration with acceptance-pass should succeed");

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert_eq!(state.status, ProjectStatus::Completed);
    assert_eq!(state.completion_attempts.len(), 1);

    let attempt = &state.completion_attempts[0];
    assert_eq!(attempt.verdict, Some(CompletionVerdict::Complete));
    assert_acceptance_results_cover_both_families(&attempt.artifacts.acceptance_results);
    assert!(
        attempt
            .artifacts
            .acceptance_results
            .iter()
            .all(|result| result.passed),
        "all acceptance results should be PASS"
    );
    for result in &attempt.artifacts.acceptance_results {
        assert_timestamped_acceptance_artifact(&result.artifact, "acceptance-pass");
    }

    let acceptance_qa_count = fs::read_to_string(counter_dir.join("acceptance_qa_count"))
        .expect("acceptance QA counter should exist");
    assert_eq!(
        acceptance_qa_count.trim(),
        "2",
        "acceptance QA should run once per required backend family on COMPLETE"
    );

    // Completion artifacts should be auto-committed
    let repo_root = workspace_root.parent().expect("repo root");
    let status_output = Command::new("git")
        .args(["status", "--porcelain", ".ralph/"])
        .current_dir(repo_root)
        .output()
        .expect("git status should execute");
    let uncommitted = String::from_utf8_lossy(&status_output.stdout);
    let uncommitted_lines: Vec<&str> = uncommitted
        .lines()
        .filter(|l| !l.is_empty() && *l != "?? .ralph/")
        .collect();
    assert!(
        uncommitted_lines.is_empty(),
        "expected no uncommitted .ralph/ files after completion, found:\n{}",
        uncommitted_lines.join("\n")
    );
}

#[tokio::test]
async fn acceptance_gate_fail_overrides_complete_to_continue() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_acceptance_gate(write_acceptance_fail_then_pass_backend_script);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.loops = None;
    options.until_complete = true;

    orchestrator
        .run(options)
        .await
        .expect("orchestration with acceptance fail-then-pass should succeed");

    let state = reconstruct_project_state_from_project_dir(
        &workspace_root.join("projects").join(&project_id),
    )
    .expect("load project state");
    assert_eq!(state.completion_attempts.len(), 2);

    let first_attempt = &state.completion_attempts[0];
    assert_eq!(
        first_attempt.verdict,
        Some(CompletionVerdict::Continue),
        "acceptance failure should force CONTINUE even when completer said COMPLETE"
    );
    assert_acceptance_results_cover_both_families(&first_attempt.artifacts.acceptance_results);
    let first_fail_count = first_attempt
        .artifacts
        .acceptance_results
        .iter()
        .filter(|result| !result.passed)
        .count();
    assert_eq!(
        first_fail_count, 1,
        "first attempt should include one failing acceptance backend"
    );
    for result in &first_attempt.artifacts.acceptance_results {
        let expected_base = if result.passed {
            "acceptance-pass"
        } else {
            "acceptance-fail"
        };
        assert_timestamped_acceptance_artifact(&result.artifact, expected_base);
    }

    let second_attempt = &state.completion_attempts[1];
    assert_eq!(second_attempt.verdict, Some(CompletionVerdict::Complete));
    assert_acceptance_results_cover_both_families(&second_attempt.artifacts.acceptance_results);
    assert!(
        second_attempt
            .artifacts
            .acceptance_results
            .iter()
            .all(|result| result.passed),
        "second attempt acceptance results should all be PASS"
    );
    assert_eq!(state.status, ProjectStatus::Completed);

    let acceptance_qa_count = fs::read_to_string(counter_dir.join("acceptance_qa_count"))
        .expect("acceptance QA counter should exist");
    assert_eq!(
        acceptance_qa_count.trim(),
        "4",
        "acceptance QA should run once per backend family for each completion attempt"
    );
}

#[tokio::test]
async fn planner_receives_acceptance_failure_context() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_acceptance_gate(write_acceptance_fail_then_pass_backend_script);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let mut options = run_options(&project_id);
    options.loops = None;
    options.until_complete = true;

    orchestrator
        .run(options)
        .await
        .expect("orchestration with acceptance fail-then-pass should succeed");

    let planner_prompt_after_fail = fs::read_to_string(counter_dir.join("planner_prompt_2.md"))
        .expect("second planner prompt should be captured");

    assert!(
        planner_prompt_after_fail.contains("## Completion Feedback"),
        "planner prompt should include completion feedback section after acceptance failure"
    );
    assert!(
        planner_prompt_after_fail.contains("Requirement alpha: satisfied by completion attempt 1"),
        "planner prompt should include completer verdict artifact content"
    );
    assert!(
        planner_prompt_after_fail
            .contains("Global acceptance check failed: missing end-to-end acceptance evidence."),
        "planner prompt should include acceptance-fail artifact content"
    );
}

fn write_final_review_backend_script(path: &Path) {
    let script = r###"#!/usr/bin/env bash
set -euo pipefail

prompt="$(cat)"
scenario="${FINAL_REVIEW_SCENARIO:-no-amendments}"
backend_id="${FINAL_REVIEW_BACKEND:-unknown}"

inc_counter() {
  local name="$1"
  local file="${COUNTER_DIR}/${name}"
  local value=0
  if [ -f "$file" ]; then
    value=$(cat "$file")
  fi
  value=$((value + 1))
  echo "$value" > "$file"
  echo "$value"
}

if [[ "$prompt" == *"You are a software architect planning features for a project."* ]]; then
  cat <<'EOF'
# Project Completion Request

## Rationale
This test project is ready for completion validation.

## Summary of Work
- Existing test implementation satisfies requirements.

## Remaining Items
- None
EOF
elif [[ "$prompt" == *"You are a project completion validator."* ]]; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Requirement alpha: complete
EOF
elif [[ "$prompt" == *"You are a QA engineer validating overall project acceptance."* ]]; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- acceptance check passed

## Automated Tests
- acceptance check passed

## Acceptance Criteria Verification
Project-level acceptance requirements are satisfied.
EOF
elif [[ "$prompt" == *"You are a final reviewer auditing a completed project for correctness, safety, and robustness."* ]]; then
  total=$(inc_counter "final_reviewer_total")
  inc_counter "final_reviewer_${backend_id}" >/dev/null
  round=$(( (total - 1) / 2 + 1 ))
  upper_backend=$(printf "%s" "$backend_id" | tr '[:lower:]' '[:upper:]')

  if [[ "$scenario" == "no-amendments" ]]; then
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No amendments are required.
EOF
  elif [[ "$scenario" == "accepted-restart" ]]; then
    if [ "$round" -eq 1 ]; then
      cat <<EOF
# Final Review: AMENDMENTS

## Amendment: ${upper_backend}-R1

### Problem
Gap identified by ${backend_id} reviewer.

### Proposed Change
Apply ${backend_id} round-1 amendment.

### Affected Files
- \`README.md\` - update details
EOF
    else
      cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
Round-two verification found no additional amendments.
EOF
    fi
  elif [[ "$scenario" == "fail-after-proposals-once" || "$scenario" == "config-mismatch" ]]; then
    cat <<EOF
# Final Review: AMENDMENTS

## Amendment: ${upper_backend}-R1

### Problem
Gap identified by ${backend_id} reviewer.

### Proposed Change
Apply ${backend_id} round-1 amendment.

### Affected Files
- \`README.md\` - update details
EOF
  elif [[ "$scenario" == "disputed-restart" ]]; then
    if [ "$round" -eq 1 ]; then
      cat <<EOF
# Final Review: AMENDMENTS

## Amendment: ${upper_backend}-R1

### Problem
Disputed round-one amendment from ${backend_id}.

### Proposed Change
Apply ${backend_id} disputed amendment.

### Affected Files
- \`README.md\` - update details
EOF
    else
      cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No further amendments are required in round two.
EOF
    fi
  else
    cat <<EOF
# Final Review: AMENDMENTS

## Amendment: ${upper_backend}-R${round}

### Problem
Recurring amendment in round ${round}.

### Proposed Change
Apply recurring ${backend_id} amendment for round ${round}.

### Affected Files
- \`README.md\` - update details
EOF
  fi
elif [[ "$prompt" == *"You are a technical evaluator assessing proposed amendments from final reviewers."* ]]; then
  call=$(inc_counter "planner_position_calls")
  if [[ "$scenario" == "fail-after-proposals-once" || "$scenario" == "config-mismatch" ]]; then
    marker="${COUNTER_DIR}/planner_position_failed_once"
    if [ ! -f "$marker" ]; then
      echo "$call" > "$marker"
      cat <<'EOF'
# Invalid Planner Output
EOF
      exit 0
    fi
  fi

  cat <<'EOF'
# Planner Positions
EOF
  round="$call"
  if [[ "$scenario" == "accepted-restart" || "$scenario" == "disputed-restart" ]]; then
    if [ "$round" -eq 1 ]; then
      ids=("CLAUDE-R1" "CODEX-R1")
    else
      ids=()
    fi
  elif [[ "$scenario" == "fail-after-proposals-once" || "$scenario" == "config-mismatch" ]]; then
    ids=("CLAUDE-R1" "CODEX-R1")
  elif [[ "$scenario" == "always-amend" ]]; then
    ids=("CLAUDE-R${round}" "CODEX-R${round}")
  else
    ids=()
  fi
  for id in "${ids[@]}"; do
    cat <<EOF

## Amendment: $id

### Position
ACCEPT

### Rationale
Accepted by planner.
EOF
  done
elif [[ "$prompt" == *"You are a reviewer voting on proposed amendments after considering the planner's positions."* ]]; then
  total=$(inc_counter "vote_total")
  inc_counter "vote_${backend_id}" >/dev/null
  round=$(( (total - 1) / 2 + 1 ))
  cat <<'EOF'
# Vote Results
EOF
  if [[ "$scenario" == "accepted-restart" || "$scenario" == "disputed-restart" ]]; then
    ids=("CLAUDE-R1" "CODEX-R1")
  elif [[ "$scenario" == "fail-after-proposals-once" || "$scenario" == "config-mismatch" ]]; then
    ids=("CLAUDE-R1" "CODEX-R1")
  elif [[ "$scenario" == "always-amend" ]]; then
    ids=("CLAUDE-R${round}" "CODEX-R${round}")
  else
    ids=()
  fi
  for id in "${ids[@]}"; do
    vote="ACCEPT"
    if [[ "$scenario" == "disputed-restart" && "$round" -eq 1 && "$id" == "CODEX-R1" && "$backend_id" == "codex" ]]; then
      vote="REJECT"
    fi
    cat <<EOF

## Amendment: $id

### Vote
$vote

### Rationale
${backend_id} vote is $vote.
EOF
  done
elif [[ "$prompt" == *"You are the arbiter resolving disputed amendments where reviewers and planner disagree."* ]]; then
  inc_counter "arbiter_calls" >/dev/null
  printf "%s" "$prompt" > "${COUNTER_DIR}/arbiter_prompt.md"
  cat <<'EOF'
# Arbiter Ruling
EOF
  if [[ "$scenario" == "disputed-restart" ]]; then
    ids=("CODEX-R1")
  else
    ids=()
  fi
  for id in "${ids[@]}"; do
    ruling="ACCEPT"
    if [[ "$scenario" == "disputed-restart" ]]; then
      ruling="REJECT"
    fi
    cat <<EOF

## Amendment: $id

### Ruling
$ruling

### Rationale
Arbiter ruling is $ruling.
EOF
  done
elif [[ "$prompt" == *"You are a prompt reviewer"* ]]; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- None

## Refined Prompt
No refinement needed.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###;

    fs::write(path, script).expect("write final-review backend script");
    let status = Command::new("chmod")
        .args(["+x", path.to_str().expect("script utf8 path")])
        .status()
        .expect("chmod should execute");
    assert!(status.success(), "chmod +x failed");
}

fn setup_workspace_for_final_review(
    scenario: &str,
    max_restarts: u32,
) -> (TempDir, PathBuf, String, PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);
    fs::write(repo_root.join("README.md"), "# demo\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    let script_path = repo_root.join("mock_final_review.sh");
    write_final_review_backend_script(&script_path);
    git_ok(repo_root, &["add", "mock_final_review.sh"]);
    git_ok(repo_root, &["commit", "-m", "test: add final review mock"]);

    add_local_bare_remote(repo_root);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");

    let counter_dir = workspace_root.join("counters");
    fs::create_dir_all(&counter_dir).expect("create counter dir");
    let counter_dir_str = counter_dir.to_string_lossy().to_string();

    workspace.config.backends.claude.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace
        .config
        .backends
        .claude
        .env
        .insert("COUNTER_DIR".to_owned(), counter_dir_str.clone());
    workspace
        .config
        .backends
        .claude
        .env
        .insert("FINAL_REVIEW_SCENARIO".to_owned(), scenario.to_owned());
    workspace
        .config
        .backends
        .claude
        .env
        .insert("FINAL_REVIEW_BACKEND".to_owned(), "claude".to_owned());

    workspace.config.backends.codex.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace
        .config
        .backends
        .codex
        .env
        .insert("COUNTER_DIR".to_owned(), counter_dir_str);
    workspace
        .config
        .backends
        .codex
        .env
        .insert("FINAL_REVIEW_SCENARIO".to_owned(), scenario.to_owned());
    workspace
        .config
        .backends
        .codex
        .env
        .insert("FINAL_REVIEW_BACKEND".to_owned(), "codex".to_owned());

    workspace.config.workflow.prompt_review_enabled = false;
    workspace.config.workflow.final_review_enabled = true;
    workspace.config.workflow.max_final_review_restarts = max_restarts;
    workspace.config.workflow.final_review_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.config.workflow.final_review_arbiter_backend = "claude".to_owned();
    workspace.config.workflow.final_review_consensus_threshold = 1.0;
    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.config.workflow.completion_backends = vec!["claude".to_owned(), "codex".to_owned()];
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Final review test prompt\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt source"]);

    let project_id = "01-poc".to_owned();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Proof of Concept".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id, counter_dir)
}

fn run_until_complete_options(project_id: &str) -> RunOptions {
    let mut options = run_options(project_id);
    options.loops = None;
    options.until_complete = true;
    options
}

fn read_counter(counter_dir: &Path, name: &str) -> u32 {
    fs::read_to_string(counter_dir.join(name))
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

fn loop_has_artifact_suffix(project_dir: &Path, loop_number: u32, suffix: &str) -> bool {
    let Some(repo_rel_loop_dir) =
        fs::read_dir(project_dir.join("loops"))
            .ok()
            .and_then(|entries| {
                entries
                    .flatten()
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(&format!("{loop_number:03}-"))
                    })
                    .map(|entry| entry.path())
            })
    else {
        return false;
    };
    let Ok(entries) = fs::read_dir(repo_rel_loop_dir) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().ends_with(suffix))
}

#[tokio::test]
async fn final_review_no_amendments_completes_project() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_final_review("no-amendments", 3);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_until_complete_options(&project_id))
        .await
        .expect("run should succeed");

    let project_dir = workspace_root.join("projects").join(&project_id);
    let state = reconstruct_project_state_from_project_dir(&project_dir).expect("load state");
    assert_eq!(state.status, ProjectStatus::Completed);
    assert_eq!(state.current_phase, Phase::Completing);
    assert_eq!(state.completion_attempts.len(), 1);
    assert!(
        loop_has_artifact_suffix(
            &project_dir,
            state.completion_attempts[0].loop_number,
            "-final-review-exit-approved.md"
        ),
        "expected final-review approved exit artifact"
    );
    assert_eq!(read_counter(&counter_dir, "final_reviewer_claude"), 1);
    assert_eq!(read_counter(&counter_dir, "final_reviewer_codex"), 1);
}

#[tokio::test]
async fn final_review_accepted_amendments_restart_to_planning_then_complete() {
    let (_temp, workspace_root, project_id, _counter_dir) =
        setup_workspace_for_final_review("accepted-restart", 3);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_until_complete_options(&project_id))
        .await
        .expect("run should succeed");

    let project_dir = workspace_root.join("projects").join(&project_id);
    let state = reconstruct_project_state_from_project_dir(&project_dir).expect("load state");
    assert_eq!(state.status, ProjectStatus::Completed);
    assert_eq!(
        state.completion_attempts.len(),
        2,
        "accepted amendments should trigger a planning restart and second completion attempt"
    );
    let amendments_file =
        fs::read_to_string(project_dir.join("final-review-amendments-applied.md"))
            .expect("amendments file should exist");
    assert!(amendments_file.contains("## Round 1"));
    assert!(amendments_file.contains("CLAUDE-R1") || amendments_file.contains("CODEX-R1"));

    let repo_root = workspace_root.parent().expect("repo root");
    let log_output = git_output(
        repo_root,
        &[
            "log",
            "--format=%s",
            "--fixed-strings",
            "--grep",
            "chore(01-poc): checkpoint final_review -> planning",
        ],
    );
    assert!(
        !log_output.trim().is_empty(),
        "expected at least one final_review -> planning checkpoint commit"
    );
}

#[tokio::test]
async fn final_review_disputed_amendments_invokes_arbiter_only_for_disputed_ids() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_final_review("disputed-restart", 3);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_until_complete_options(&project_id))
        .await
        .expect("run should succeed");

    assert_eq!(read_counter(&counter_dir, "arbiter_calls"), 1);
    let arbiter_prompt =
        fs::read_to_string(counter_dir.join("arbiter_prompt.md")).expect("arbiter prompt");
    assert!(
        arbiter_prompt.contains("## Amendment: CODEX-R1"),
        "arbiter should receive disputed amendment ID"
    );
    let project_dir = workspace_root.join("projects").join(&project_id);
    let first_completion_loop = 1_u32;
    let Some(loop_dir) = fs::read_dir(project_dir.join("loops"))
        .ok()
        .and_then(|entries| {
            entries.flatten().find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{first_completion_loop:03}-completion"))
            })
        })
        .map(|entry| entry.path())
    else {
        panic!("completion loop directory should exist");
    };
    let arbiter_artifact = fs::read_dir(loop_dir)
        .expect("read loop dir")
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with("-final-review-arbiter-ruling.md")
        })
        .map(|entry| entry.path())
        .expect("arbiter ruling artifact should exist");
    let arbiter_ruling = fs::read_to_string(arbiter_artifact).expect("read arbiter artifact");
    assert!(
        !arbiter_ruling.contains("## Amendment: CLAUDE-R1"),
        "arbiter ruling should include only disputed amendment IDs"
    );
}

#[tokio::test]
async fn final_review_resume_skips_completed_proposal_step() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_final_review("fail-after-proposals-once", 1);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let first = orchestrator
        .run(run_until_complete_options(&project_id))
        .await;
    assert!(first.is_err(), "first run should fail after proposals");
    assert_eq!(read_counter(&counter_dir, "final_reviewer_claude"), 1);
    assert_eq!(read_counter(&counter_dir, "final_reviewer_codex"), 1);

    let workspace = Workspace::load(workspace_root.clone()).expect("reload workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_until_complete_options(&project_id))
        .await
        .expect("resume should succeed");
    assert_eq!(
        read_counter(&counter_dir, "final_reviewer_claude"),
        1,
        "resume should reuse round artifacts instead of re-invoking proposal step"
    );
    assert_eq!(
        read_counter(&counter_dir, "final_reviewer_codex"),
        1,
        "resume should reuse round artifacts instead of re-invoking proposal step"
    );
}

#[tokio::test]
async fn final_review_config_mismatch_invalidates_and_restarts_round() {
    let (_temp, workspace_root, project_id, counter_dir) =
        setup_workspace_for_final_review("config-mismatch", 1);
    let project_dir = workspace_root.join("projects").join(&project_id);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    let first = orchestrator
        .run(run_until_complete_options(&project_id))
        .await;
    assert!(first.is_err(), "first run should fail after proposals");
    assert_eq!(read_counter(&counter_dir, "final_reviewer_claude"), 1);
    assert_eq!(read_counter(&counter_dir, "final_reviewer_codex"), 1);

    let mut workspace = Workspace::load(workspace_root.clone()).expect("reload workspace");
    workspace.config.workflow.final_review_consensus_threshold = 0.6;
    workspace.save_config().expect("save updated config");

    let workspace = Workspace::load(workspace_root.clone()).expect("reload workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_until_complete_options(&project_id))
        .await
        .expect("second run should succeed");

    assert_eq!(
        read_counter(&counter_dir, "final_reviewer_claude"),
        2,
        "config mismatch should invalidate old proposal artifacts and re-run reviewers"
    );
    assert_eq!(
        read_counter(&counter_dir, "final_reviewer_codex"),
        2,
        "config mismatch should invalidate old proposal artifacts and re-run reviewers"
    );
    assert!(
        project_dir.join("final-review-force-complete.md").exists(),
        "force-complete artifact should exist after reaching max restart cap"
    );
}

#[tokio::test]
async fn final_review_restart_cap_triggers_force_complete() {
    let (_temp, workspace_root, project_id, _counter_dir) =
        setup_workspace_for_final_review("always-amend", 1);

    let workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    let mut orchestrator = Orchestrator::new(workspace);
    orchestrator
        .run(run_until_complete_options(&project_id))
        .await
        .expect("run should succeed");

    let project_dir = workspace_root.join("projects").join(&project_id);
    let state = reconstruct_project_state_from_project_dir(&project_dir).expect("load state");
    assert_eq!(state.status, ProjectStatus::Completed);
    assert_eq!(state.completion_attempts.len(), 2);
    assert!(
        project_dir.join("final-review-force-complete.md").exists(),
        "expected force-complete artifact at restart cap"
    );
}
