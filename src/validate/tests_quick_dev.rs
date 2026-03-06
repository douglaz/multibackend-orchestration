use std::fs;
use std::process::Command;

use super::panic_message;
use crate::validate::assertions::{
    assert_exit_code, assert_file_contains, assert_file_exists, assert_stderr_contains,
    assert_stdout_contains,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    quick_dev_final_review_always_issues_script, quick_dev_final_review_issues_once_script,
    quick_dev_implementer_mock_script, quick_dev_reviewer_always_reject_script,
    quick_dev_reviewer_mock_script, quick_dev_reviewer_reject_once_script,
};
use crate::validate::runner::{ConformanceTest, TestResult};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "quick_dev::happy_path_completes",
            func: happy_path_completes,
        },
        ConformanceTest {
            name: "quick_dev::review_loop_changes_requested",
            func: review_loop_changes_requested,
        },
        ConformanceTest {
            name: "quick_dev::final_review_reloop",
            func: final_review_reloop,
        },
        ConformanceTest {
            name: "quick_dev::max_review_iterations_guard",
            func: max_review_iterations_guard,
        },
        ConformanceTest {
            name: "quick_dev::max_final_review_retries_guard",
            func: max_final_review_retries_guard,
        },
        ConformanceTest {
            name: "quick_dev::resume_from_codex_review",
            func: resume_from_codex_review,
        },
        ConformanceTest {
            name: "quick_dev::resume_from_final_review",
            func: resume_from_final_review,
        },
        ConformanceTest {
            name: "quick_dev::resume_from_none_starts_plan_and_implement",
            func: resume_from_none_starts_plan_and_implement,
        },
        ConformanceTest {
            name: "quick_dev::reviewer_backend_missing_fails",
            func: reviewer_backend_missing_fails,
        },
        ConformanceTest {
            name: "quick_dev::equal_backends_fails",
            func: equal_backends_fails,
        },
        ConformanceTest {
            name: "quick_dev::initial_checkpoint_planning_to_implementing",
            func: initial_checkpoint_planning_to_implementing,
        },
        ConformanceTest {
            name: "quick_dev::auto_missing_reviewer_fails_fast",
            func: auto_missing_reviewer_fails_fast,
        },
        ConformanceTest {
            name: "quick_dev::auto_equal_backends_fails_fast",
            func: auto_equal_backends_fails_fast,
        },
        ConformanceTest {
            name: "quick_dev::reconstruction_restores_quick_dev_fields",
            func: reconstruction_restores_quick_dev_fields,
        },
        ConformanceTest {
            name: "quick_dev::non_quick_completed_not_reclassified",
            func: non_quick_completed_not_reclassified,
        },
        ConformanceTest {
            name: "quick_dev::auto_optional_reviewer_fails_fast",
            func: auto_optional_reviewer_fails_fast,
        },
        ConformanceTest {
            name: "quick_dev::auto_unknown_reviewer_fails_fast",
            func: auto_unknown_reviewer_fails_fast,
        },
        ConformanceTest {
            name: "quick_dev::auto_whitespace_equal_backends_fails_fast",
            func: auto_whitespace_equal_backends_fails_fast,
        },
        ConformanceTest {
            name: "quick_dev::force_complete_persists_counter",
            func: force_complete_persists_counter,
        },
        ConformanceTest {
            name: "quick_dev::guard_at_entry_codex_review_skips_to_final_review",
            func: guard_at_entry_codex_review_skips_to_final_review,
        },
        ConformanceTest {
            name: "quick_dev::guard_at_entry_final_review_force_completes",
            func: guard_at_entry_final_review_force_completes,
        },
        ConformanceTest {
            name: "quick_dev::transition_boundary_resume_persists_destination",
            func: transition_boundary_resume_persists_destination,
        },
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

/// Set up a quick-dev workspace with separate implementer (claude) and reviewer
/// (codex) mock backends.  Uses /bin/sh wrappers for Nix sandbox compatibility.
fn setup_quick_dev(h: &RalphHarness, project_id: &str, impl_script: &str, rev_script: &str) {
    h.init_workspace().expect("init failed");

    let impl_path = h
        .write_mock_script("qd-implementer.sh", impl_script)
        .expect("write implementer mock");
    let rev_path = h
        .write_mock_script("qd-reviewer.sh", rev_script)
        .expect("write reviewer mock");

    // Create /bin/sh wrappers for Nix compatibility
    let impl_wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_path.display());
    let rev_wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", rev_path.display());
    let impl_wrapper = h
        .write_mock_script("qd-impl-wrapper.sh", &impl_wrapper_content)
        .expect("write impl wrapper");
    let rev_wrapper = h
        .write_mock_script("qd-rev-wrapper.sh", &rev_wrapper_content)
        .expect("write rev wrapper");

    let impl_wrapper_str = impl_wrapper.to_string_lossy().into_owned();
    let rev_wrapper_str = rev_wrapper.to_string_lossy().into_owned();

    // Configure claude backend -> implementer mock
    for (backend, wrapper) in &[("claude", &impl_wrapper_str), ("codex", &rev_wrapper_str)] {
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.command"),
            wrapper.to_string(),
            "--global".to_owned(),
        ])
        .unwrap_or_else(|e| panic!("set {backend} command failed: {e}"));
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.args"),
            "[]".to_owned(),
            "--global".to_owned(),
        ])
        .unwrap_or_else(|e| panic!("set {backend} args failed: {e}"));
    }

    // Disable openrouter
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.openrouter.enabled".to_owned(),
        "false".to_owned(),
        "--global".to_owned(),
    ])
    .expect("disable openrouter");

    h.create_project(
        project_id,
        "Quick-Dev Test Project",
        "Quick-dev test prompt",
    )
    .expect("create_project failed");
}

/// Load project state from state.json on disk (written by the quick-dev
/// orchestrator). This is used instead of `h.load_state()` because the CLI
/// `project show --json` route goes through `reconstruct_project_state` which
/// doesn't propagate `current_phase` from the quick-dev state.json file.
fn load_state_json(h: &RalphHarness, project_id: &str) -> serde_json::Value {
    let state_path = h.project_dir(project_id).join("state.json");
    let content = fs::read_to_string(&state_path)
        .unwrap_or_else(|e| panic!("failed to read state.json: {e}"));
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("failed to parse state.json: {e}"))
}

/// Load project state via CLI reconstruction (for non-quick-dev-specific fields).
fn load_state(h: &RalphHarness, project_id: &str) -> serde_json::Value {
    h.load_state(project_id).expect("load_state failed")
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

/// Happy path: PlanAndImplement -> CodexReview (SATISFIED) -> FinalReview (both COMPLETE)
/// -> Completed.
fn happy_path_completes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-happy-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "quick-dev completed successfully");

        let state = load_state_json(h, project_id);
        assert_eq!(
            state["status"].as_str().unwrap(),
            "completed",
            "expected status=completed"
        );
        assert_eq!(
            state["current_phase"].as_str().unwrap(),
            "completing",
            "expected current_phase=completing"
        );
        assert!(
            state["quick_dev_phase"].is_null(),
            "expected quick_dev_phase=null after completion"
        );
    })
}

/// Review loop: reviewer requests changes once, implementer applies fixes,
/// reviewer then approves.  Verifies the CodexReview -> ApplyFixes -> CodexReview loop.
fn review_loop_changes_requested(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-review-loop-001";

        let state_file = h.temp_dir.path().join("qd-review-state-loop");
        let state_file_str = state_file.to_string_lossy().into_owned();

        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_reject_once_script(),
        );

        let output = h
            .ralph_env(
                [
                    "quick-dev-run",
                    "--project",
                    project_id,
                    "--implementer-backend",
                    "claude",
                    "--reviewer-backend",
                    "codex",
                    "--skip-commit",
                ],
                &[("QUICK_DEV_REVIEW_STATE_FILE", &state_file_str)],
            )
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "quick-dev completed successfully");

        // The state file should exist (reviewer was called and rejected once)
        assert_file_exists(&state_file);

        let state = load_state_json(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");
        assert_eq!(state["current_phase"].as_str().unwrap(), "completing");
        assert!(state["quick_dev_phase"].is_null());
    })
}

/// Final review reloop: first FinalReview finds issues (both backends return
/// AMENDMENTS for the first 2 calls), causing a transition back to
/// PlanAndImplement. Second time through, FinalReview both return NO AMENDMENTS.
fn final_review_reloop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-fr-reloop-001";

        let fr_state_file = h.temp_dir.path().join("qd-fr-state-reloop");
        let fr_state_str = fr_state_file.to_string_lossy().into_owned();

        setup_quick_dev(
            h,
            project_id,
            &quick_dev_final_review_issues_once_script(),
            &quick_dev_final_review_issues_once_script(),
        );

        let output = h
            .ralph_env(
                [
                    "quick-dev-run",
                    "--project",
                    project_id,
                    "--implementer-backend",
                    "claude",
                    "--reviewer-backend",
                    "codex",
                    "--skip-commit",
                    "--max-final-review-retries",
                    "3",
                ],
                &[("QUICK_DEV_FR_STATE_FILE", &fr_state_str)],
            )
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "quick-dev completed successfully");

        let state = load_state_json(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");
        assert_eq!(state["current_phase"].as_str().unwrap(), "completing");
        assert!(state["quick_dev_phase"].is_null());

        // Final review state file should have been incremented
        assert_file_exists(&fr_state_file);
    })
}

/// Max review iterations guard: reviewer always requests changes, hitting the
/// max_review_iterations limit. Should skip to FinalReview, write a warning
/// artifact, and not fail.
fn max_review_iterations_guard(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-max-review-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_always_reject_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
                "--max-review-iterations",
                "2",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        // Verify warning artifact was written
        let project_dir = h.project_dir(project_id);
        let warning_path = project_dir.join("quick-dev-review-limit-warning.md");
        assert_file_exists(&warning_path);
        assert_file_contains(&warning_path, "Review Iteration Limit Reached");

        let state = load_state_json(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");
        assert_eq!(state["current_phase"].as_str().unwrap(), "completing");
    })
}

/// Max final review retries guard: final review always finds issues, hitting
/// the max_final_review_retries limit. Should write force-complete artifact,
/// set status=Completed, current_phase=Completing, quick_dev_phase=None.
fn max_final_review_retries_guard(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-max-fr-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_final_review_always_issues_script(),
            &quick_dev_final_review_always_issues_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
                "--max-final-review-retries",
                "1",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "force-completed");

        // Verify force-complete artifact
        let project_dir = h.project_dir(project_id);
        let force_path = project_dir.join("quick-dev-force-complete.md");
        assert_file_exists(&force_path);
        assert_file_contains(&force_path, "Force Complete");

        let state = load_state_json(h, project_id);
        assert_eq!(
            state["status"].as_str().unwrap(),
            "completed",
            "status must be completed after force-complete"
        );
        assert_eq!(
            state["current_phase"].as_str().unwrap(),
            "completing",
            "current_phase must be completing after force-complete"
        );
        assert!(
            state["quick_dev_phase"].is_null(),
            "quick_dev_phase must be None after force-complete"
        );
    })
}

/// Resume from persisted CodexReview phase via direct state seeding.
/// Collision-safe: no prior run is performed, so artifact-delta assertions
/// cannot suffer from timestamp-based overwrite false positives.
/// Phase-sensitive: resume must NOT create a plan-implement artifact
/// and MUST produce codex-review artifacts first.
fn resume_from_codex_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-resume-cr-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Directly seed state.json with CodexReview phase (no prior run).
        // This avoids timestamp-overwrite false positives from a prior full run.
        let project_dir = h.project_dir(project_id);
        let state_path = project_dir.join("state.json");
        let seeded_state = serde_json::json!({
            "project_id": project_id,
            "name": "Quick-Dev Test Project",
            "prompt_hash": "seed",
            "current_loop": 1,
            "current_phase": "reviewing",
            "quick_dev_phase": "codex_review",
            "phase_iteration": 1,
            "status": "in_progress",
            "loops": [{
                "loop_number": 1,
                "slug": "quick-dev",
                "feature_name": "Quick Dev",
                "loop_type": "Feature",
                "status": "InProgress",
                "backends": {
                    "planner": "claude",
                    "implementer": "claude",
                    "reviewer": "codex",
                    "qa": ""
                },
                "artifacts": {
                    "spec": "",
                    "impl_notes": null,
                    "reviews": [],
                    "approval": null,
                    "qa_results": [],
                    "pending_qa_feedback": null
                },
                "commit": null,
                "started_at": "2026-01-01T00:00:00Z",
                "completed_at": null
            }],
            "completion_attempts": [],
            "prompt_file": "prompt.md",
            "created_at": "2026-01-01T00:00:00Z"
        });
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&seeded_state).unwrap(),
        )
        .expect("write seeded state");

        // Create the loop directory so artifacts can be written
        let loops_dir = project_dir.join("loops").join("001-quick-dev");
        fs::create_dir_all(&loops_dir).expect("create loop dir");

        // Pre-artifact snapshot: should be empty (no prior run)
        let pre_artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts before resume");
        assert!(
            pre_artifacts.is_empty(),
            "seeded resume should start with no artifacts; found {}",
            pre_artifacts.len()
        );

        // Resume from CodexReview
        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("resume quick-dev-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        let state = load_state(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");

        // Phase-sensitive artifact-delta assertions:
        let post_artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts after resume");
        let artifact_names: Vec<String> = post_artifacts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        // 1. No plan-implement artifact should exist (resume must not restart)
        assert!(
            !artifact_names
                .iter()
                .any(|n| n.contains("quick-dev-plan-implement")),
            "resume from CodexReview must NOT create plan-implement artifact; found: {:?}",
            artifact_names
        );

        // 2. First new artifact must be codex-review (proving phase-order)
        assert!(
            !artifact_names.is_empty(),
            "resume from CodexReview must produce artifacts"
        );
        assert!(
            artifact_names[0].contains("quick-dev-codex-review"),
            "first artifact after CodexReview resume must be codex-review, got: {}",
            artifact_names[0]
        );
    })
}

/// Resume from persisted FinalReview phase via direct state seeding.
/// Collision-safe: no prior run is performed, so artifact-delta assertions
/// cannot suffer from timestamp-based overwrite false positives.
/// Phase-sensitive: resume must NOT create plan-implement or apply-fixes
/// artifacts (no restart path).
fn resume_from_final_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-resume-fr-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Directly seed state.json with FinalReview phase (no prior run).
        // This avoids timestamp-overwrite false positives from a prior full run.
        let project_dir = h.project_dir(project_id);
        let state_path = project_dir.join("state.json");
        let seeded_state = serde_json::json!({
            "project_id": project_id,
            "name": "Quick-Dev Test Project",
            "prompt_hash": "seed",
            "current_loop": 1,
            "current_phase": "final_review",
            "quick_dev_phase": "final_review",
            "phase_iteration": 1,
            "status": "in_progress",
            "loops": [{
                "loop_number": 1,
                "slug": "quick-dev",
                "feature_name": "Quick Dev",
                "loop_type": "Feature",
                "status": "InProgress",
                "backends": {
                    "planner": "claude",
                    "implementer": "claude",
                    "reviewer": "codex",
                    "qa": ""
                },
                "artifacts": {
                    "spec": "",
                    "impl_notes": null,
                    "reviews": [],
                    "approval": null,
                    "qa_results": [],
                    "pending_qa_feedback": null
                },
                "commit": null,
                "started_at": "2026-01-01T00:00:00Z",
                "completed_at": null
            }],
            "completion_attempts": [],
            "prompt_file": "prompt.md",
            "created_at": "2026-01-01T00:00:00Z"
        });
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&seeded_state).unwrap(),
        )
        .expect("write seeded state");

        // Create the loop directory so artifacts can be written
        let loops_dir = project_dir.join("loops").join("001-quick-dev");
        fs::create_dir_all(&loops_dir).expect("create loop dir");

        // Pre-artifact snapshot: should be empty (no prior run)
        let pre_artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts before resume");
        assert!(
            pre_artifacts.is_empty(),
            "seeded resume should start with no artifacts; found {}",
            pre_artifacts.len()
        );

        // Resume from FinalReview
        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("resume quick-dev-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        let state = load_state(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");

        // Phase-sensitive artifact-delta assertions:
        // All artifacts are new (no prior run), so delta = full inventory.
        let post_artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts after resume");
        let artifact_names: Vec<String> = post_artifacts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        for name in &artifact_names {
            assert!(
                !name.contains("quick-dev-plan-implement"),
                "resume from FinalReview must NOT create plan-implement artifact; found: {name}"
            );
            assert!(
                !name.contains("quick-dev-apply-fixes"),
                "resume from FinalReview must NOT create apply-fixes artifact; found: {name}"
            );
        }
    })
}

/// Resume from None quick_dev_phase starts at PlanAndImplement.
/// Phase-sensitive: must create a plan-implement artifact as the first phase
/// marker, proving default start phase is PlanAndImplement.
fn resume_from_none_starts_plan_and_implement(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-resume-none-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // State has quick_dev_phase = null (None) — this is the default after
        // project creation. Running quick-dev-run should start from PlanAndImplement.
        let state = load_state(h, project_id);
        assert!(
            state["quick_dev_phase"].is_null(),
            "fresh project should have quick_dev_phase=null"
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        let state = load_state(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");

        // Phase-sensitive artifact assertion: starting from None must produce
        // a plan-implement artifact, proving default start phase is PlanAndImplement.
        let artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts should succeed");
        assert!(
            !artifacts.is_empty(),
            "starting from None must produce artifacts"
        );
        assert!(
            artifacts.iter().any(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("quick-dev-plan-implement")
            }),
            "starting from None must produce a plan-implement artifact; got: {:?}",
            artifacts
        );
    })
}

/// Reviewer backend missing: quick-dev-run with no --reviewer-backend and no
/// configured reviewer_backend should fail with a clear error.
fn reviewer_backend_missing_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-no-reviewer-001";

        h.init_workspace().expect("init failed");

        // Set up only the implementer backend (claude)
        let impl_script = h
            .write_mock_script("qd-impl-only.sh", &quick_dev_implementer_mock_script())
            .expect("write impl mock");
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_script.display());
        let wrapper = h
            .write_mock_script("qd-impl-only-wrapper.sh", &wrapper_content)
            .expect("write wrapper");
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.claude.command".to_owned(),
            wrapper_str.clone(),
            "--global".to_owned(),
        ])
        .expect("set claude command");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.claude.args".to_owned(),
            "[]".to_owned(),
            "--global".to_owned(),
        ])
        .expect("set claude args");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "false".to_owned(),
            "--global".to_owned(),
        ])
        .expect("disable openrouter");

        h.create_project(project_id, "No Reviewer Project", "test prompt")
            .expect("create_project");

        // Run with only implementer backend, no reviewer
        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");

        // Should fail with exit code 2 (Validation error)
        assert_exit_code(&output, 2);
        assert_stderr_contains(&output, "quick-dev requires a second backend for review");

        // No completion artifacts should exist
        let project_dir = h.project_dir(project_id);
        let force_path = project_dir.join("quick-dev-force-complete.md");
        assert!(
            !force_path.exists(),
            "no force-complete artifact should exist"
        );

        // State should not be completed
        let state = load_state(h, project_id);
        assert_ne!(
            state["status"].as_str().unwrap_or(""),
            "completed",
            "status must not be completed"
        );
    })
}

/// Equal implementer and reviewer backends should fail with a clear error.
fn equal_backends_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-equal-backends-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Pass same backend for both implementer and reviewer
        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "claude",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");

        // Should fail with exit code 2 (Validation error)
        assert_exit_code(&output, 2);
        assert_stderr_contains(
            &output,
            "quick-dev requires distinct implementer and reviewer backends",
        );

        // No completion artifacts
        let project_dir = h.project_dir(project_id);
        assert!(
            !project_dir.join("quick-dev-force-complete.md").exists(),
            "no force-complete artifact should exist"
        );

        // State should not be completed
        let state = load_state(h, project_id);
        assert_ne!(
            state["status"].as_str().unwrap_or(""),
            "completed",
            "status must not be completed after equal-backends error"
        );
    })
}

/// Regression: the initial `start -> PlanAndImplement` transition must emit a
/// `planning -> implementing` checkpoint when auto-commit is enabled and there
/// are changes outside `.ralph/` to commit.
fn initial_checkpoint_planning_to_implementing(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-checkpoint-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Create a file outside .ralph/ so the initial checkpoint has changes to commit.
        let seed_file = h.repo_root.join("seed-for-checkpoint.txt");
        fs::write(&seed_file, "seed content for initial checkpoint test").expect("write seed file");

        // Run WITHOUT --skip-commit so checkpoints fire.
        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        // Verify the initial planning -> implementing checkpoint exists in git log.
        let git_log = Command::new("git")
            .args(["log", "--format=%s"])
            .current_dir(&h.repo_root)
            .output()
            .expect("git log failed");
        let log_output = String::from_utf8_lossy(&git_log.stdout);
        let expected_msg = format!("ralph({project_id}): loop 1 planning -> implementing");
        assert!(
            log_output.contains(&expected_msg),
            "expected '{expected_msg}' in git log, got:\n{log_output}"
        );
    })
}

/// `quick-dev-auto` with missing reviewer backend must fail-fast (exit 2)
/// before creating a project directory or running quick-prd.
fn auto_missing_reviewer_fails_fast(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let impl_script = h
            .write_mock_script("qd-auto-impl.sh", &quick_dev_implementer_mock_script())
            .expect("write impl mock");
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_script.display());
        let wrapper = h
            .write_mock_script("qd-auto-impl-wrapper.sh", &wrapper_content)
            .expect("write wrapper");
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.claude.command".to_owned(),
            wrapper_str,
            "--global".to_owned(),
        ])
        .expect("set claude command");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.claude.args".to_owned(),
            "[]".to_owned(),
            "--global".to_owned(),
        ])
        .expect("set claude args");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "false".to_owned(),
            "--global".to_owned(),
        ])
        .expect("disable openrouter");

        let output = h
            .ralph([
                "quick-dev-auto",
                "--idea",
                "auto-missing-reviewer-test",
                "--implementer-backend",
                "claude",
            ])
            .expect("quick-dev-auto should execute");

        // Must fail with exit code 2
        assert_exit_code(&output, 2);
        assert_stderr_contains(&output, "quick-dev requires a second backend for review");

        // No project should have been created
        let project_dir = h.project_dir("auto-missing-reviewer-test");
        assert!(
            !project_dir.exists(),
            "project directory must not exist after fail-fast: {}",
            project_dir.display()
        );
    })
}

/// `quick-dev-auto` with equal implementer/reviewer backends must fail-fast
/// (exit 2) before creating a project directory or running quick-prd.
fn auto_equal_backends_fails_fast(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let impl_script = h
            .write_mock_script("qd-auto-eq-impl.sh", &quick_dev_implementer_mock_script())
            .expect("write impl mock");
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_script.display());
        let wrapper = h
            .write_mock_script("qd-auto-eq-wrapper.sh", &wrapper_content)
            .expect("write wrapper");
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        for backend in &["claude", "codex"] {
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.command"),
                wrapper_str.clone(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} command failed: {e}"));
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.args"),
                "[]".to_owned(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} args failed: {e}"));
        }
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "false".to_owned(),
            "--global".to_owned(),
        ])
        .expect("disable openrouter");

        let output = h
            .ralph([
                "quick-dev-auto",
                "--idea",
                "auto-equal-backends-test",
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "claude",
            ])
            .expect("quick-dev-auto should execute");

        assert_exit_code(&output, 2);
        assert_stderr_contains(
            &output,
            "quick-dev requires distinct implementer and reviewer backends",
        );

        // No project should have been created
        let project_dir = h.project_dir("auto-equal-backends-test");
        assert!(
            !project_dir.exists(),
            "project directory must not exist after fail-fast: {}",
            project_dir.display()
        );
    })
}

/// After a successful quick-dev-run, `h.load_state()` (which uses
/// `reconstruct_project_state`) must reflect the persisted quick-dev fields
/// including `current_phase` and `phase_iteration`.
fn reconstruction_restores_quick_dev_fields(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-recon-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");
        assert_exit_code(&output, 0);

        // Verify via reconstruction (h.load_state) — not just state.json
        let state = load_state(h, project_id);
        assert_eq!(
            state["status"].as_str().unwrap(),
            "completed",
            "reconstructed status must be completed"
        );
        assert_eq!(
            state["current_phase"].as_str().unwrap(),
            "completing",
            "reconstructed current_phase must be completing"
        );

        // Also verify that state.json matches reconstruction
        let state_json = load_state_json(h, project_id);
        assert_eq!(
            state_json["current_phase"].as_str().unwrap(),
            "completing",
            "state.json current_phase must match reconstruction"
        );
    })
}

/// `quick-dev-auto` with optional reviewer backend (`?codex`) must fail-fast
/// (exit 2). Optional (`?`) prefixes are only allowed on panel surfaces.
fn auto_optional_reviewer_fails_fast(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let impl_script = h
            .write_mock_script("qd-auto-opt-impl.sh", &quick_dev_implementer_mock_script())
            .expect("write impl mock");
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_script.display());
        let wrapper = h
            .write_mock_script("qd-auto-opt-wrapper.sh", &wrapper_content)
            .expect("write wrapper");
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        for backend in &["claude", "codex"] {
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.command"),
                wrapper_str.clone(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} command failed: {e}"));
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.args"),
                "[]".to_owned(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} args failed: {e}"));
        }
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "false".to_owned(),
            "--global".to_owned(),
        ])
        .expect("disable openrouter");

        let output = h
            .ralph([
                "quick-dev-auto",
                "--idea",
                "auto-optional-reviewer-test",
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "?codex",
            ])
            .expect("quick-dev-auto should execute");

        // Must fail with exit code 2
        assert_exit_code(&output, 2);
        assert_stderr_contains(&output, "optional backend specs");

        // No project should have been created
        let project_dir = h.project_dir("auto-optional-reviewer-test");
        assert!(
            !project_dir.exists(),
            "project directory must not exist after fail-fast: {}",
            project_dir.display()
        );
    })
}

/// `quick-dev-auto` with an unknown reviewer backend must fail-fast (exit 2).
fn auto_unknown_reviewer_fails_fast(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let impl_script = h
            .write_mock_script("qd-auto-gem-impl.sh", &quick_dev_implementer_mock_script())
            .expect("write impl mock");
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_script.display());
        let wrapper = h
            .write_mock_script("qd-auto-gem-wrapper.sh", &wrapper_content)
            .expect("write wrapper");
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        for backend in &["claude", "codex"] {
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.command"),
                wrapper_str.clone(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} command failed: {e}"));
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.args"),
                "[]".to_owned(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} args failed: {e}"));
        }

        let output = h
            .ralph([
                "quick-dev-auto",
                "--idea",
                "auto-unknown-reviewer-test",
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "badbackend",
            ])
            .expect("quick-dev-auto should execute");

        // Must fail with exit code 2
        assert_exit_code(&output, 2);
        assert_stderr_contains(&output, "unknown backend configured as quick-dev reviewer backend");

        // No project should have been created
        let project_dir = h.project_dir("auto-unknown-reviewer-test");
        assert!(
            !project_dir.exists(),
            "project directory must not exist after fail-fast: {}",
            project_dir.display()
        );
    })
}

/// Non-quick projects with `status=completed` must not be reclassified by
/// quick-dev reconstruction logic.  Only state.json files written by the
/// quick-dev orchestrator affect completion status.
fn non_quick_completed_not_reclassified(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-non-quick-001";
        h.init_workspace().expect("init failed");
        h.create_project(project_id, "Non-Quick Project", "non-quick test prompt")
            .expect("create_project failed");

        // Verify the project has no quick_dev_phase and is not completed
        let state = load_state(h, project_id);
        assert!(
            state["quick_dev_phase"].is_null(),
            "fresh project must not have quick_dev_phase"
        );
        assert_ne!(
            state["status"].as_str().unwrap_or(""),
            "completed",
            "fresh non-quick project must not be completed"
        );

        // Write a state.json without quick_dev_phase but with status=completed
        // to simulate a non-quick scenario where state.json exists
        let project_dir = h.project_dir(project_id);
        let state_path = project_dir.join("state.json");
        fs::write(
            &state_path,
            r#"{"status":"completed","current_phase":"completing"}"#,
        )
        .expect("write fake state.json");

        // Reconstruction should NOT mark this as completed because
        // there's no quick_dev_phase marker and no completion_attempts
        let state = load_state(h, project_id);
        assert_ne!(
            state["status"].as_str().unwrap_or(""),
            "completed",
            "non-quick project with fake state.json must not be reclassified as completed"
        );
    })
}

/// `quick-dev-auto` must fail with exit code 2 when implementer and reviewer
/// are semantically equal but differently formatted (whitespace-padded), and
/// must not create `.ralph/projects/<id>`.
fn auto_whitespace_equal_backends_fails_fast(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let impl_script = h
            .write_mock_script("qd-auto-ws-impl.sh", &quick_dev_implementer_mock_script())
            .expect("write impl mock");
        let wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_script.display());
        let wrapper = h
            .write_mock_script("qd-auto-ws-wrapper.sh", &wrapper_content)
            .expect("write wrapper");
        let wrapper_str = wrapper.to_string_lossy().into_owned();

        for backend in &["claude", "codex"] {
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.command"),
                wrapper_str.clone(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} command failed: {e}"));
            h.ralph_ok(vec![
                "config".to_owned(),
                "set".to_owned(),
                format!("backends.{backend}.args"),
                "[]".to_owned(),
                "--global".to_owned(),
            ])
            .unwrap_or_else(|e| panic!("set {backend} args failed: {e}"));
        }
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "false".to_owned(),
            "--global".to_owned(),
        ])
        .expect("disable openrouter");

        // Pass semantically equal but differently formatted backends
        let output = h
            .ralph([
                "quick-dev-auto",
                "--idea",
                "auto-ws-equal-test",
                "--implementer-backend",
                " claude ",
                "--reviewer-backend",
                "claude",
            ])
            .expect("quick-dev-auto should execute");

        assert_exit_code(&output, 2);
        assert_stderr_contains(
            &output,
            "quick-dev requires distinct implementer and reviewer backends",
        );

        // No project should have been created
        let project_dir = h.project_dir("auto-ws-equal-test");
        assert!(
            !project_dir.exists(),
            "project directory must not exist after fail-fast: {}",
            project_dir.display()
        );
    })
}

/// After force-complete, the persisted `quick_dev_final_review_attempts` in
/// state.json must reflect the incremented attempt count.
fn force_complete_persists_counter(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-force-counter-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_final_review_always_issues_script(),
            &quick_dev_final_review_always_issues_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
                "--max-final-review-retries",
                "2",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);

        // Verify persisted counter
        let state = load_state_json(h, project_id);
        assert_eq!(
            state["status"].as_str().unwrap_or(""),
            "completed",
            "project must be completed after force-complete"
        );
        assert_eq!(
            state["quick_dev_final_review_attempts"].as_u64().unwrap_or(0),
            2,
            "persisted final_review_attempts must equal max_final_review_retries after force-complete"
        );
    })
}

/// Guard-at-entry for CodexReview: when resuming with review_iteration at the
/// limit, the reviewer backend call must be skipped and flow goes directly to
/// FinalReview. Verifies no codex-review artifacts are created.
fn guard_at_entry_codex_review_skips_to_final_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-guard-cr-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Seed state at CodexReview with review_iteration=2 (at limit when max=2)
        let project_dir = h.project_dir(project_id);
        let state_path = project_dir.join("state.json");
        let seeded_state = serde_json::json!({
            "project_id": project_id,
            "name": "Quick-Dev Test Project",
            "prompt_hash": "seed",
            "current_loop": 1,
            "current_phase": "reviewing",
            "quick_dev_phase": "codex_review",
            "phase_iteration": 1,
            "status": "in_progress",
            "quick_dev_review_iteration": 2,
            "quick_dev_final_review_attempts": 0,
            "loops": [{
                "loop_number": 1,
                "slug": "quick-dev",
                "feature_name": "Quick Dev",
                "loop_type": "Feature",
                "status": "InProgress",
                "backends": {
                    "planner": "claude",
                    "implementer": "claude",
                    "reviewer": "codex",
                    "qa": ""
                },
                "artifacts": {
                    "spec": "",
                    "impl_notes": null,
                    "reviews": [],
                    "approval": null,
                    "qa_results": [],
                    "pending_qa_feedback": null
                },
                "commit": null,
                "started_at": "2026-01-01T00:00:00Z",
                "completed_at": null
            }],
            "completion_attempts": [],
            "prompt_file": "prompt.md",
            "created_at": "2026-01-01T00:00:00Z"
        });
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&seeded_state).unwrap(),
        )
        .expect("write seeded state");

        let loops_dir = project_dir.join("loops").join("001-quick-dev");
        fs::create_dir_all(&loops_dir).expect("create loop dir");

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
                "--max-review-iterations",
                "2",
            ])
            .expect("quick-dev-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        // Verify: no codex-review artifact (reviewer call was skipped)
        let artifacts = h.list_artifacts(project_id, 1).expect("list_artifacts");
        let artifact_names: Vec<String> = artifacts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            !artifact_names.iter().any(|n| n.contains("codex-review")),
            "guard-at-entry should skip CodexReview; found codex-review artifact in: {:?}",
            artifact_names
        );

        let state = load_state_json(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");
    })
}

/// Guard-at-entry for FinalReview: when resuming with final_review_attempts at
/// the limit, both backend calls must be skipped, force-complete artifact
/// written, and project marked completed.
fn guard_at_entry_final_review_force_completes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-guard-fr-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Seed state at FinalReview with final_review_attempts=2 (at limit when max=2)
        let project_dir = h.project_dir(project_id);
        let state_path = project_dir.join("state.json");
        let seeded_state = serde_json::json!({
            "project_id": project_id,
            "name": "Quick-Dev Test Project",
            "prompt_hash": "seed",
            "current_loop": 1,
            "current_phase": "final_review",
            "quick_dev_phase": "final_review",
            "phase_iteration": 1,
            "status": "in_progress",
            "quick_dev_review_iteration": 0,
            "quick_dev_final_review_attempts": 2,
            "loops": [{
                "loop_number": 1,
                "slug": "quick-dev",
                "feature_name": "Quick Dev",
                "loop_type": "Feature",
                "status": "InProgress",
                "backends": {
                    "planner": "claude",
                    "implementer": "claude",
                    "reviewer": "codex",
                    "qa": ""
                },
                "artifacts": {
                    "spec": "",
                    "impl_notes": null,
                    "reviews": [],
                    "approval": null,
                    "qa_results": [],
                    "pending_qa_feedback": null
                },
                "commit": null,
                "started_at": "2026-01-01T00:00:00Z",
                "completed_at": null
            }],
            "completion_attempts": [],
            "prompt_file": "prompt.md",
            "created_at": "2026-01-01T00:00:00Z"
        });
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&seeded_state).unwrap(),
        )
        .expect("write seeded state");

        let loops_dir = project_dir.join("loops").join("001-quick-dev");
        fs::create_dir_all(&loops_dir).expect("create loop dir");

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
                "--max-final-review-retries",
                "2",
            ])
            .expect("quick-dev-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "force-completed");

        // Verify: no final-review artifacts (both backend calls were skipped)
        let artifacts = h.list_artifacts(project_id, 1).expect("list_artifacts");
        let artifact_names: Vec<String> = artifacts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            !artifact_names
                .iter()
                .any(|n| n.contains("final-review-impl") || n.contains("final-review-rev")),
            "guard-at-entry should skip FinalReview calls; found: {:?}",
            artifact_names
        );

        // Verify force-complete artifact exists
        let fc_path = project_dir.join("quick-dev-force-complete.md");
        assert_file_exists(&fc_path);

        let state = load_state_json(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");
        assert!(state["quick_dev_phase"].is_null());
    })
}

/// Transition-boundary resume: seed state.json at CodexReview (simulating a
/// crash after PlanAndImplement completed and destination state was persisted).
/// Verifies resume starts from CodexReview (not PlanAndImplement), proving
/// destination state was persisted before the checkpoint.
fn transition_boundary_resume_persists_destination(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "qd-transition-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_mock_script(),
            &quick_dev_reviewer_mock_script(),
        );

        // Seed state at CodexReview (as if PlanAndImplement completed and
        // destination state was persisted but checkpoint/process crashed).
        let project_dir = h.project_dir(project_id);
        let state_path = project_dir.join("state.json");
        let seeded_state = serde_json::json!({
            "project_id": project_id,
            "name": "Quick-Dev Test Project",
            "prompt_hash": "seed",
            "current_loop": 1,
            "current_phase": "reviewing",
            "quick_dev_phase": "codex_review",
            "phase_iteration": 1,
            "status": "in_progress",
            "quick_dev_review_iteration": 0,
            "quick_dev_final_review_attempts": 0,
            "loops": [{
                "loop_number": 1,
                "slug": "quick-dev",
                "feature_name": "Quick Dev",
                "loop_type": "Feature",
                "status": "InProgress",
                "backends": {
                    "planner": "claude",
                    "implementer": "claude",
                    "reviewer": "codex",
                    "qa": ""
                },
                "artifacts": {
                    "spec": "",
                    "impl_notes": null,
                    "reviews": [],
                    "approval": null,
                    "qa_results": [],
                    "pending_qa_feedback": null
                },
                "commit": null,
                "started_at": "2026-01-01T00:00:00Z",
                "completed_at": null
            }],
            "completion_attempts": [],
            "prompt_file": "prompt.md",
            "created_at": "2026-01-01T00:00:00Z"
        });
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&seeded_state).unwrap(),
        )
        .expect("write seeded state");

        let loops_dir = project_dir.join("loops").join("001-quick-dev");
        fs::create_dir_all(&loops_dir).expect("create loop dir");

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "completed");

        // Verify: no plan-implement artifact (resume skipped PlanAndImplement)
        let artifacts = h.list_artifacts(project_id, 1).expect("list_artifacts");
        let artifact_names: Vec<String> = artifacts
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            !artifact_names.iter().any(|n| n.contains("plan-implement")),
            "transition-boundary resume should NOT re-run PlanAndImplement; found: {:?}",
            artifact_names
        );

        // First artifact must be codex-review (proving correct resume phase)
        assert!(!artifact_names.is_empty(), "resume must produce artifacts");
        assert!(
            artifact_names[0].contains("codex-review"),
            "first artifact must be codex-review, got: {}",
            artifact_names[0]
        );

        let state = load_state_json(h, project_id);
        assert_eq!(state["status"].as_str().unwrap(), "completed");
    })
}
