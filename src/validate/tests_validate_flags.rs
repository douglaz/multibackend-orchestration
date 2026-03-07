use super::*;

use crate::validate::assertions::{assert_exit_code, assert_stdout_contains};
use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "validate_flags::list_prints_names",
            func: list_prints_names,
        },
        ConformanceTest {
            name: "validate_flags::filter_nonexistent_zero",
            func: filter_nonexistent_zero,
        },
        ConformanceTest {
            name: "validate_flags::single_job_filter",
            func: single_job_filter,
        },
    ]
}

fn list_prints_names(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let bin = h.ralph_bin.to_string_lossy().into_owned();
        let output = h
            .ralph(vec![
                "validate".to_owned(),
                "--bin".to_owned(),
                bin,
                "--list".to_owned(),
            ])
            .expect("ralph validate --list should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "run::single_feature_loop");
    })
}

fn filter_nonexistent_zero(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let bin = h.ralph_bin.to_string_lossy().into_owned();
        let output = h
            .ralph(vec![
                "validate".to_owned(),
                "--bin".to_owned(),
                bin,
                "--filter".to_owned(),
                "nonexistent_prefix_zzz".to_owned(),
            ])
            .expect("ralph validate --filter nonexistent should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "running 0 tests");
    })
}

fn single_job_filter(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let bin = h.ralph_bin.to_string_lossy().into_owned();
        let output = h
            .ralph(vec![
                "validate".to_owned(),
                "--bin".to_owned(),
                bin,
                "-j".to_owned(),
                "1".to_owned(),
                "--filter".to_owned(),
                "run::single_feature_loop".to_owned(),
            ])
            .expect("ralph validate -j 1 --filter should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "running 1 tests");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("jobs: 1") || stdout.contains("jobs=1"),
            "expected validate output to mention jobs=1, got:\n{stdout}"
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
