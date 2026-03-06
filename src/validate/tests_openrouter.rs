use super::*;

use crate::validate::assertions::{assert_exit_code, assert_path_not_exists};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::openrouter_arg_logging_script;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "openrouter::model_injection",
            func: model_injection,
        },
        ConformanceTest {
            name: "openrouter::disabled_default_backend",
            func: disabled_default_backend,
        },
    ]
}

fn model_injection(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");

        let log_path = h.temp_dir.path().join("openrouter-args.log");
        let script = openrouter_arg_logging_script(&log_path);
        let mock_path = h
            .write_mock_script("openrouter-mock.sh", &script)
            .expect("write openrouter mock");
        let mock_str = mock_path.to_string_lossy().into_owned();

        // Configure openrouter backend to use our mock
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "true".to_owned(),
            "--global".to_owned(),
        ])
        .expect("enable openrouter");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.command".to_owned(),
            mock_str,
            "--global".to_owned(),
        ])
        .expect("set openrouter command");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.args".to_owned(),
            "[]".to_owned(),
            "--global".to_owned(),
        ])
        .expect("set openrouter args");

        let output = h
            .ralph_with_stdin(
                ["backend", "exec", "openrouter(test-model)"],
                "test prompt for openrouter",
            )
            .expect("backend exec openrouter(test-model)");
        assert_exit_code(&output, 0);

        // Verify the arg-logging mock captured --model and test-model
        let logged_args =
            std::fs::read_to_string(&log_path).expect("read openrouter arg log");
        assert!(
            logged_args.contains("--model"),
            "expected logged args to contain '--model', got:\n{}",
            logged_args
        );
        assert!(
            logged_args.contains("test-model"),
            "expected logged args to contain 'test-model', got:\n{}",
            logged_args
        );
    })
}

fn disabled_default_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");

        // Set default backend to openrouter
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "workspace.default_backend".to_owned(),
            "openrouter".to_owned(),
            "--global".to_owned(),
        ])
        .expect("set default backend to openrouter");

        // Disable openrouter
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.enabled".to_owned(),
            "false".to_owned(),
            "--global".to_owned(),
        ])
        .expect("disable openrouter");

        // Set up a log path for the openrouter mock to prove it was NOT spawned
        let log_path = h.temp_dir.path().join("openrouter-disabled.log");
        let script = openrouter_arg_logging_script(&log_path);
        let mock_path = h
            .write_mock_script("openrouter-disabled-mock.sh", &script)
            .expect("write openrouter mock for disabled test");
        let mock_str = mock_path.to_string_lossy().into_owned();

        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.openrouter.command".to_owned(),
            mock_str,
            "--global".to_owned(),
        ])
        .expect("set openrouter command");

        // Create a project so `run` has something to operate on
        h.create_project("disabled-or-test", "Disabled OR Test", "Test prompt")
            .expect("create project");

        // Run a command that resolves the default backend through BackendRegistry
        let output = h
            .ralph(["run", "--project", "disabled-or-test", "--loops", "1"])
            .expect("run with disabled default backend");

        assert!(
            !output.status.success(),
            "expected non-zero exit for disabled default backend, got: {:?}",
            output.status
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("unavailable"),
            "expected stderr to contain 'unavailable', got:\n{}",
            stderr
        );

        // Prove openrouter process was never spawned (log file absent)
        assert_path_not_exists(&log_path);
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
