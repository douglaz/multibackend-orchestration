//! Integration tests for orchestration flows.

use ralph::error::RalphError;
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
        planner_backend: None,
        implementer_backend: None,
        reviewer_backend: None,
        completer_backend: None,
        tmux: None,
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

    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
        .expect("load project state");
    assert_eq!(state.current_loop, 0);
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

    // Initialise workspace
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
    workspace.save_config().expect("save config");

    // Create project
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

    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
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
    assert_eq!(
        state.loops[0].backends.planner,
        "claude(opus)"
    );
    assert_eq!(state.loops[0].backends.implementer, "codex(gpt-5.3-codex-high)");
    assert_eq!(
        state.loops[0].backends.reviewer,
        "claude(opus)"
    );
    // Loop 2 (even): planner=codex, implementer=claude, reviewer=codex
    assert_eq!(state.loops[1].backends.planner, "codex(gpt-5.3-codex-xhigh)");
    assert_eq!(
        state.loops[1].backends.implementer,
        "claude(sonnet)"
    );
    assert_eq!(state.loops[1].backends.reviewer, "codex(gpt-5.3-codex-xhigh)");

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

    // --- 5. Git tags ---
    let repo_root = workspace_root.parent().expect("repo root");
    let tag1 = git_output(repo_root, &["tag", "--list", "ralph/01-poc/loop-1"]);
    assert_eq!(tag1.trim(), "ralph/01-poc/loop-1");
    let tag2 = git_output(repo_root, &["tag", "--list", "ralph/01-poc/loop-2"]);
    assert_eq!(tag2.trim(), "ralph/01-poc/loop-2");

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
    assert_timestamped_artifact(
        state.completion_attempts[0]
            .artifacts
            .verdict
            .as_deref()
            .expect("completion verdict artifact should exist"),
        "completer-verdict.md",
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
    env.insert("PLANNER_MODE".to_owned(), "feature".to_owned());
    env.insert("REVIEW_MODE".to_owned(), "suggestions".to_owned());
    env.insert("COMPLETER_MODE".to_owned(), "complete".to_owned());

    workspace.config.backends.claude.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = env.clone();

    workspace.config.backends.codex.command = script_path.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = env;

    workspace.config.workflow.max_review_iterations = 1;

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
    let state = load_project_state(&workspace_root.join("projects").join(&project_id))
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
        state.current_loop, 0,
        "current_loop should be 0 after rollback"
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
  if [[ " $* " == *" --model gpt-5.3-codex-medium "* ]]; then
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
