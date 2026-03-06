use super::*;

use std::fs;

use crate::validate::assertions::{assert_exit_code, assert_file_contains, assert_file_exists};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    prd_invocation_counting_script, prd_mock_response_body, prd_stdin_capturing_script,
};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "prd::preset_default_is_full_with_ask_max_3",
            func: preset_default_is_full_with_ask_max_3,
        },
        ConformanceTest {
            name: "prd::preset_overrides_default_ask_max",
            func: preset_overrides_default_ask_max,
        },
        ConformanceTest {
            name: "prd::preset_fast_has_zero_ask_max",
            func: preset_fast_has_zero_ask_max,
        },
        ConformanceTest {
            name: "prd::explicit_ask_max_preempts_preset",
            func: explicit_ask_max_preempts_preset,
        },
        ConformanceTest {
            name: "prd::prd_resume_fewer_invocations",
            func: prd_resume_fewer_invocations,
        },
        ConformanceTest {
            name: "prd::prd_answers_ingested",
            func: prd_answers_ingested,
        },
    ]
}

fn preset_default_is_full_with_ask_max_3(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph(["prd", "--idea", "Add one-click onboarding"])
            .expect("prd default preset command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: full"),
            "expected full preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 3"),
            "expected default ask max 3"
        );

        let prd = h.repo_root.join("PRD.md");
        assert_file_exists(&prd);
        assert_file_contains(&prd, "## Executive Summary");
    })
}

fn preset_overrides_default_ask_max(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph(["prd", "--idea", "Add quick notes", "--preset", "discuss"])
            .expect("prd discuss preset command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: discuss"),
            "expected discuss preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 1"),
            "expected discuss preset to resolve to ask max 1"
        );
    })
}

fn preset_fast_has_zero_ask_max(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph(["prd", "--idea", "Add quick exports", "--preset", "fast"])
            .expect("prd fast preset command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: fast"),
            "expected fast preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 0"),
            "expected fast preset to resolve to ask max 0"
        );
    })
}

fn explicit_ask_max_preempts_preset(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph([
                "prd",
                "--idea",
                "Add bulk edits",
                "--preset",
                "fast",
                "--ask-max",
                "2",
            ])
            .expect("prd preset+ask-max command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: fast"),
            "expected fast preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 2"),
            "expected explicit ask max to override preset"
        );
    })
}

fn prd_resume_fewer_invocations(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init_workspace failed");

        let counter_path = h.temp_dir.path().join("prd-invocation-counter.txt");
        let script = prd_invocation_counting_script(&counter_path);
        let script_path = h
            .write_stable_mock_script("prd-counting-mock.sh", &script)
            .expect("failed to write counting mock script");
        h.setup_mock_backends_stable(&script_path)
            .expect("setup_mock_backends_stable failed");

        // First run: fresh PRD generation
        let output = h
            .ralph(["prd", "--idea", "Build a task scheduler"])
            .expect("prd first run should execute");
        assert_exit_code(&output, 0);

        let first_count: u64 = fs::read_to_string(&counter_path)
            .expect("counter file should exist after first run")
            .trim()
            .parse()
            .expect("counter should be a number");
        assert!(
            first_count > 0,
            "expected at least one invocation in first run"
        );

        // Reset counter for second run
        fs::write(&counter_path, "0").expect("failed to reset counter");

        // Second run: resume should reuse cached stages
        let output = h
            .ralph(["prd", "--idea", "Build a task scheduler", "--resume"])
            .expect("prd resume run should execute");
        assert_exit_code(&output, 0);

        let second_count: u64 = fs::read_to_string(&counter_path)
            .expect("counter file should exist after resume run")
            .trim()
            .parse()
            .expect("counter should be a number");

        assert!(
            second_count < first_count,
            "expected resume run ({second_count}) to invoke backend fewer times than first run ({first_count})"
        );
    })
}

fn prd_answers_ingested(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init_workspace failed");

        let capture_dir = h.temp_dir.path().join("prd-stdin-captures");
        let script = prd_stdin_capturing_script(&capture_dir);
        let script_path = h
            .write_stable_mock_script("prd-capture-mock.sh", &script)
            .expect("failed to write stdin-capturing mock script");
        h.setup_mock_backends_stable(&script_path)
            .expect("setup_mock_backends_stable failed");

        // Write answers YAML as a flat map (key: value) matching AnswerStore's
        // BTreeMap<String, String> deserialization format.
        let sentinel = "SENTINEL_ANSWER_VALUE_12345";
        let answers_path = h.temp_dir.path().join("test-answers.yaml");
        fs::write(
            &answers_path,
            format!("What is the target audience?: {sentinel}\n"),
        )
        .expect("failed to write answers file");

        let output = h
            .ralph([
                "prd",
                "--idea",
                "Build a notification service",
                "--answers",
                &answers_path.to_string_lossy(),
            ])
            .expect("prd with answers should execute");
        assert_exit_code(&output, 0);

        // Check that at least one captured stdin file contains the sentinel
        let entries: Vec<_> = fs::read_dir(&capture_dir)
            .expect("capture dir should exist")
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            !entries.is_empty(),
            "expected at least one captured stdin file"
        );

        let found_sentinel = entries.iter().any(|entry| {
            fs::read_to_string(entry.path())
                .unwrap_or_default()
                .contains(sentinel)
        });
        assert!(
            found_sentinel,
            "expected sentinel '{sentinel}' in at least one captured stdin file"
        );
    })
}

fn setup_prd_mock(h: &RalphHarness) {
    h.init_workspace().expect("init_workspace failed");
    let script = h
        .write_mock_script("prd-mock.sh", &prd_mock_script())
        .expect("failed to write PRD mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends failed");
}

fn prd_mock_script() -> String {
    let response_body = prd_mock_response_body();
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"
{response_body}
"###
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
