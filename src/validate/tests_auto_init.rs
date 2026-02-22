use std::fs;

use super::*;

use crate::validate::assertions::{assert_dir_exists, assert_exit_code, assert_file_exists};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::auto_mock_script;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "auto_init::auto_initializes_workspace_when_missing",
            func: auto_initializes_workspace_when_missing,
        },
        ConformanceTest {
            name: "auto_init::auto_init_prints_stderr_notice",
            func: auto_init_prints_stderr_notice,
        },
        ConformanceTest {
            name: "auto_init::auto_does_not_change_other_commands_workspace_not_found_behavior",
            func: auto_does_not_change_other_commands_workspace_not_found_behavior,
        },
        ConformanceTest {
            name: "auto_init::auto_on_existing_workspace_with_missing_ralph_toml_reinitializes",
            func: auto_on_existing_workspace_with_missing_ralph_toml_reinitializes,
        },
        ConformanceTest {
            name: "auto_init::init_behavior_unchanged_for_non_empty_target",
            func: init_behavior_unchanged_for_non_empty_target,
        },
    ]
}

fn auto_initializes_workspace_when_missing(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let path_env = setup_auto_backend_path(h);
        let output = h
            .ralph_env(
                ["auto", "--idea", "auto init dry-run", "--dry-run"],
                &[("PATH", path_env.as_str())],
            )
            .expect("ralph auto --dry-run should execute");
        assert_exit_code(&output, 0);

        let workspace_root = h.repo_root.join(".ralph");
        assert_dir_exists(&workspace_root);
        assert_file_exists(&workspace_root.join("ralph.toml"));
        assert_dir_exists(&workspace_root.join("projects"));
        assert_dir_exists(&workspace_root.join("templates"));
        assert_file_exists(&workspace_root.join("templates/spec.md"));
        assert_file_exists(&workspace_root.join("templates/implementation.md"));
        assert_file_exists(&workspace_root.join("templates/review.md"));
        assert_file_exists(&workspace_root.join("templates/prompt_reviewer.md"));
        assert_file_exists(&workspace_root.join("templates/prompt_review_validator.md"));
        assert_file_exists(&workspace_root.join("templates/completion.md"));
        assert_file_exists(&workspace_root.join("templates/qa.md"));
        assert_file_exists(&workspace_root.join("templates/final_reviewer.md"));
        assert_file_exists(&workspace_root.join("templates/planner_position.md"));
        assert_file_exists(&workspace_root.join("templates/vote.md"));
        assert_file_exists(&workspace_root.join("templates/arbiter.md"));
    })
}

fn auto_init_prints_stderr_notice(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let path_env = setup_auto_backend_path(h);
        let output = h
            .ralph_env(
                ["auto", "--idea", "auto init stderr", "--dry-run"],
                &[("PATH", path_env.as_str())],
            )
            .expect("ralph auto --dry-run should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        let notice_count = stderr
            .lines()
            .filter(|line| *line == "initialized workspace at .ralph")
            .count();
        assert_eq!(
            notice_count, 1,
            "expected exactly one auto-init notice in stderr, got:\n{stderr}"
        );
    })
}

fn auto_does_not_change_other_commands_workspace_not_found_behavior(
    h: &RalphHarness,
) -> TestResult {
    run_case(|| {
        let output = h
            .ralph(["run", "--dry-run"])
            .expect("ralph run --dry-run should execute");
        assert_exit_code(&output, 2);

        assert!(
            !h.repo_root.join(".ralph").exists(),
            "non-auto commands should not auto-create .ralph"
        );

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .to_lowercase();
        assert!(
            combined.contains("workspace not found"),
            "expected workspace-not-found error, got:\n{}",
            combined
        );
    })
}

fn auto_on_existing_workspace_with_missing_ralph_toml_reinitializes(
    h: &RalphHarness,
) -> TestResult {
    run_case(|| {
        let workspace_root = h.repo_root.join(".ralph");
        fs::create_dir_all(&workspace_root).expect("create .ralph dir");

        let path_env = setup_auto_backend_path(h);
        let output = h
            .ralph_env(
                ["auto", "--idea", "missing config", "--dry-run"],
                &[("PATH", path_env.as_str())],
            )
            .expect("ralph auto --dry-run should execute");
        // Discovery skips .ralph/ without ralph.toml, so ensure_workspace
        // treats it as WorkspaceNotFound and reinitializes the workspace.
        assert_exit_code(&output, 0);
        assert_file_exists(&workspace_root.join("ralph.toml"));

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("initialized workspace at .ralph"),
            "auto-init notice should be printed when .ralph exists but ralph.toml is missing, got:\n{stderr}"
        );
    })
}

fn init_behavior_unchanged_for_non_empty_target(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("first init should succeed");

        let output = h.ralph(["init"]).expect("second init should execute");
        assert_exit_code(&output, 2);

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("already exists") && combined.contains("not empty"),
            "expected non-empty init validation error, got:\n{combined}"
        );
    })
}

fn setup_auto_backend_path(h: &RalphHarness) -> String {
    let mock_script = h
        .write_mock_script("auto-mock.sh", &auto_mock_script())
        .expect("write auto mock script");

    let wrapper = format!("#!/bin/sh\nexec bash \"{}\"\n", mock_script.display());
    let bin_dir = h.temp_dir.path().join("bin");
    h.write_mock_script("bin/claude", &wrapper)
        .expect("write mock claude");
    h.write_mock_script("bin/codex", &wrapper)
        .expect("write mock codex");

    format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    )
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
