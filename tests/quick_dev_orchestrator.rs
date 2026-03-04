//! Integration tests for quick-dev orchestrator.
//! These tests validate the phase machine, backend resolution, guard behavior,
//! crash-safe resume, and commit-guard logic.

use ralph::project::lifecycle::{create_project, CreateProjectOptions, PromptSource};
use ralph::project::state::{Phase, ProjectStatus, QuickDevPhase};
use ralph::workflow::quick_dev_orchestrator::{QuickDevOrchestrator, QuickDevRunOptions};
use ralph::workspace::Workspace;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

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

fn add_local_bare_remote(repo_root: &Path) {
    let bare_dir = repo_root.join(".test-remote.git");
    let bare_str = bare_dir.to_string_lossy().to_string();

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

// ---------------------------------------------------------------------------
// Quick-dev backend script
// ---------------------------------------------------------------------------

/// Write a mock backend script that responds differently for quick-dev phases.
/// Env vars control the behavior:
///   QUICK_DEV_REVIEW_MODE: "satisfied" (default) | "changes_requested" | "loop_then_satisfied"
///   QUICK_DEV_FINAL_REVIEW_MODE: "complete" (default) | "issues_found" | "loop_then_complete"
///   QUICK_DEV_COUNTER_DIR: directory for stable cross-invocation counter files
fn write_quick_dev_backend_script(path: &Path, role: &str) {
    let script = if role == "implementer" {
        r#"#!/bin/bash
set -euo pipefail
prompt="$(cat)"

# Check if this is a final review prompt
if [[ "$prompt" == *"final review"* ]] || [[ "$prompt" == *"Final Review"* ]] || [[ "$prompt" == *"final-review"* ]]; then
    mode="${QUICK_DEV_FINAL_REVIEW_MODE:-complete}"
    counter_dir="${QUICK_DEV_COUNTER_DIR:-/tmp}"
    counter_file="${counter_dir}/impl_final_review_counter"

    if [[ "$mode" == "loop_then_complete" ]]; then
        count=0
        if [[ -f "$counter_file" ]]; then
            count=$(cat "$counter_file")
        fi
        count=$((count + 1))
        echo "$count" > "$counter_file"
        if [[ $count -ge 2 ]]; then
            cat <<'EOF'
# Final Review: COMPLETE

All requirements are met from implementer perspective.
EOF
        else
            cat <<'EOF'
# Final Review: ISSUES FOUND

Implementation has issues that need addressing.
EOF
        fi
    elif [[ "$mode" == "issues_found" ]]; then
        cat <<'EOF'
# Final Review: ISSUES FOUND

Implementation has issues that need addressing.
EOF
    else
        cat <<'EOF'
# Final Review: COMPLETE

All requirements are met from implementer perspective.
EOF
    fi
else
    # Plan and implement / apply fixes response
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Implemented the feature as specified

## Spec Deviations
- None

## Testing
- All tests pass
EOF
fi
"#
    } else {
        // reviewer
        r#"#!/bin/bash
set -euo pipefail
prompt="$(cat)"

# Check if this is a final review prompt
if [[ "$prompt" == *"final review"* ]] || [[ "$prompt" == *"Final Review"* ]] || [[ "$prompt" == *"final-review"* ]]; then
    mode="${QUICK_DEV_FINAL_REVIEW_MODE:-complete}"
    counter_dir="${QUICK_DEV_COUNTER_DIR:-/tmp}"
    counter_file="${counter_dir}/rev_final_review_counter"

    if [[ "$mode" == "loop_then_complete" ]]; then
        count=0
        if [[ -f "$counter_file" ]]; then
            count=$(cat "$counter_file")
        fi
        count=$((count + 1))
        echo "$count" > "$counter_file"
        if [[ $count -ge 2 ]]; then
            cat <<'EOF'
# Final Review: COMPLETE

All requirements are met.
EOF
        else
            cat <<'EOF'
# Final Review: ISSUES FOUND

Missing test coverage for edge cases.
EOF
        fi
    elif [[ "$mode" == "issues_found" ]]; then
        cat <<'EOF'
# Final Review: ISSUES FOUND

Missing test coverage for edge cases.
EOF
    else
        cat <<'EOF'
# Final Review: COMPLETE

All requirements are met.
EOF
    fi
else
    # Codex review
    mode="${QUICK_DEV_REVIEW_MODE:-satisfied}"
    counter_dir="${QUICK_DEV_COUNTER_DIR:-/tmp}"
    counter_file="${counter_dir}/rev_review_counter"

    if [[ "$mode" == "loop_then_satisfied" ]]; then
        count=0
        if [[ -f "$counter_file" ]]; then
            count=$(cat "$counter_file")
        fi
        count=$((count + 1))
        echo "$count" > "$counter_file"
        if [[ $count -ge 2 ]]; then
            cat <<'EOF'
# Review: SATISFIED

Implementation looks good.
EOF
        else
            cat <<'EOF'
# Review: CHANGES REQUESTED

Please add error handling.
EOF
        fi
    elif [[ "$mode" == "changes_requested" ]]; then
        cat <<'EOF'
# Review: CHANGES REQUESTED

Please add error handling.
EOF
    else
        cat <<'EOF'
# Review: SATISFIED

Implementation looks good.
EOF
    fi
fi
"#
    };
    fs::write(path, script).expect("write backend script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }
}

fn write_bash_wrapper(path: &Path, target_script: &Path) {
    let target = target_script.to_string_lossy();
    let wrapper = format!("#!/bin/sh\nexec bash \"{target}\" \"$@\"\n");
    fs::write(path, wrapper).expect("write bash wrapper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }
}

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

fn setup_quick_dev_workspace(
    review_mode: &str,
    final_review_mode: &str,
) -> (TempDir, PathBuf, String) {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path();

    git_ok(repo_root, &["init"]);
    git_ok(repo_root, &["config", "user.email", "test@example.com"]);
    git_ok(repo_root, &["config", "user.name", "Test User"]);

    fs::write(repo_root.join("README.md"), "# demo\n").expect("write README");
    git_ok(repo_root, &["add", "-A"]);
    git_ok(repo_root, &["commit", "-m", "initial"]);

    let impl_script = repo_root.join("mock_impl.sh");
    let rev_script = repo_root.join("mock_rev.sh");
    let impl_wrapper = repo_root.join("mock_impl_wrapper.sh");
    let rev_wrapper = repo_root.join("mock_rev_wrapper.sh");
    write_quick_dev_backend_script(&impl_script, "implementer");
    write_quick_dev_backend_script(&rev_script, "reviewer");
    write_bash_wrapper(&impl_wrapper, &impl_script);
    write_bash_wrapper(&rev_wrapper, &rev_script);
    git_ok(
        repo_root,
        &[
            "add",
            "mock_impl.sh",
            "mock_rev.sh",
            "mock_impl_wrapper.sh",
            "mock_rev_wrapper.sh",
        ],
    );
    git_ok(repo_root, &["commit", "-m", "test: add backend mocks"]);

    add_local_bare_remote(repo_root);

    let workspace_root = repo_root.join(".ralph");
    let mut workspace = Workspace::init(&workspace_root).expect("workspace init");

    // Use a stable counter directory inside the temp dir
    let counter_dir = repo_root.join(".test-counters");
    fs::create_dir_all(&counter_dir).expect("create counter dir");
    let counter_dir_str = counter_dir.to_string_lossy().to_string();

    let mut impl_env = BTreeMap::new();
    impl_env.insert("QUICK_DEV_REVIEW_MODE".to_owned(), review_mode.to_owned());
    impl_env.insert(
        "QUICK_DEV_FINAL_REVIEW_MODE".to_owned(),
        final_review_mode.to_owned(),
    );
    impl_env.insert("QUICK_DEV_COUNTER_DIR".to_owned(), counter_dir_str.clone());

    let mut rev_env = BTreeMap::new();
    rev_env.insert("QUICK_DEV_REVIEW_MODE".to_owned(), review_mode.to_owned());
    rev_env.insert(
        "QUICK_DEV_FINAL_REVIEW_MODE".to_owned(),
        final_review_mode.to_owned(),
    );
    rev_env.insert("QUICK_DEV_COUNTER_DIR".to_owned(), counter_dir_str);

    workspace.config.backends.claude.command = impl_wrapper.to_string_lossy().to_string();
    workspace.config.backends.claude.args = Vec::new();
    workspace.config.backends.claude.timeout_seconds = 30;
    workspace.config.backends.claude.env = impl_env;

    workspace.config.backends.codex.command = rev_wrapper.to_string_lossy().to_string();
    workspace.config.backends.codex.args = Vec::new();
    workspace.config.backends.codex.timeout_seconds = 30;
    workspace.config.backends.codex.env = rev_env;

    workspace.config.git.base_branch =
        git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    workspace.config.workflow.implementer_backend = Some("claude".to_owned());
    workspace.config.workflow.reviewer_backend = Some("codex".to_owned());
    workspace.save_config().expect("save config");

    let prompt_path = repo_root.join("PROMPT.md");
    fs::write(&prompt_path, "# Build a quick-dev demo\n").expect("write prompt");
    git_ok(repo_root, &["add", "PROMPT.md"]);
    git_ok(repo_root, &["commit", "-m", "test: add prompt"]);

    let project_id = "issue-99".to_owned();
    create_project(
        &workspace,
        CreateProjectOptions {
            id: project_id.clone(),
            name: "Quick Dev Test".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: Some("claude".to_owned()),
        },
    )
    .expect("create project");

    (temp, workspace_root, project_id)
}

// ---------------------------------------------------------------------------
// Unit tests (no workspace needed)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_reviewer_backend_fails_fast() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Remove the reviewer backend from config so resolution fails
    let workspace_root_clone = workspace_root.clone();
    let mut workspace = Workspace::load(workspace_root_clone).expect("load workspace");
    workspace.config.workflow.reviewer_backend = None;
    workspace.save_config().expect("save config");

    let workspace = Workspace::load(workspace_root).expect("reload workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let err = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: None,
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect_err("should fail when reviewer backend is missing");

    let msg = err.to_string();
    assert!(
        msg.contains("quick-dev requires a second backend for review"),
        "expected exact missing-reviewer message, got: {msg}"
    );
}

#[tokio::test]
async fn equal_backend_rejection() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let err = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("claude".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect_err("should fail when both backends are equal");

    let msg = err.to_string();
    assert!(
        msg.contains("distinct"),
        "expected distinct-backend error, got: {msg}"
    );
    assert!(
        msg.contains("claude"),
        "error should mention the backend spec, got: {msg}"
    );
}

#[test]
fn quick_dev_phase_serde_roundtrip() {
    use ralph::project::state::ProjectState;

    let mut state = ProjectState::new("test", "Test", "hash", None);
    state.quick_dev_phase = Some(QuickDevPhase::CodexReview);

    let json = serde_json::to_string(&state).unwrap();
    let loaded: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.quick_dev_phase, Some(QuickDevPhase::CodexReview));
}

#[test]
fn quick_dev_phase_to_current_phase() {
    assert_eq!(QuickDevPhase::PlanAndImplement.to_current_phase(), Phase::Implementing);
    assert_eq!(QuickDevPhase::CodexReview.to_current_phase(), Phase::Reviewing);
    assert_eq!(QuickDevPhase::ApplyFixes.to_current_phase(), Phase::Implementing);
    assert_eq!(QuickDevPhase::FinalReview.to_current_phase(), Phase::FinalReview);
}

#[test]
fn default_option_values() {
    let options = QuickDevRunOptions {
        project: None,
        implementer_backend: None,
        reviewer_backend: None,
        pr_url: None,
        skip_commit: false,
        max_review_iterations: None,
        max_final_review_retries: None,
    };

    // When max values are None, defaults should be applied internally
    assert!(options.max_review_iterations.is_none());
    assert!(options.max_final_review_retries.is_none());
}

#[test]
fn quick_dev_counter_fields_serde_roundtrip() {
    use ralph::project::state::ProjectState;

    let mut state = ProjectState::new("test", "Test", "hash", None);
    state.quick_dev_review_iteration = 3;
    state.quick_dev_final_review_attempts = 1;

    let json = serde_json::to_string(&state).unwrap();
    let loaded: ProjectState = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded.quick_dev_review_iteration, 3);
    assert_eq!(loaded.quick_dev_final_review_attempts, 1);
}

#[test]
fn legacy_state_without_counter_fields_defaults_to_zero() {
    use ralph::project::state::ProjectState;

    let state = ProjectState::new("test", "Test", "hash", None);
    let mut value = serde_json::to_value(&state).expect("serialize");
    let obj = value.as_object_mut().unwrap();
    obj.remove("quick_dev_review_iteration");
    obj.remove("quick_dev_final_review_attempts");

    let loaded: ProjectState = serde_json::from_value(value).unwrap();
    assert_eq!(loaded.quick_dev_review_iteration, 0);
    assert_eq!(loaded.quick_dev_final_review_attempts, 0);
}

// ---------------------------------------------------------------------------
// Integration tests - happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn happy_path_review_satisfied_then_complete() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should succeed");

    assert!(
        result.summary.contains("completed"),
        "expected 'completed' in summary, got: {}",
        result.summary
    );
    assert_eq!(result.loop_number, Some(1));

    // Verify state on disk
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let state_json = fs::read_to_string(project_dir.join("state.json")).expect("read state.json");
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();
    assert_eq!(state["status"], "completed");
}

// ---------------------------------------------------------------------------
// Integration tests - review loop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn review_loop_with_changes_requested_then_satisfied() {
    let (_temp, workspace_root, project_id) =
        setup_quick_dev_workspace("loop_then_satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: Some(5),
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should succeed after review loop");

    assert!(
        result.summary.contains("completed"),
        "expected 'completed' in summary, got: {}",
        result.summary
    );

    // Verify artifacts were written
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let loops_dir = project_dir.join("loops");
    assert!(loops_dir.exists(), "loops directory should exist");
}

// ---------------------------------------------------------------------------
// Integration tests - max review iterations guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_review_iterations_guard_skips_to_final_review() {
    let (_temp, workspace_root, project_id) =
        setup_quick_dev_workspace("changes_requested", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: Some(2),
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should complete via guard");

    assert!(
        result.summary.contains("completed"),
        "expected completion, got: {}",
        result.summary
    );

    // The review limit warning artifact should exist
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    assert!(
        project_dir
            .join("quick-dev-review-limit-warning.md")
            .exists(),
        "review limit warning artifact should exist"
    );
}

// ---------------------------------------------------------------------------
// Integration tests - max final review retries guard (force-complete)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_final_review_retries_guard_force_completes() {
    let (_temp, workspace_root, project_id) =
        setup_quick_dev_workspace("satisfied", "issues_found");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: Some(2),
        })
        .await
        .expect("orchestrator should force-complete");

    assert!(
        result.summary.contains("force-completed"),
        "expected 'force-completed' in summary, got: {}",
        result.summary
    );

    // The force-complete artifact should exist
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    assert!(
        project_dir.join("quick-dev-force-complete.md").exists(),
        "force-complete artifact should exist"
    );

    // Verify final status is completed
    let state_json = fs::read_to_string(project_dir.join("state.json")).expect("read state.json");
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();
    assert_eq!(state["status"], "completed");
}

// ---------------------------------------------------------------------------
// Integration tests - final review reloop (FinalReview -> PlanAndImplement -> ... -> Complete)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn final_review_reloop_then_complete() {
    // First final review: both say ISSUES FOUND (implementer: issues, reviewer: issues)
    // After reloop through PlanAndImplement -> CodexReview(satisfied) -> FinalReview:
    // Second final review: both say COMPLETE
    let (_temp, workspace_root, project_id) =
        setup_quick_dev_workspace("satisfied", "loop_then_complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: Some(5), // allow enough retries
        })
        .await
        .expect("orchestrator should complete after final-review reloop");

    assert!(
        result.summary.contains("completed"),
        "expected 'completed' in summary, got: {}",
        result.summary
    );

    // Verify state on disk
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let state_json = fs::read_to_string(project_dir.join("state.json")).expect("read state.json");
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();
    assert_eq!(state["status"], "completed");

    // Force-complete artifact should NOT exist (completed normally)
    assert!(
        !project_dir.join("quick-dev-force-complete.md").exists(),
        "force-complete artifact should not exist for normal completion after reloop"
    );
}

// ---------------------------------------------------------------------------
// Integration tests - resume from persisted state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resume_from_codex_review_phase() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Write persisted state with quick_dev_phase = CodexReview
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let mut state = ralph::project::state::ProjectState::new(
        &project_id,
        "Quick Dev Test",
        "hash123",
        None,
    );
    state.quick_dev_phase = Some(QuickDevPhase::CodexReview);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::Reviewing;
    state.phase_iteration = 1;
    // Register a loop so we have a loop_number
    state.register_feature_loop(
        1,
        "quick-dev".to_owned(),
        "Quick Dev".to_owned(),
        ralph::project::state::FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "claude".to_owned(),
            reviewer: "codex".to_owned(),
            qa: String::new(),
        },
        String::new(),
        chrono::Utc::now(),
    );
    let state_json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(project_dir.join("state.json"), &state_json).expect("write state.json");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should resume from CodexReview");

    assert!(
        result.summary.contains("completed"),
        "expected completion, got: {}",
        result.summary
    );
}

#[tokio::test]
async fn resume_from_final_review_phase() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Write persisted state with quick_dev_phase = FinalReview
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let mut state = ralph::project::state::ProjectState::new(
        &project_id,
        "Quick Dev Test",
        "hash123",
        None,
    );
    state.quick_dev_phase = Some(QuickDevPhase::FinalReview);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::FinalReview;
    state.phase_iteration = 1;
    state.register_feature_loop(
        1,
        "quick-dev".to_owned(),
        "Quick Dev".to_owned(),
        ralph::project::state::FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "claude".to_owned(),
            reviewer: "codex".to_owned(),
            qa: String::new(),
        },
        String::new(),
        chrono::Utc::now(),
    );
    let state_json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(project_dir.join("state.json"), &state_json).expect("write state.json");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should resume from FinalReview");

    assert!(
        result.summary.contains("completed"),
        "expected completion, got: {}",
        result.summary
    );
}

#[tokio::test]
async fn resume_from_none_starts_at_plan_and_implement() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // No state.json written -> starts from PlanAndImplement
    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should start from PlanAndImplement");

    assert!(
        result.summary.contains("completed"),
        "expected completion, got: {}",
        result.summary
    );
}

#[tokio::test]
async fn resume_after_completion_does_not_restart() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Write persisted state marking project as completed with quick_dev_phase = None
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let mut state = ralph::project::state::ProjectState::new(
        &project_id,
        "Quick Dev Test",
        "hash123",
        None,
    );
    state.register_feature_loop(
        1,
        "quick-dev".to_owned(),
        "Quick Dev".to_owned(),
        ralph::project::state::FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "claude".to_owned(),
            reviewer: "codex".to_owned(),
            qa: String::new(),
        },
        String::new(),
        chrono::Utc::now(),
    );
    // Set completion status AFTER register_feature_loop (which sets InProgress)
    state.quick_dev_phase = None;
    state.status = ProjectStatus::Completed;
    state.current_phase = Phase::Completing;
    state.phase_iteration = 1;
    let state_json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(project_dir.join("state.json"), &state_json).expect("write state.json");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should not restart a completed project");

    assert!(
        result.summary.contains("already completed"),
        "expected 'already completed' in summary, got: {}",
        result.summary
    );
}

// ---------------------------------------------------------------------------
// Integration tests - ApplyFixes resume with review feedback
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resume_from_apply_fixes_includes_review_feedback() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Write persisted state with quick_dev_phase = ApplyFixes and review_iteration = 1
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let mut state = ralph::project::state::ProjectState::new(
        &project_id,
        "Quick Dev Test",
        "hash123",
        None,
    );
    state.quick_dev_phase = Some(QuickDevPhase::ApplyFixes);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::Implementing;
    state.phase_iteration = 1;
    state.quick_dev_review_iteration = 1;
    state.register_feature_loop(
        1,
        "quick-dev".to_owned(),
        "Quick Dev".to_owned(),
        ralph::project::state::FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "claude".to_owned(),
            reviewer: "codex".to_owned(),
            qa: String::new(),
        },
        String::new(),
        chrono::Utc::now(),
    );
    // Set ApplyFixes state AFTER register_feature_loop (which overrides phase)
    state.quick_dev_phase = Some(QuickDevPhase::ApplyFixes);
    state.current_phase = Phase::Implementing;
    let state_json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(project_dir.join("state.json"), &state_json).expect("write state.json");

    // Write a changes-requested artifact so the orchestrator can reload
    // reviewer feedback on resume.
    let loop_dir = project_dir.join("loops/001-quick-dev");
    fs::create_dir_all(&loop_dir).expect("create loop dir");
    let review_feedback_body = "Please add proper error handling for edge cases.";
    let artifact_content = format!(
        "---\nartifact: quick-dev-codex-review\nloop: 1\nproject: {project_id}\nbackend: codex\nrole: reviewer\ncreated_at: 2026-03-04T02:00:00Z\n---\n\n{review_feedback_body}"
    );
    fs::write(
        loop_dir.join("20260304020000-quick-dev-codex-review-changes-requested.md"),
        &artifact_content,
    )
    .expect("write review artifact");

    // Replace the implementer mock with one that captures the prompt and
    // verifies it contains the review feedback.
    let capture_script = _temp.path().join("mock_impl_capture.sh");
    let capture_wrapper = _temp.path().join("mock_impl_capture_wrapper.sh");
    let capture_prompt_file = _temp.path().join(".captured-prompt.txt");
    let capture_script_content = format!(
        r#"#!/bin/bash
set -euo pipefail
prompt="$(cat)"

# Only capture the first prompt (the apply-fixes call); do not overwrite on
# subsequent calls (e.g. final review).
capture_file="{}"
if [[ ! -f "$capture_file" ]]; then
    echo "$prompt" > "$capture_file"
fi

# Check if this is a final review prompt
if [[ "$prompt" == *"final review"* ]] || [[ "$prompt" == *"Final Review"* ]] || [[ "$prompt" == *"final-review"* ]]; then
    cat <<'EOF'
# Final Review: COMPLETE

All requirements are met from implementer perspective.
EOF
else
    cat <<'EOF'
# Implementation Notes

## Decisions Made
- Applied the requested fixes

## Spec Deviations
- None

## Testing
- All tests pass
EOF
fi
"#,
        capture_prompt_file.to_string_lossy()
    );
    fs::write(&capture_script, capture_script_content).expect("write capture script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&capture_script, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }
    write_bash_wrapper(&capture_wrapper, &capture_script);

    // Update workspace config to use the capture script as the implementer
    let mut workspace = Workspace::load(workspace_root.clone()).expect("load workspace");
    workspace.config.backends.claude.command = capture_wrapper.to_string_lossy().to_string();
    workspace.save_config().expect("save config");

    let workspace = Workspace::load(workspace_root).expect("reload workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should resume from ApplyFixes");

    assert!(
        result.summary.contains("completed"),
        "expected completion, got: {}",
        result.summary
    );

    // Verify the captured prompt (the first backend call, which is apply-fixes)
    // contains the review feedback from the persisted artifact.
    let captured = fs::read_to_string(&capture_prompt_file)
        .expect("read captured prompt");
    assert!(
        captured.contains("error handling"),
        "apply-fixes prompt should contain review feedback from artifact, got: {}",
        &captured[..captured.len().min(500)]
    );
}

// ---------------------------------------------------------------------------
// Integration tests - commit guard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn skip_commit_prevents_git_commits() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let before_hash = git_output(_temp.path(), &["rev-parse", "HEAD"]);

    let _ = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should succeed");

    let after_hash = git_output(_temp.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        before_hash, after_hash,
        "skip_commit=true should prevent any new commits"
    );
}

// ---------------------------------------------------------------------------
// Regression tests - canonical backend equality
// ---------------------------------------------------------------------------

#[tokio::test]
async fn whitespace_equal_backend_rejection() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let err = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend: Some(" claude ".to_owned()),
            reviewer_backend: Some("claude".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect_err("should fail when backends are semantically equal");

    let msg = err.to_string();
    assert!(
        msg.contains("distinct"),
        "expected distinct-backend error, got: {msg}"
    );
}

#[tokio::test]
async fn whitespace_padded_model_equal_backend_rejection() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let err = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id),
            implementer_backend: Some(" claude(opus) ".to_owned()),
            reviewer_backend: Some("claude(opus)".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect_err("should fail when backends are semantically equal (whitespace-padded model)");

    let msg = err.to_string();
    assert!(
        msg.contains("distinct"),
        "expected distinct-backend error, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Regression tests - crash-durable counter persistence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn force_complete_persists_final_review_attempts() {
    let (_temp, workspace_root, project_id) =
        setup_quick_dev_workspace("satisfied", "issues_found");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: Some(2),
        })
        .await
        .expect("orchestrator should force-complete");

    assert!(
        result.summary.contains("force-completed"),
        "expected 'force-completed' in summary, got: {}",
        result.summary
    );

    // Verify persisted counter matches the number of attempts made
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let state_json = fs::read_to_string(project_dir.join("state.json")).expect("read state.json");
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();

    assert_eq!(
        state["quick_dev_final_review_attempts"].as_u64().unwrap_or(0),
        2,
        "persisted final_review_attempts should equal max_final_review_retries after force-complete"
    );
}

#[tokio::test]
async fn review_iteration_persisted_after_changes_requested() {
    // Use "loop_then_satisfied" so the reviewer rejects once then accepts
    let (_temp, workspace_root, project_id) =
        setup_quick_dev_workspace("loop_then_satisfied", "complete");

    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let _ = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("orchestrator should succeed");

    // Verify persisted counter reflects the review iteration
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let state_json = fs::read_to_string(project_dir.join("state.json")).expect("read state.json");
    let state: serde_json::Value = serde_json::from_str(&state_json).unwrap();

    // After one rejection + one satisfaction, review_iteration should be at least 1
    let review_iter = state["quick_dev_review_iteration"].as_u64().unwrap_or(0);
    assert!(
        review_iter >= 1,
        "persisted quick_dev_review_iteration should be >= 1 after changes_requested, got: {}",
        review_iter
    );
}

// ---------------------------------------------------------------------------
// Regression tests - transition/checkpoint failure preserves counters
// ---------------------------------------------------------------------------

/// Simulates a crash between counter persistence and transition/checkpoint
/// completion in the CodexReview phase. After `review_iteration` is incremented
/// and persisted via `save_state_to_disk`, but before the CodexReview -> ApplyFixes
/// transition completes, the process crashes.
///
/// On resume, the invariants are:
/// - Persisted counters never decrease from their pre-crash values.
/// - `status` remains `in_progress` until an explicit completion path is reached.
/// - Resume continues from the last persisted `quick_dev_phase` and counters.
#[tokio::test]
async fn transition_failure_preserves_review_counter_on_resume() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Write persisted state simulating a crash at the CodexReview -> ApplyFixes
    // boundary: review_iteration=2 was already incremented and persisted, but
    // the transition to ApplyFixes never completed (quick_dev_phase is still
    // CodexReview).
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let mut state = ralph::project::state::ProjectState::new(
        &project_id,
        "Quick Dev Test",
        "hash123",
        None,
    );
    state.quick_dev_phase = Some(QuickDevPhase::CodexReview);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::Reviewing;
    state.phase_iteration = 1;
    state.quick_dev_review_iteration = 2;
    state.quick_dev_final_review_attempts = 0;
    state.register_feature_loop(
        1,
        "quick-dev".to_owned(),
        "Quick Dev".to_owned(),
        ralph::project::state::FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "claude".to_owned(),
            reviewer: "codex".to_owned(),
            qa: String::new(),
        },
        String::new(),
        chrono::Utc::now(),
    );
    // Restore phase/counter state after register_feature_loop (which overrides)
    state.quick_dev_phase = Some(QuickDevPhase::CodexReview);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::Reviewing;
    state.quick_dev_review_iteration = 2;
    let state_json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(project_dir.join("state.json"), &state_json).expect("write crash-simulated state");

    // Resume: reviewer returns SATISFIED -> FinalReview (COMPLETE) -> Completed
    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("resumed run should complete");

    assert!(
        result.summary.contains("completed"),
        "resumed run should complete, got: {}",
        result.summary
    );

    // Verify counter invariants
    let final_state_json =
        fs::read_to_string(project_dir.join("state.json")).expect("read final state.json");
    let final_state: serde_json::Value =
        serde_json::from_str(&final_state_json).expect("parse final state.json");

    // review_iteration must never decrease from persisted pre-crash value
    let review_iter = final_state["quick_dev_review_iteration"].as_u64().unwrap_or(0);
    assert!(
        review_iter >= 2,
        "review_iteration must not decrease from persisted value 2 after resume, got: {}",
        review_iter
    );

    // Status must be completed (explicit completion path reached)
    assert_eq!(
        final_state["status"].as_str().unwrap(),
        "completed",
        "status must be completed after explicit completion path"
    );

    // quick_dev_phase must be None after completion
    assert!(
        final_state["quick_dev_phase"].is_null(),
        "quick_dev_phase must be null after completion"
    );
}

/// Same as above but for FinalReview: crash after `final_review_attempts` is
/// incremented and persisted, before the FinalReview -> PlanAndImplement
/// transition completes. Verifies the same invariants apply to
/// `final_review_attempts`.
#[tokio::test]
async fn transition_failure_preserves_final_review_counter_on_resume() {
    let (_temp, workspace_root, project_id) = setup_quick_dev_workspace("satisfied", "complete");

    // Write persisted state: final_review_attempts=1 was incremented and
    // persisted, but the transition to PlanAndImplement never completed
    // (quick_dev_phase is still FinalReview, status is still in_progress).
    let project_dir = _temp.path().join(".ralph/projects").join(&project_id);
    let mut state = ralph::project::state::ProjectState::new(
        &project_id,
        "Quick Dev Test",
        "hash123",
        None,
    );
    state.quick_dev_phase = Some(QuickDevPhase::FinalReview);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::FinalReview;
    state.phase_iteration = 1;
    state.quick_dev_review_iteration = 0;
    state.quick_dev_final_review_attempts = 1;
    state.register_feature_loop(
        1,
        "quick-dev".to_owned(),
        "Quick Dev".to_owned(),
        ralph::project::state::FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "claude".to_owned(),
            reviewer: "codex".to_owned(),
            qa: String::new(),
        },
        String::new(),
        chrono::Utc::now(),
    );
    // Restore after register_feature_loop overrides
    state.quick_dev_phase = Some(QuickDevPhase::FinalReview);
    state.status = ProjectStatus::InProgress;
    state.current_phase = Phase::FinalReview;
    state.quick_dev_final_review_attempts = 1;
    let state_json = serde_json::to_string_pretty(&state).unwrap();
    fs::write(project_dir.join("state.json"), &state_json).expect("write crash-simulated state");

    // Resume: both final reviews return COMPLETE -> Completed
    let workspace = Workspace::load(workspace_root).expect("load workspace");
    let mut orchestrator = QuickDevOrchestrator::new(workspace);

    let result = orchestrator
        .run(QuickDevRunOptions {
            project: Some(project_id.clone()),
            implementer_backend: Some("claude".to_owned()),
            reviewer_backend: Some("codex".to_owned()),
            pr_url: None,
            skip_commit: true,
            max_review_iterations: None,
            max_final_review_retries: None,
        })
        .await
        .expect("resumed run should complete");

    assert!(
        result.summary.contains("completed"),
        "resumed run should complete, got: {}",
        result.summary
    );

    // Verify counter invariants
    let final_state_json =
        fs::read_to_string(project_dir.join("state.json")).expect("read final state.json");
    let final_state: serde_json::Value =
        serde_json::from_str(&final_state_json).expect("parse final state.json");

    // final_review_attempts must never decrease from persisted pre-crash value
    let fr_attempts = final_state["quick_dev_final_review_attempts"].as_u64().unwrap_or(0);
    assert!(
        fr_attempts >= 1,
        "final_review_attempts must not decrease from persisted value 1 after resume, got: {}",
        fr_attempts
    );

    // Status must be completed (explicit completion path reached)
    assert_eq!(
        final_state["status"].as_str().unwrap(),
        "completed",
        "status must be completed after explicit completion path"
    );

    // quick_dev_phase must be None after completion
    assert!(
        final_state["quick_dev_phase"].is_null(),
        "quick_dev_phase must be null after completion"
    );
}
