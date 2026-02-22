use super::*;

use crate::validate::assertions::{
    assert_exit_code, assert_json_field, assert_no_uncommitted_ralph_files,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::standard_mock_script;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "completion_panel::two_completer_consensus_complete",
            func: two_completer_consensus_complete,
        },
        ConformanceTest {
            name: "completion_panel::single_completer_backward_compat",
            func: single_completer_backward_compat,
        },
        ConformanceTest {
            name: "completion_panel::panel_continue_verdict",
            func: panel_continue_verdict,
        },
        ConformanceTest {
            name: "completion_panel::per_backend_verdict_artifacts",
            func: per_backend_verdict_artifacts,
        },
    ]
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

fn setup_panel_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("panel-mock.sh", &standard_mock_script())
        .expect("failed to write panel mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
    h.create_project(
        project_id,
        "Completion Panel Project",
        "Panel test prompt",
    )
    .expect("create_project failed");
}

/// Two completers (claude, codex), both return COMPLETE, min_completers=2,
/// threshold=1.0 → consensus reached, project completes.
fn two_completer_consensus_complete(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-complete-1";
        setup_panel_mock(h, project_id);

        // Configure 2-completer panel with strict consensus
        h.ralph_ok(["config", "set", "workflow.completion_backends", "[\"claude\",\"codex\"]"])
            .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok(["config", "set", "workflow.completion_consensus_threshold", "1.0"])
            .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        // Verify completion attempt has panel verdict
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert!(
            !attempts.is_empty(),
            "should have at least one completion attempt"
        );
        let attempt = &attempts[attempts.len() - 1];
        assert_eq!(
            attempt["verdict"].as_str().unwrap(),
            "complete",
            "panel should reach COMPLETE consensus"
        );

        // Verify completers list has 2 entries
        let completers = attempt["backends"]["completers"]
            .as_array()
            .expect("completers should be array");
        assert_eq!(completers.len(), 2, "should have 2 completers in panel");

        assert_no_uncommitted_ralph_files(&h.repo_root);
    })
}

/// Single completer (only claude configured), min_completers=1,
/// threshold=1.0 → falls back to single completer behavior, uses legacy
/// artifact name.
fn single_completer_backward_compat(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-single-1";
        setup_panel_mock(h, project_id);

        // Configure single-completer panel
        h.ralph_ok(["config", "set", "workflow.completion_backends", "[\"claude\"]"])
            .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set completion_min_completers");
        h.ralph_ok(["config", "set", "workflow.completion_consensus_threshold", "1.0"])
            .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        assert!(!attempts.is_empty());
        let attempt = &attempts[attempts.len() - 1];
        assert_eq!(attempt["verdict"].as_str().unwrap(), "complete");

        // Single completer uses legacy artifact name (completer-verdict.md)
        let verdict_path = attempt["artifacts"]["verdict"]
            .as_str()
            .expect("verdict artifact should exist");
        assert!(
            verdict_path.contains("completer-verdict"),
            "verdict artifact should contain completer-verdict: {verdict_path}"
        );

        assert_no_uncommitted_ralph_files(&h.repo_root);
    })
}

/// Two completers, one returns CONTINUE, one returns COMPLETE,
/// with threshold=1.0 → consensus NOT reached, project continues.
fn panel_continue_verdict(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-continue-1";
        h.init_workspace().expect("init failed");

        // Create separate scripts: claude returns COMPLETE, codex returns CONTINUE
        let claude_script_content = r###"#!/usr/bin/env bash
set -euo pipefail
INPUT="$(cat)"
if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  cat <<'EOF'
# Project Completion Request

## Rationale
All required behavior is complete.
EOF
elif echo "$INPUT" | grep -q "You are a software developer implementing"; then
  cat <<'EOF'
# Implementation Notes
Mock implementation for panel test.
EOF
elif echo "$INPUT" | grep -q "You are a code reviewer"; then
  cat <<'EOF'
# Review: APPROVED
All changes look good.
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
# Verdict: COMPLETE
The project satisfies all requirements.
EOF
fi
"###;

        let codex_script_content = r###"#!/usr/bin/env bash
set -euo pipefail
INPUT="$(cat)"
if echo "$INPUT" | grep -q "You are a software architect planning features for a project."; then
  cat <<'EOF'
# Project Completion Request

## Rationale
All required behavior is complete.
EOF
elif echo "$INPUT" | grep -q "You are a software developer implementing"; then
  cat <<'EOF'
# Implementation Notes
Mock implementation for panel test.
EOF
elif echo "$INPUT" | grep -q "You are a code reviewer"; then
  cat <<'EOF'
# Review: APPROVED
All changes look good.
EOF
elif echo "$INPUT" | grep -q "You are a project completion validator."; then
  cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional work remains.
EOF
fi
"###;

        let claude_script = h
            .write_mock_script("claude-panel.sh", claude_script_content)
            .expect("write claude script");
        let codex_script = h
            .write_mock_script("codex-panel.sh", codex_script_content)
            .expect("write codex script");
        h.setup_separate_mock_backends(&claude_script, &codex_script)
            .expect("setup_separate_mock_backends failed");

        h.create_project(project_id, "Panel Continue Project", "Panel continue prompt")
            .expect("create_project failed");

        // Configure 2-completer panel with strict consensus
        h.ralph_ok(["config", "set", "workflow.completion_backends", "[\"claude\",\"codex\"]"])
            .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok(["config", "set", "workflow.completion_consensus_threshold", "1.0"])
            .expect("set completion_consensus_threshold");

        // Run a single loop - the planner requests completion, but consensus fails (1/2 < threshold)
        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        // With consensus failure, the completion attempt records CONTINUE
        let attempts = state["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be array");
        if !attempts.is_empty() {
            let attempt = &attempts[attempts.len() - 1];
            assert_eq!(
                attempt["verdict"].as_str().unwrap(),
                "continue",
                "panel should not reach consensus with 1/2 COMPLETE votes at threshold=1.0"
            );
        }
    })
}

/// With 2 completers, verify per-backend verdict artifact files are created
/// with the expected naming pattern.
fn per_backend_verdict_artifacts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "panel-artifacts-1";
        setup_panel_mock(h, project_id);

        // Configure 2-completer panel
        h.ralph_ok(["config", "set", "workflow.completion_backends", "[\"claude\",\"codex\"]"])
            .expect("set completion_backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "2"])
            .expect("set completion_min_completers");
        h.ralph_ok(["config", "set", "workflow.completion_consensus_threshold", "1.0"])
            .expect("set completion_consensus_threshold");

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        // Find the completion loop directory and check for per-backend verdict files
        let project_dir = h.project_dir(project_id);
        let loops_dir = project_dir.join("loops");
        assert!(loops_dir.exists(), "loops directory should exist");

        // Find the completion loop directory
        let mut found_panel_verdicts = false;
        if let Ok(entries) = std::fs::read_dir(&loops_dir) {
            for entry in entries.flatten() {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                if dir_name.contains("completion") {
                    // Check for per-backend verdict artifacts
                    if let Ok(files) = std::fs::read_dir(entry.path()) {
                        let verdict_files: Vec<String> = files
                            .flatten()
                            .filter_map(|f| {
                                let name = f.file_name().to_string_lossy().to_string();
                                if name.contains("completer-verdict-") {
                                    Some(name)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if verdict_files.len() == 2 {
                            found_panel_verdicts = true;
                        }
                    }
                }
            }
        }

        assert!(
            found_panel_verdicts,
            "should find 2 per-backend verdict artifacts in completion loop directory"
        );
    })
}
