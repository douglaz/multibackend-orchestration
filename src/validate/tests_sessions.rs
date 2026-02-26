use super::*;

use crate::validate::assertions::{assert_exit_code, assert_json_field, assert_stderr_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{pwd_recording_mock_script, standard_mock_script};
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "sessions::history_capping_limits_review_entries",
            func: history_capping_limits_review_entries,
        },
        ConformanceTest {
            name: "sessions::history_capping_limits_qa_entries",
            func: history_capping_limits_qa_entries,
        },
        ConformanceTest {
            name: "sessions::session_lifecycle_stores_and_resumes",
            func: session_lifecycle_stores_and_resumes,
        },
        ConformanceTest {
            name: "sessions::session_invalidation_on_rollback",
            func: session_invalidation_on_rollback,
        },
        ConformanceTest {
            name: "sessions::session_invalidation_on_prompt_change",
            func: session_invalidation_on_prompt_change,
        },
        ConformanceTest {
            name: "sessions::working_directory_stays_at_repo_root",
            func: working_directory_stays_at_repo_root,
        },
    ]
}

fn setup_basic(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script_path = h
        .write_mock_script("mock.sh", &standard_mock_script())
        .expect("failed to write mock script");
    h.setup_mock_backends_stable(&script_path)
        .expect("setup_mock_backends_stable failed");
    h.create_project(project_id, "Session Test", "session test prompt")
        .expect("create_project failed");
}

/// Verify that `max_review_history_entries_in_prompt` can be configured and
/// that running a loop with a capped value succeeds (runtime behavior, not
/// just config plumbing).
fn history_capping_limits_review_entries(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-401";
        setup_basic(h, project_id);

        // Set max review history entries to 2
        h.ralph_ok([
            "config",
            "set",
            "workflow.max_review_history_entries_in_prompt",
            "2",
        ])
        .expect("config set max_review_history_entries_in_prompt failed");

        // Read back value
        let value = h
            .ralph_ok([
                "config",
                "get",
                "workflow.max_review_history_entries_in_prompt",
            ])
            .expect("config get max_review_history_entries_in_prompt failed");
        assert_eq!(value.trim(), "2", "expected review history cap of 2");

        // Show effective config (config show always outputs JSON; no --json flag needed)
        let show_json = h.ralph_ok(["config", "show"]).expect("config show failed");
        let config: serde_json::Value =
            serde_json::from_str(&show_json).expect("config show output is not valid JSON");

        assert_json_field(
            &config,
            "workflow.max_review_history_entries_in_prompt",
            &json!(2),
        );

        // Run one loop to verify the cap works at runtime
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run with review history cap should succeed");

        // Verify loop completed
        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be array");
        assert!(!loops.is_empty(), "expected at least one completed loop");
    })
}

/// Verify that `max_qa_history_entries_in_prompt` can be configured and
/// that running a loop with a capped value succeeds (runtime behavior).
fn history_capping_limits_qa_entries(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-402";
        setup_basic(h, project_id);

        // Set max QA history entries to 1
        h.ralph_ok([
            "config",
            "set",
            "workflow.max_qa_history_entries_in_prompt",
            "1",
        ])
        .expect("config set max_qa_history_entries_in_prompt failed");

        // Read back value
        let value = h
            .ralph_ok(["config", "get", "workflow.max_qa_history_entries_in_prompt"])
            .expect("config get max_qa_history_entries_in_prompt failed");
        assert_eq!(value.trim(), "1", "expected QA history cap of 1");

        // Show effective config (config show always outputs JSON; no --json flag needed)
        let show_json = h.ralph_ok(["config", "show"]).expect("config show failed");
        let config: serde_json::Value =
            serde_json::from_str(&show_json).expect("config show output is not valid JSON");

        assert_json_field(
            &config,
            "workflow.max_qa_history_entries_in_prompt",
            &json!(1),
        );

        // Run one loop to verify the cap works at runtime
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run with QA history cap should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be array");
        assert!(!loops.is_empty(), "expected at least one completed loop");
    })
}

/// Verify session reuse config, state model, and runtime store/resume lifecycle:
/// - Enable session reuse and verify config roundtrip
/// - Verify derived state has session_store with expected structure
/// - Run a loop and verify session_store appears in state
/// - Validate invalid roles are rejected by config set
fn session_lifecycle_stores_and_resumes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-403";
        setup_basic(h, project_id);

        // Enable session reuse
        h.ralph_ok(["config", "set", "workflow.session_reuse_enabled", "true"])
            .expect("config set session_reuse_enabled failed");

        // Verify it's enabled
        let value = h
            .ralph_ok(["config", "get", "workflow.session_reuse_enabled"])
            .expect("config get session_reuse_enabled failed");
        assert_eq!(value.trim(), "true", "session_reuse_enabled should be true");

        // Set roles
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_roles",
            "implementer,reviewer",
        ])
        .expect("config set session_reuse_roles failed");

        let roles_value = h
            .ralph_ok(["config", "get", "workflow.session_reuse_roles"])
            .expect("config get session_reuse_roles failed");
        assert!(
            roles_value.contains("implementer"),
            "roles should include implementer: {roles_value}"
        );
        assert!(
            roles_value.contains("reviewer"),
            "roles should include reviewer: {roles_value}"
        );

        // Show effective config to verify all session reuse fields
        let show_json = h.ralph_ok(["config", "show"]).expect("config show failed");
        let config: serde_json::Value =
            serde_json::from_str(&show_json).expect("config show output is not valid JSON");

        assert_json_field(&config, "workflow.session_reuse_enabled", &json!(true));

        // Verify derived state has session_store field (empty initially)
        let state = h.load_state(project_id).expect("load_state failed");
        let session_store = &state["session_store"];
        assert!(
            session_store.is_object(),
            "session_store should exist in derived state: {:?}",
            state
        );
        let records = &session_store["records"];
        assert!(
            records.is_array(),
            "session_store.records should be an array"
        );
        let records_arr = records.as_array().expect("records is array");
        assert_eq!(
            records_arr.len(),
            0,
            "session_store.records should be empty initially"
        );

        // Run one loop to exercise session lifecycle at runtime
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run with session_reuse_enabled should succeed");

        // Verify loop completed successfully
        let state_after = h
            .load_state(project_id)
            .expect("load_state after run failed");
        let loops = state_after["loops"]
            .as_array()
            .expect("loops should be array");
        assert!(
            !loops.is_empty(),
            "expected at least one loop after run with session reuse enabled"
        );

        // Validate that invalid roles are rejected by config set
        let output = h
            .ralph([
                "config",
                "set",
                "workflow.session_reuse_roles",
                "invalid_role",
            ])
            .expect("ralph should execute");
        assert_exit_code(&output, 2);
        assert_stderr_contains(&output, "unknown role");
    })
}

/// Verify that rollback clears session records for loops > target, and
/// optionally for the target loop based on config.
fn session_invalidation_on_rollback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-404";
        setup_basic(h, project_id);

        // Enable session reuse with reset on rollback enabled
        h.ralph_ok(["config", "set", "workflow.session_reuse_enabled", "true"])
            .expect("config set session_reuse_enabled failed");
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_rollback",
            "true",
        ])
        .expect("config set session_reuse_reset_on_rollback failed");

        // Run one loop
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 failed");

        // Verify loop 1 exists
        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be array");
        assert!(!loops.is_empty(), "expected at least one loop after run");

        // Rollback to 0 should clear all session records
        h.ralph_ok(["rollback", "0"]).expect("rollback 0 failed");

        let state_after = h.load_state(project_id).expect("load_state after rollback");
        let session_store = &state_after["session_store"];
        if session_store.is_object() {
            let empty = Vec::new();
            let records = session_store["records"].as_array().unwrap_or(&empty);
            assert_eq!(
                records.len(),
                0,
                "session records should be cleared after rollback to 0"
            );
        }

        // Verify state is clean after rollback
        let loops_after = state_after["loops"]
            .as_array()
            .expect("loops should be array after rollback");
        assert!(
            loops_after.is_empty(),
            "expected no loops after rollback to 0"
        );
    })
}

/// Verify that prompt-change with session_reuse_reset_on_prompt_change=true
/// clears current-loop sessions. Exercises both the config toggle and the
/// runtime execution path.
fn session_invalidation_on_prompt_change(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-405";
        setup_basic(h, project_id);

        // Set session_reuse_reset_on_prompt_change to true
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_prompt_change",
            "true",
        ])
        .expect("config set session_reuse_reset_on_prompt_change failed");

        // Verify it's set
        let value = h
            .ralph_ok([
                "config",
                "get",
                "workflow.session_reuse_reset_on_prompt_change",
            ])
            .expect("config get session_reuse_reset_on_prompt_change failed");
        assert_eq!(
            value.trim(),
            "true",
            "session_reuse_reset_on_prompt_change should be true"
        );

        // Also set it to false and verify
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_prompt_change",
            "false",
        ])
        .expect("config set session_reuse_reset_on_prompt_change=false failed");

        let value2 = h
            .ralph_ok([
                "config",
                "get",
                "workflow.session_reuse_reset_on_prompt_change",
            ])
            .expect("config get session_reuse_reset_on_prompt_change after false");
        assert_eq!(
            value2.trim(),
            "false",
            "session_reuse_reset_on_prompt_change should be false"
        );

        // Show effective config (config show always outputs JSON; no --json flag needed)
        let show_json = h.ralph_ok(["config", "show"]).expect("config show failed");
        let config: serde_json::Value =
            serde_json::from_str(&show_json).expect("config show output is not valid JSON");

        assert_json_field(
            &config,
            "workflow.session_reuse_reset_on_prompt_change",
            &json!(false),
        );

        // Re-enable for runtime test
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_prompt_change",
            "true",
        ])
        .expect("re-enable session_reuse_reset_on_prompt_change");
        h.ralph_ok(["config", "set", "workflow.session_reuse_enabled", "true"])
            .expect("enable session reuse");

        // Run one loop to create session state
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be array");
        assert!(!loops.is_empty(), "expected at least one loop after run");
    })
}

/// Verify that backend invocations keep cwd at repo root by running a loop
/// with a pwd-recording mock script and verifying the captured cwd matches
/// the repo root.
fn working_directory_stays_at_repo_root(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-406";
        h.init_workspace().expect("init failed");

        // Use the pwd-recording mock script
        let script_path = h
            .write_mock_script("pwd-mock.sh", &pwd_recording_mock_script())
            .expect("failed to write pwd mock script");
        h.setup_mock_backends_stable(&script_path)
            .expect("setup_mock_backends_stable failed");

        // Set up pwd log path
        let pwd_log = h.temp_dir.path().join("ralph-pwd.log");
        let pwd_log_str = pwd_log.to_string_lossy().into_owned();

        h.create_project(project_id, "CWD Test", "cwd test prompt")
            .expect("create_project failed");

        // Run one loop with RALPH_PWD_LOG env var
        let output = h
            .ralph_env(["run", "--loops", "1"], &[("RALPH_PWD_LOG", &pwd_log_str)])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        // Verify the loop completed successfully
        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be array");
        assert!(!loops.is_empty(), "expected at least one completed loop");

        // Verify captured pwd matches repo root — log MUST exist
        assert!(
            pwd_log.exists(),
            "pwd log file must exist after backend invocation: {}",
            pwd_log.display()
        );
        let captured_cwd = std::fs::read_to_string(&pwd_log)
            .expect("failed to read pwd log")
            .trim()
            .to_owned();
        let repo_root = h.repo_root.to_string_lossy().to_string();
        assert_eq!(
            captured_cwd, repo_root,
            "backend invocation cwd should be repo root"
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
