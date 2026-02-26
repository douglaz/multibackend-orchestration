use super::*;

use std::process::Output;

use crate::validate::assertions::{assert_exit_code, assert_json_field};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{auto_mock_script, daemon_mock_gh_script, standard_mock_script};
use crate::workspace::Workspace;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "gemini_backend::optional_backend_skips_when_unavailable",
            func: optional_backend_skips_when_unavailable,
        },
        ConformanceTest {
            name: "gemini_backend::required_backend_fails_when_unavailable",
            func: required_backend_fails_when_unavailable,
        },
        ConformanceTest {
            name: "gemini_backend::guardrails_reject_disallowed_surfaces",
            func: guardrails_reject_disallowed_surfaces,
        },
        ConformanceTest {
            name: "gemini_backend::daemon_refinement_guardrail_rejects_project_override",
            func: daemon_refinement_guardrail_rejects_project_override,
        },
        ConformanceTest {
            name: "gemini_backend::quick_prd_reviewer_gemini_success",
            func: quick_prd_reviewer_gemini_success,
        },
        ConformanceTest {
            name: "gemini_backend::quick_prd_writer_gemini_success",
            func: quick_prd_writer_gemini_success,
        },
        ConformanceTest {
            name: "gemini_backend::quick_prd_reviewer_gemini_disabled_fails",
            func: quick_prd_reviewer_gemini_disabled_fails,
        },
        ConformanceTest {
            name: "gemini_backend::quick_prd_writer_gemini_disabled_fails",
            func: quick_prd_writer_gemini_disabled_fails,
        },
        ConformanceTest {
            name: "gemini_backend::quick_prd_reviewer_optional_unavailable_fails",
            func: quick_prd_reviewer_optional_unavailable_fails,
        },
        ConformanceTest {
            name: "gemini_backend::quick_prd_writer_optional_unavailable_fails",
            func: quick_prd_writer_optional_unavailable_fails,
        },
        ConformanceTest {
            name: "gemini_backend::auto_spec_reviewer_gemini_success",
            func: auto_spec_reviewer_gemini_success,
        },
        ConformanceTest {
            name: "gemini_backend::auto_spec_writer_gemini_success",
            func: auto_spec_writer_gemini_success,
        },
        ConformanceTest {
            name: "gemini_backend::auto_spec_reviewer_gemini_disabled_fails",
            func: auto_spec_reviewer_gemini_disabled_fails,
        },
        ConformanceTest {
            name: "gemini_backend::auto_spec_writer_gemini_disabled_fails",
            func: auto_spec_writer_gemini_disabled_fails,
        },
        ConformanceTest {
            name: "gemini_backend::auto_spec_reviewer_optional_unavailable_fails",
            func: auto_spec_reviewer_optional_unavailable_fails,
        },
        ConformanceTest {
            name: "gemini_backend::auto_spec_writer_optional_unavailable_fails",
            func: auto_spec_writer_optional_unavailable_fails,
        },
        ConformanceTest {
            name: "gemini_backend::daemon_prd_guardrail_rejects_optional_gemini",
            func: daemon_prd_guardrail_rejects_optional_gemini,
        },
    ]
}

fn optional_backend_skips_when_unavailable(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "gemini-optional-skip";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("gemini-optional-standard.sh", &standard_mock_script())
            .expect("failed to write standard mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup mock backends failed");
        h.create_project(
            project_id,
            "Gemini Optional Skip",
            "Validate optional gemini backend skip",
        )
        .expect("create project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_min_reviewers", "2"])
            .expect("set final_review_min_reviewers failed");
        h.ralph_ok([
            "config",
            "set",
            "workflow.final_review_backends",
            "[\"claude\",\"codex\",\"?gemini\"]",
        ])
        .expect("set final_review_backends failed");
        h.ralph_ok([
            "config",
            "set",
            "backends.gemini.enabled",
            "false",
            "--global",
        ])
        .expect("disable gemini backend failed");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load state failed");
        assert_json_field(&state, "status", &json!("completed"));
    })
}

fn required_backend_fails_when_unavailable(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "gemini-required-fails";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("gemini-required-standard.sh", &standard_mock_script())
            .expect("failed to write standard mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup mock backends failed");
        h.create_project(
            project_id,
            "Gemini Required Failure",
            "Validate required gemini backend failure",
        )
        .expect("create project failed");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review failed");
        h.ralph_ok(["config", "set", "workflow.final_review_min_reviewers", "1"])
            .expect("set final_review_min_reviewers failed");
        h.ralph_ok([
            "config",
            "set",
            "workflow.final_review_backends",
            "[\"gemini\"]",
        ])
        .expect("set final_review_backends failed");
        h.ralph_ok([
            "config",
            "set",
            "backends.gemini.enabled",
            "false",
            "--global",
        ])
        .expect("disable gemini backend failed");

        let output = h
            .ralph_env(["run", "--until-complete"], &[("RALPH_COMPLETE", "yes")])
            .expect("run should execute");
        assert!(
            output.status.code().unwrap_or(-1) != 0,
            "required unavailable backend should fail"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .to_lowercase();
        assert!(
            combined.contains("gemini"),
            "failure output should mention gemini, got:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    })
}

fn guardrails_reject_disallowed_surfaces(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "gemini-guardrails";
        h.init_workspace().expect("init failed");
        h.create_project(
            project_id,
            "Gemini Guardrails",
            "Validate gemini guardrail validation",
        )
        .expect("create project failed");

        h.ralph_ok(["config", "set", "workflow.starting_backend", "gemini"])
            .expect("set starting backend failed");
        let output = h
            .ralph(["run", "--dry-run"])
            .expect("dry-run should execute");
        assert!(
            output.status.code().unwrap_or(-1) != 0,
            "gemini starting backend should be rejected"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("gemini backend is not supported for starting backend"),
            "expected starting-backend guardrail message, got:\n{combined}"
        );

        h.ralph_ok(["config", "set", "workflow.starting_backend", "claude"])
            .expect("reset starting backend failed");
        h.ralph_ok(["config", "set", "workflow.planner_backend", "?gemini"])
            .expect("set planner backend optional syntax failed");
        let output = h
            .ralph(["run", "--dry-run"])
            .expect("dry-run should execute");
        assert!(
            output.status.code().unwrap_or(-1) != 0,
            "optional syntax on required surface should be rejected"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains(
                "optional backend specs (?backend) are not supported for planner backend override"
            ),
            "expected optional syntax rejection message, got:\n{combined}"
        );
    })
}

fn daemon_refinement_guardrail_rejects_project_override(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets")
            .expect("create daemon harness");
        dh.init_workspace().expect("init failed");
        dh.create_project(
            "daemon-guardrail",
            "Daemon Guardrail Project",
            "Project used for daemon guardrail validation",
        )
        .expect("create project failed");

        dh.ralph_ok(["project", "use", "daemon-guardrail"])
            .expect("set active project");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.refinement_backend",
            "gemini(gemini-3-pro-preview)",
            "--project",
            "daemon-guardrail",
        ])
        .expect("set project daemon refinement backend");

        let gh = dh
            .write_mock_script("gh", &daemon_mock_gh_script())
            .expect("write mock gh");
        let gh_path = format!(
            "{}:{}",
            gh.parent()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            std::env::var("PATH").unwrap_or_default()
        );

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--single-iteration",
                    "--repo",
                    "acme/widgets",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_nonzero_exit(
            &output,
            "daemon start should reject gemini refinement backend",
        );

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("invalid daemon config for acme/widgets"),
            "expected daemon config validation failure, got:\n{combined}"
        );
        assert!(
            combined.contains("daemon.refinement_backend"),
            "expected effective daemon key in error, got:\n{combined}"
        );
        assert!(
            combined.contains("gemini backend is not supported"),
            "expected gemini guardrail message, got:\n{combined}"
        );
    })
}

fn quick_prd_reviewer_gemini_success(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        let output = h
            .ralph([
                "quick-prd",
                "--idea",
                "quick-prd reviewer gemini",
                "--reviewer-backend",
                "gemini",
            ])
            .expect("quick-prd command should execute");
        assert_exit_code(&output, 0);
    })
}

fn quick_prd_writer_gemini_success(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        let output = h
            .ralph([
                "quick-prd",
                "--idea",
                "quick-prd writer gemini",
                "--writer-backend",
                "gemini",
            ])
            .expect("quick-prd command should execute");
        assert_exit_code(&output, 0);
    })
}

fn quick_prd_reviewer_gemini_disabled_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "quick-prd",
                "--idea",
                "quick-prd reviewer disabled",
                "--reviewer-backend",
                "gemini",
            ])
            .expect("quick-prd command should execute");
        assert_nonzero_exit(&output, "disabled gemini reviewer should fail");
    })
}

fn quick_prd_writer_gemini_disabled_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "quick-prd",
                "--idea",
                "quick-prd writer disabled",
                "--writer-backend",
                "gemini",
            ])
            .expect("quick-prd command should execute");
        assert_nonzero_exit(&output, "disabled gemini writer should fail");
    })
}

fn quick_prd_reviewer_optional_unavailable_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "quick-prd",
                "--idea",
                "quick-prd reviewer optional",
                "--reviewer-backend",
                "?gemini",
            ])
            .expect("quick-prd command should execute");
        assert_nonzero_exit(&output, "optional unavailable reviewer backend should fail");
    })
}

fn quick_prd_writer_optional_unavailable_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "quick-prd",
                "--idea",
                "quick-prd writer optional",
                "--writer-backend",
                "?gemini",
            ])
            .expect("quick-prd command should execute");
        assert_nonzero_exit(&output, "optional unavailable writer backend should fail");
    })
}

fn auto_spec_reviewer_gemini_success(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        let output = h
            .ralph([
                "auto",
                "--idea",
                "auto reviewer gemini",
                "--spec-reviewer",
                "gemini",
                "--dry-run",
            ])
            .expect("auto command should execute");
        assert_exit_code(&output, 0);
    })
}

fn auto_spec_writer_gemini_success(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        let output = h
            .ralph([
                "auto",
                "--idea",
                "auto writer gemini",
                "--spec-writer",
                "gemini",
                "--dry-run",
            ])
            .expect("auto command should execute");
        assert_exit_code(&output, 0);
    })
}

fn auto_spec_reviewer_gemini_disabled_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "auto",
                "--idea",
                "auto reviewer disabled",
                "--spec-reviewer",
                "gemini",
                "--dry-run",
            ])
            .expect("auto command should execute");
        assert_nonzero_exit(&output, "disabled auto spec reviewer should fail");
    })
}

fn auto_spec_writer_gemini_disabled_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "auto",
                "--idea",
                "auto writer disabled",
                "--spec-writer",
                "gemini",
                "--dry-run",
            ])
            .expect("auto command should execute");
        assert_nonzero_exit(&output, "disabled auto spec writer should fail");
    })
}

fn auto_spec_reviewer_optional_unavailable_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "auto",
                "--idea",
                "auto reviewer optional",
                "--spec-reviewer",
                "?gemini",
                "--dry-run",
            ])
            .expect("auto command should execute");
        assert_nonzero_exit(&output, "optional unavailable auto reviewer should fail");
    })
}

fn auto_spec_writer_optional_unavailable_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_auto_mock_with_gemini(h);
        disable_gemini_backend(h);

        let output = h
            .ralph([
                "auto",
                "--idea",
                "auto writer optional",
                "--spec-writer",
                "?gemini",
                "--dry-run",
            ])
            .expect("auto command should execute");
        assert_nonzero_exit(&output, "optional unavailable auto writer should fail");
    })
}

fn daemon_prd_guardrail_rejects_optional_gemini(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets")
            .expect("create daemon harness");
        dh.init_workspace().expect("init failed");

        let workspace_root = dh.repo_root.join(".ralph");
        let mut workspace = Workspace::load(workspace_root).expect("load workspace");
        workspace.config.workspace.daemon_prd_reviewer_backend = "?gemini".to_owned();
        workspace
            .save_config()
            .expect("save daemon PRD config mutation");

        let gh = dh
            .write_mock_script("gh", &daemon_mock_gh_script())
            .expect("write mock gh");
        let gh_path = format!(
            "{}:{}",
            gh.parent()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            std::env::var("PATH").unwrap_or_default()
        );

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--single-iteration",
                    "--repo",
                    "acme/widgets",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_nonzero_exit(
            &output,
            "daemon start should reject optional gemini on PRD surface",
        );

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("workspace.daemon_prd_reviewer_backend"),
            "expected daemon PRD reviewer key in error, got:\n{combined}"
        );
        assert!(
            combined.contains("optional backend specs (?backend)"),
            "expected optional backend syntax rejection, got:\n{combined}"
        );
    })
}

fn setup_auto_mock_with_gemini(h: &RalphHarness) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("gemini-prd-auto-mock.sh", &auto_mock_script())
        .expect("write auto mock script");
    h.setup_mock_backends_with_gemini(&script)
        .expect("setup gemini mock backends");
}

fn disable_gemini_backend(h: &RalphHarness) {
    h.ralph_ok([
        "config",
        "set",
        "backends.gemini.enabled",
        "false",
        "--global",
    ])
    .expect("disable gemini backend failed");
}

fn assert_nonzero_exit(output: &Output, message: &str) {
    assert!(
        output.status.code().unwrap_or(-1) != 0,
        "{message}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    assert!(
        combined.contains("gemini")
            || combined.contains("optional backend specs (?backend)")
            || combined.contains("backend unavailable"),
        "expected output to mention gemini/backend unavailability, got:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
