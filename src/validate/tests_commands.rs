use super::*;

use crate::validate::assertions::{
    assert_exit_code, assert_git_tag_exists, assert_json_field, assert_stdout_contains,
    git_head_commit, git_tag_commit,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::standard_mock_script;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "commands::status_shows_info",
            func: status_shows_info,
        },
        ConformanceTest {
            name: "commands::status_no_active_project",
            func: status_no_active_project,
        },
        ConformanceTest {
            name: "commands::history_shows_loops",
            func: history_shows_loops,
        },
        ConformanceTest {
            name: "commands::history_json",
            func: history_json,
        },
        ConformanceTest {
            name: "commands::history_verbose",
            func: history_verbose,
        },
        ConformanceTest {
            name: "commands::rollback_removes_loops",
            func: rollback_removes_loops,
        },
        ConformanceTest {
            name: "commands::rollback_resets_phase",
            func: rollback_resets_phase,
        },
        ConformanceTest {
            name: "commands::rollback_hard",
            func: rollback_hard,
        },
        ConformanceTest {
            name: "commands::config_get",
            func: config_get,
        },
        ConformanceTest {
            name: "commands::config_set",
            func: config_set,
        },
        ConformanceTest {
            name: "commands::exit_code_workspace_not_found",
            func: exit_code_workspace_not_found,
        },
        ConformanceTest {
            name: "commands::exit_code_project_not_found",
            func: exit_code_project_not_found,
        },
    ]
}

fn status_shows_info(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project("status-test", "Status Test Project", "Status prompt")
            .expect("create_project failed");

        let output = h
            .ralph(["status"])
            .expect("ralph status should execute");
        assert_exit_code(&output, 0);

        // Output should include project identity and phase/status fields
        assert_stdout_contains(&output, "status-test");
        assert_stdout_contains(&output, "planning");
        assert_stdout_contains(&output, "pending");
    })
}

fn status_no_active_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        // Do NOT create or activate any project

        let output = h
            .ralph(["status"])
            .expect("ralph status should execute");

        // Should fail with exit code 2 (not a crash)
        assert_exit_code(&output, 2);

        // Should have a meaningful message mentioning active project in stdout or stderr
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let combined_lower = combined.to_lowercase();
        assert!(
            combined_lower.contains("active project") || combined_lower.contains("no project"),
            "expected error message to mention 'active project' or 'no project', got:\n{combined}"
        );
    })
}

fn history_shows_loops(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "history-loops";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let output = h
            .ralph(["history"])
            .expect("ralph history should execute");
        assert_exit_code(&output, 0);

        // History output should list loop entries
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Loop 1") || stdout.contains("loop 1") || stdout.contains("loop-1"),
            "expected history to list loop entries, got:\n{stdout}"
        );
    })
}

fn history_json(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::validate::assertions::assert_json_array;

        let project_id = "history-json";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let stdout = h
            .ralph_ok(["history", "--json"])
            .expect("ralph history --json should succeed");

        // Must be valid JSON
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("history --json output should be valid JSON");

        // Per spec contract: output must be a top-level JSON array (not an object wrapper)
        let arr = assert_json_array(&parsed);
        assert!(
            !arr.is_empty(),
            "expected at least one loop entry in history JSON array"
        );
    })
}

fn history_verbose(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "history-verbose";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let default_stdout = h
            .ralph_ok(["history"])
            .expect("ralph history should succeed");
        let verbose_stdout = h
            .ralph_ok(["history", "--verbose"])
            .expect("ralph history --verbose should succeed");

        // Verbose output should be strictly richer (longer) than default
        assert!(
            verbose_stdout.len() > default_stdout.len(),
            "expected verbose history to be richer than default history.\ndefault ({} bytes):\n{}\nverbose ({} bytes):\n{}",
            default_stdout.len(),
            default_stdout,
            verbose_stdout.len(),
            verbose_stdout
        );
    })
}

fn rollback_removes_loops(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "rollback-remove";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        // Verify two loops exist
        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 2, "expected two loops before rollback");

        // Verify loop-2 dir exists
        let loop2_dir = h
            .loop_dir(project_id, 2)
            .expect("loop_dir should succeed");
        assert!(loop2_dir.is_some(), "expected loop-2 directory to exist before rollback");

        // Rollback to loop 1
        h.ralph_ok(["rollback", "1"])
            .expect("ralph rollback 1 should succeed");

        // After rollback: loop-1 should remain, loop-2 should be gone
        let state = h.load_state(project_id).expect("load_state after rollback failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop after rollback");

        // loop-1 artifacts should still exist
        let loop1_dir = h
            .loop_dir(project_id, 1)
            .expect("loop_dir should succeed");
        assert!(loop1_dir.is_some(), "expected loop-1 directory to remain after rollback");

        // loop-2 artifacts should be removed
        let loop2_dir = h
            .loop_dir(project_id, 2)
            .expect("loop_dir should succeed");
        assert!(loop2_dir.is_none(), "expected loop-2 directory to be removed after rollback");
    })
}

fn rollback_resets_phase(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "rollback-phase";
        setup_with_standard_mock(h, project_id);

        // Run one loop with --until-review to stop in a non-planning phase
        h.ralph_ok(["run", "--until-review"])
            .expect("ralph run --until-review should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let phase = state["current_phase"]
            .as_str()
            .expect("current_phase should be a string");
        assert_ne!(
            phase, "planning",
            "expected non-planning phase after --until-review"
        );

        // Rollback to loop 0 (before any loops)
        h.ralph_ok(["rollback", "0"])
            .expect("ralph rollback 0 should succeed");

        let state = h.load_state(project_id).expect("load_state after rollback failed");
        assert_json_field(&state, "current_phase", &json!("planning"));

        // Assert iteration reset semantics
        assert_json_field(&state, "phase_iteration", &json!(1));

        // Optionally verify current_loop is reset to 0
        if state.get("current_loop").is_some() {
            assert_json_field(&state, "current_loop", &json!(0));
        }
    })
}

fn rollback_hard(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "rollback-hard";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        // Get the git reference for loop-1 tag
        let tag_name = format!("ralph/{project_id}/loop-1");
        assert_git_tag_exists(&h.repo_root, &tag_name);
        let loop1_commit = git_tag_commit(&h.repo_root, &tag_name);

        // Rollback --hard to loop 1
        h.ralph_ok(["rollback", "--hard", "1"])
            .expect("ralph rollback --hard 1 should succeed");

        // Verify state is rolled back
        let state = h.load_state(project_id).expect("load_state after rollback failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop after hard rollback");
        assert_json_field(&state, "current_phase", &json!("planning"));

        // Verify git HEAD matches loop-1 tag commit
        let head = git_head_commit(&h.repo_root);
        assert_eq!(
            head, loop1_commit,
            "expected HEAD to be at loop-1 tag after --hard rollback"
        );

        // Verify artifact rollback: loop-2 artifacts removed, loop-1 artifacts remain
        let loop1_dir = h
            .loop_dir(project_id, 1)
            .expect("loop_dir should succeed");
        assert!(
            loop1_dir.is_some(),
            "expected loop-1 artifacts to remain after --hard rollback"
        );

        let loop2_dir = h
            .loop_dir(project_id, 2)
            .expect("loop_dir should succeed");
        assert!(
            loop2_dir.is_none(),
            "expected loop-2 artifacts to be removed after --hard rollback"
        );
    })
}

fn config_get(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Per spec contract: key is `planner_backend` (not `workflow.planner_backend`)
        let stdout = h
            .ralph_ok(["config", "get", "planner_backend"])
            .expect("ralph config get should succeed");

        // Should return a non-empty value
        let value = stdout.trim();
        assert!(
            !value.is_empty(),
            "expected config get planner_backend to return a non-empty value"
        );
    })
}

fn config_set(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Per spec contract: key is `planner_backend` (not `workflow.planner_backend`)
        h.ralph_ok(["config", "set", "planner_backend", "codex"])
            .expect("ralph config set should succeed");

        // Read-after-write: verify the value persisted
        let stdout = h
            .ralph_ok(["config", "get", "planner_backend"])
            .expect("ralph config get should succeed");

        let value = stdout.trim();
        assert!(
            value.contains("codex"),
            "expected config get planner_backend to return 'codex' after set, got: '{value}'"
        );
    })
}

fn exit_code_workspace_not_found(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Do NOT initialize workspace - run a workspace-dependent command
        let output = h
            .ralph(["status"])
            .expect("ralph status should execute");
        assert_exit_code(&output, 2);
    })
}

fn exit_code_project_not_found(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let output = h
            .ralph(["project", "show", "nonexistent"])
            .expect("ralph project show should execute");
        assert_exit_code(&output, 2);
    })
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
        "Commands Conformance Project",
        "Commands suite test prompt",
    )
    .expect("create_project failed");
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
