use super::*;

use std::fs;
use std::process::Command;

use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{e2e_mock_gh_logging_script, e2e_mock_ralph_script};
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "e2e_conformance::backend_timeout_exhausted_fails_task",
            func: backend_timeout_exhausted_fails_task,
        },
        ConformanceTest {
            name: "e2e_conformance::e2e_mock_ralph_script_delegates_to_auto",
            func: e2e_mock_ralph_script_delegates_to_auto,
        },
        ConformanceTest {
            name: "e2e_conformance::e2e_mock_gh_logging_script_captures_pr_create",
            func: e2e_mock_gh_logging_script_captures_pr_create,
        },
    ]
}

fn backend_timeout_exhausted_fails_task(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "e2e-timeout";
        h.init_workspace().expect("init failed");

        let script = h
            .write_mock_script("sleep-timeout.sh", &sleeping_backend_script())
            .expect("failed to write sleeping backend script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");

        // Backend timeout settings are global config keys.
        h.ralph_ok(["config", "set", "backends.claude.timeout_seconds", "2"])
            .expect("config set backends.claude.timeout_seconds failed");
        h.ralph_ok(["config", "set", "backends.codex.timeout_seconds", "2"])
            .expect("config set backends.codex.timeout_seconds failed");

        h.create_project(
            project_id,
            "E2E Timeout Project",
            "Backend timeout test prompt",
        )
        .expect("create_project failed");
        // Keep the test focused on orchestration timeout handling during planning.
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("config set workflow.prompt_review_enabled failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");

        let exit_code = output.status.code().unwrap_or(-1);
        assert_ne!(
            exit_code, 0,
            "expected non-zero exit when backend times out"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("BackendTimeoutExhausted"),
            "expected timeout exhaustion to be surfaced in stderr, got:\n{stderr}"
        );
        assert!(
            !stderr.contains("requesting reformat via"),
            "timeout should not trigger reformatter fallback, got:\n{stderr}"
        );

        let state = h.load_state(project_id).expect("load_state failed");
        assert_eq!(
            state["status"],
            json!("failed"),
            "project should be marked failed after backend timeout"
        );
    })
}

fn e2e_mock_ralph_script_delegates_to_auto(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let script = h
            .write_mock_script("e2e-mock-ralph.sh", &e2e_mock_ralph_script(&h.ralph_bin))
            .expect("failed to write e2e mock ralph script");
        let script_content = fs::read_to_string(&script).expect("failed to read mock ralph script");

        let expected_bin = h
            .ralph_bin
            .canonicalize()
            .unwrap_or_else(|_| h.ralph_bin.clone());
        let expected_bin_str = expected_bin.to_string_lossy();

        assert!(
            script_content.contains(&*expected_bin_str),
            "script should embed the absolute ralph binary path"
        );
        assert!(
            script_content.contains(" auto \"$@\""),
            "script should execute ralph auto with forwarded args"
        );
        assert!(
            !script_content.contains("exec ralph "),
            "script should not resolve ralph via PATH"
        );

        let output = Command::new(&script)
            .arg("--help")
            .output()
            .expect("mock ralph script should execute");
        assert!(
            output.status.success(),
            "mock ralph script should delegate successfully; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    })
}

fn e2e_mock_gh_logging_script_captures_pr_create(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let script = h
            .write_mock_script("e2e-mock-gh.sh", &e2e_mock_gh_logging_script())
            .expect("failed to write e2e mock gh script");

        let body_path = h.temp_dir.path().join("pr-body.md");
        fs::write(
            &body_path,
            "Closes #42\n\nDiff stat:\n- src/lib.rs | 2 +-\n\nProject: acme/widgets\n",
        )
        .expect("failed to write body file");
        let body_file = body_path.to_string_lossy().into_owned();
        let log_path = h.temp_dir.path().join("gh-pr-create.log");

        let output = Command::new(&script)
            .args([
                "pr",
                "create",
                "--title",
                "ralph: test PR title",
                "--body-file",
                &body_file,
                "--head",
                "ralph/test-branch",
                "--repo",
                "acme/widgets",
            ])
            .env("RALPH_E2E_GH_LOG", &log_path)
            .output()
            .expect("mock gh script should execute");
        assert!(
            output.status.success(),
            "mock gh script should succeed; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("https://github.com/mock/repo/pull/123"),
            "mock gh script should return a synthetic PR URL"
        );

        let log_content = fs::read_to_string(&log_path).expect("failed to read gh log");
        assert!(
            log_content.contains("--title") && log_content.contains("ralph: test PR title"),
            "log should capture full --title args, got:\n{log_content}"
        );
        assert!(
            log_content.contains("--head") && log_content.contains("ralph/test-branch"),
            "log should capture full --head args, got:\n{log_content}"
        );
        assert!(
            log_content.contains("--repo") && log_content.contains("acme/widgets"),
            "log should capture full --repo args, got:\n{log_content}"
        );
        assert!(
            log_content.contains("body_begin")
                && log_content.contains("Closes #42")
                && log_content.contains("Diff stat:"),
            "log should capture --body-file content, got:\n{log_content}"
        );
    })
}

fn sleeping_backend_script() -> String {
    r#"#!/bin/sh
set -eu

# Consume prompt input, then sleep long enough to exceed backend timeout.
cat >/dev/null
sleep 30
echo "unreachable"
"#
    .to_owned()
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
