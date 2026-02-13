use super::*;

use std::fs;

use crate::validate::assertions::{
    assert_exit_code, assert_file_exists, assert_path_not_exists, assert_stdout_contains,
    parse_yaml_frontmatter,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    auto_mock_script, nested_heading_prompt_review_script, standard_mock_script,
};
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "prompt_review::runs_and_rewrites_prompt",
            func: runs_and_rewrites_prompt,
        },
        ConformanceTest {
            name: "prompt_review::skip_flag_bypasses",
            func: skip_flag_bypasses,
        },
        ConformanceTest {
            name: "prompt_review::auto_skip_flag_bypasses",
            func: auto_skip_flag_bypasses,
        },
        ConformanceTest {
            name: "prompt_review::resume_skips_completed",
            func: resume_skips_completed,
        },
        ConformanceTest {
            name: "prompt_review::disabled_via_config",
            func: disabled_via_config,
        },
        ConformanceTest {
            name: "prompt_review::dry_run_reports_status",
            func: dry_run_reports_status,
        },
        ConformanceTest {
            name: "prompt_review::existing_project_migration",
            func: existing_project_migration,
        },
        ConformanceTest {
            name: "prompt_review::stable_mock_with_modeled_backend",
            func: stable_mock_with_modeled_backend,
        },
        ConformanceTest {
            name: "prompt_review::nested_refined_prompt_headings",
            func: nested_refined_prompt_headings,
        },
    ]
}

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
    h.create_project(
        project_id,
        "Prompt Review Test Project",
        "Prompt review test prompt content",
    )
    .expect("create_project failed");
}

fn runs_and_rewrites_prompt(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-runs";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let project_dir = h.project_dir(project_id);

        // prompt-original.md should exist as a backup.
        let backup_path = project_dir.join("prompt-original.md");
        assert_file_exists(&backup_path);
        let backup = fs::read_to_string(&backup_path).expect("read backup");
        assert!(
            backup.contains("Prompt review test prompt content"),
            "backup should contain original prompt"
        );

        // prompt.md should be rewritten with the refined prompt.
        let prompt_path = project_dir.join("prompt.md");
        let prompt = fs::read_to_string(&prompt_path).expect("read prompt");
        assert!(
            prompt.contains("This is the refined prompt from the mock reviewer"),
            "prompt should contain refined content, got: {prompt}"
        );

        // prompt-review.md artifact should exist with project-scoped frontmatter.
        let review_artifact = project_dir.join("prompt-review.md");
        assert_file_exists(&review_artifact);
        let fm = parse_yaml_frontmatter(&review_artifact);
        assert_eq!(
            fm["artifact"].as_str(),
            Some("prompt-review"),
            "artifact field should be prompt-review"
        );
        assert_eq!(
            fm["project"].as_str(),
            Some(project_id),
            "project field should match"
        );
        assert!(
            fm["role"].as_str().is_some(),
            "role field should be present"
        );
        assert!(
            fm.get("loop").is_none()
                || fm["loop"].is_null()
                || fm["loop"].as_str() == Some(""),
            "project-scoped artifact must not include loop field"
        );

        // State should show prompt_review_completed.
        let state = h.load_state(project_id).expect("load state");
        assert_eq!(
            state["prompt_review_completed"],
            json!(true),
            "prompt_review_completed should be true"
        );
    })
}

fn skip_flag_bypasses(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-skip";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--skip-prompt-review", "--loops", "1"])
            .expect("ralph run --skip-prompt-review --loops 1 should succeed");

        let project_dir = h.project_dir(project_id);

        // No prompt-review.md or prompt-original.md should exist.
        assert_path_not_exists(&project_dir.join("prompt-review.md"));
        assert_path_not_exists(&project_dir.join("prompt-original.md"));

        // State should show prompt_review_completed.
        let state = h.load_state(project_id).expect("load state");
        assert_eq!(
            state["prompt_review_completed"],
            json!(true),
            "prompt_review_completed should be true after skip"
        );
    })
}

fn auto_skip_flag_bypasses(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("auto-mock.sh", &auto_mock_script())
            .expect("failed to write auto mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");

        let output = h
            .ralph_env(
                [
                    "auto",
                    "--idea",
                    "auto skip test",
                    "--skip-prompt-review",
                ],
                &[("RALPH_COMPLETE", "yes")],
            )
            .expect("ralph auto --skip-prompt-review should execute");
        assert_exit_code(&output, 0);

        let project_dir = h.project_dir("auto-skip-test");

        assert_path_not_exists(&project_dir.join("prompt-review.md"));
        assert_path_not_exists(&project_dir.join("prompt-original.md"));
    })
}

fn resume_skips_completed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-resume";
        setup_with_standard_mock(h, project_id);

        // First run: prompt review runs.
        h.ralph_ok(["run", "--loops", "1"])
            .expect("first ralph run should succeed");

        let project_dir = h.project_dir(project_id);
        let review_content =
            fs::read_to_string(project_dir.join("prompt-review.md")).expect("read review");

        // Second run: prompt review should not run again.
        h.ralph_ok(["run", "--loops", "1"])
            .expect("second ralph run should succeed");

        // prompt-review.md should remain the same (not overwritten).
        let review_content_2 =
            fs::read_to_string(project_dir.join("prompt-review.md")).expect("read review 2");
        assert_eq!(
            review_content, review_content_2,
            "prompt-review.md should not be overwritten on resume"
        );

        // prompt-original.md should still exist and not be overwritten.
        assert_file_exists(&project_dir.join("prompt-original.md"));
    })
}

fn disabled_via_config(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-disabled";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("config set should succeed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run should succeed with prompt review disabled");

        let project_dir = h.project_dir(project_id);

        // No prompt review artifacts should be created.
        assert_path_not_exists(&project_dir.join("prompt-review.md"));
        assert_path_not_exists(&project_dir.join("prompt-original.md"));

        // prompt_review_completed should remain false.
        let state = h.load_state(project_id).expect("load state");
        assert_eq!(
            state["prompt_review_completed"],
            json!(false),
            "prompt_review_completed should remain false when disabled"
        );
    })
}

fn dry_run_reports_status(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-dry-run";
        setup_with_standard_mock(h, project_id);

        // For a new project, dry-run should show pending.
        let output = h
            .ralph(["run", "--dry-run"])
            .expect("ralph run --dry-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "prompt_review: pending");

        // Run one loop so prompt review completes.
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run should succeed");

        // Now dry-run should show completed.
        let output2 = h
            .ralph(["run", "--dry-run"])
            .expect("ralph run --dry-run should execute");
        assert_exit_code(&output2, 0);
        assert_stdout_contains(&output2, "prompt_review: completed");
    })
}

fn existing_project_migration(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-migration";
        setup_with_standard_mock(h, project_id);

        // Run one loop to create an existing project with loops.
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        // Manually remove prompt_review_completed from state to simulate legacy.
        let state_path = h.project_dir(project_id).join("state.json");
        let state_json = fs::read_to_string(&state_path).expect("read state");
        let mut state: serde_json::Value =
            serde_json::from_str(&state_json).expect("parse state");
        state["prompt_review_completed"] = json!(false);
        fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap())
            .expect("write state");

        // Remove the prompt-review.md and prompt-original.md so we can detect if
        // prompt review re-ran.
        let project_dir = h.project_dir(project_id);
        let _ = fs::remove_file(project_dir.join("prompt-review.md"));
        let _ = fs::remove_file(project_dir.join("prompt-original.md"));

        // Running again should trigger migration guard (set completed=true) without
        // actually running the reviewer.
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed after migration");

        // prompt-review.md and prompt-original.md should NOT be created by the
        // migration guard, proving the reviewer did not re-run.
        assert_path_not_exists(&project_dir.join("prompt-review.md"));
        assert_path_not_exists(&project_dir.join("prompt-original.md"));

        // The key assertion is that prompt_review_completed is now true in state.
        let state_after = h.load_state(project_id).expect("load state");
        assert_eq!(
            state_after["prompt_review_completed"],
            json!(true),
            "migration should set prompt_review_completed to true"
        );
    })
}

/// Regression test: verify that `setup_mock_backends_stable` works correctly
/// when role backends resolve to modeled specs like `claude(opus)`, which
/// inject `--model` flags. Previously this would produce
/// `bash --model opus script.sh` and fail.
fn stable_mock_with_modeled_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("stable-model-mock.sh", &standard_mock_script())
            .expect("failed to write mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            "pr-stable-model",
            "Stable Model Test",
            "Stable model test prompt",
        )
        .expect("create_project failed");

        // Run a full loop — this invokes backends with modeled specs
        // (e.g. claude(opus) for planner) which inject --model flags.
        // The stable mock setup must handle this without breaking.
        h.ralph_ok(["run", "--skip-prompt-review", "--loops", "1"])
            .expect("ralph run with stable mock and modeled backends should succeed");

        // Verify the loop completed successfully.
        let state = h.load_state("pr-stable-model").expect("load state");
        assert!(
            state["loops"].as_array().map_or(false, |l| !l.is_empty()),
            "at least one loop should have been completed"
        );
    })
}

/// AC14 conformance: verify that a reviewer response with nested `##` headings
/// inside `## Refined Prompt` is fully preserved through the runtime rewrite
/// pipeline (not truncated at the first nested heading).
fn nested_refined_prompt_headings(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "pr-nested-headings";
        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script(
                "nested-heading-mock.sh",
                &nested_heading_prompt_review_script(),
            )
            .expect("failed to write nested heading mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Nested Heading Test",
            "Original prompt for nested heading test",
        )
        .expect("create_project failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let project_dir = h.project_dir(project_id);

        // prompt-original.md should exist as a backup of the original prompt.
        let backup_path = project_dir.join("prompt-original.md");
        assert_file_exists(&backup_path);
        let backup = fs::read_to_string(&backup_path).expect("read backup");
        assert!(
            backup.contains("Original prompt for nested heading test"),
            "backup should contain original prompt"
        );

        // prompt.md should contain the full nested refined content.
        let prompt_path = project_dir.join("prompt.md");
        let prompt = fs::read_to_string(&prompt_path).expect("read rewritten prompt");

        // Verify the nested headings are all present — not truncated at the
        // first `##` inside the refined prompt body.
        assert!(
            prompt.contains("## Overview"),
            "rewritten prompt should contain ## Overview, got: {prompt}"
        );
        assert!(
            prompt.contains("## Architecture"),
            "rewritten prompt should contain ## Architecture, got: {prompt}"
        );
        assert!(
            prompt.contains("### Component Registry"),
            "rewritten prompt should contain ### Component Registry, got: {prompt}"
        );
        assert!(
            prompt.contains("## Acceptance Criteria"),
            "rewritten prompt should contain ## Acceptance Criteria, got: {prompt}"
        );
        assert!(
            prompt.contains("## Technical Notes"),
            "rewritten prompt should contain ## Technical Notes (last section), got: {prompt}"
        );
        assert!(
            prompt.contains("Use the observer pattern for event dispatch."),
            "rewritten prompt should contain final line of the refined prompt, got: {prompt}"
        );

        // prompt-review.md artifact should exist with project-scoped frontmatter.
        let review_artifact = project_dir.join("prompt-review.md");
        assert_file_exists(&review_artifact);
        let fm = parse_yaml_frontmatter(&review_artifact);
        assert_eq!(
            fm["artifact"].as_str(),
            Some("prompt-review"),
            "artifact field should be prompt-review"
        );
        assert_eq!(
            fm["project"].as_str(),
            Some(project_id),
            "project field should match"
        );
        assert!(
            fm["role"].as_str().is_some(),
            "role field should be present"
        );
        // Project-scoped artifact must not include loop-scoped fields.
        assert!(
            fm.get("loop").is_none()
                || fm["loop"].is_null()
                || fm["loop"].as_str() == Some(""),
            "project-scoped artifact must not include loop field"
        );
        assert!(
            fm.get("iteration").is_none() || fm["iteration"].is_null(),
            "project-scoped artifact must not include iteration field"
        );

        // State should show prompt_review_completed.
        let state = h.load_state(project_id).expect("load state");
        assert_eq!(
            state["prompt_review_completed"],
            json!(true),
            "prompt_review_completed should be true"
        );
    })
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
