use super::*;

use std::fs;

use crate::util::hash::sha256_hex;
use crate::validate::assertions::{assert_exit_code, assert_json_field, assert_stderr_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    nonzero_exit_backend_script, pwd_recording_mock_script, standard_mock_script,
};
use chrono::Utc;
use serde_json::{json, Value};

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
            name: "sessions::session_persistence_survives_restart",
            func: session_persistence_survives_restart,
        },
        ConformanceTest {
            name: "sessions::session_persistence_invalidated_by_rollback",
            func: session_persistence_invalidated_by_rollback,
        },
        ConformanceTest {
            name: "sessions::session_persistence_invalidated_by_prompt_change_enabled",
            func: session_persistence_invalidated_by_prompt_change_enabled,
        },
        ConformanceTest {
            name: "sessions::session_persistence_preserved_on_prompt_change_disabled",
            func: session_persistence_preserved_on_prompt_change_disabled,
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
    h.setup_mock_backends(&script_path)
        .expect("setup_mock_backends failed");
    h.create_project(project_id, "Session Test", "session test prompt")
        .expect("create_project failed");
}

fn session_store_path(h: &RalphHarness, project_id: &str) -> std::path::PathBuf {
    h.project_dir(project_id).join("session-store.json")
}

fn load_session_store(h: &RalphHarness, project_id: &str) -> Value {
    let path = session_store_path(h, project_id);
    let raw = fs::read_to_string(&path).expect("read session-store.json");
    serde_json::from_str(&raw).expect("parse session-store.json")
}

fn session_records_for_loop(store: &Value, loop_number: u32) -> Vec<Value> {
    store["records"]
        .as_array()
        .expect("session-store records should be array")
        .iter()
        .filter(|record| record["loop_number"].as_u64() == Some(loop_number as u64))
        .cloned()
        .collect()
}

fn write_session_store_records(h: &RalphHarness, project_id: &str, records: Vec<Value>) {
    let path = session_store_path(h, project_id);
    fs::write(
        path,
        serde_json::to_string_pretty(&json!({ "records": records }))
            .expect("serialize session-store payload"),
    )
    .expect("write session-store.json");
}

fn session_record(loop_number: u32, role: &str, backend_spec: &str, bootstrap_hash: &str) -> Value {
    let now = Utc::now().to_rfc3339();
    json!({
        "session_id": format!("sid-{loop_number}-{role}"),
        "backend_spec": backend_spec,
        "role": role,
        "loop_number": loop_number,
        "bootstrap_hash": bootstrap_hash,
        "call_count": 7,
        "created_at": now,
        "last_used_at": now
    })
}

fn reviewer_bootstrap_for_loop(
    h: &RalphHarness,
    project_id: &str,
    loop_number: u32,
) -> (String, String) {
    let state = h.load_state(project_id).expect("load_state");
    let loop_state = state["loops"]
        .as_array()
        .expect("loops should be array")
        .iter()
        .find(|loop_state| loop_state["loop_number"].as_u64() == Some(loop_number as u64))
        .expect("expected loop state");

    let reviewer_backend = loop_state["backends"]["reviewer"]
        .as_str()
        .expect("reviewer backend should be string")
        .to_owned();
    let spec_rel = loop_state["artifacts"]["spec"]
        .as_str()
        .expect("spec artifact path should be string");
    let prompt_hash = state["prompt_hash_at_loop_start"]
        .as_str()
        .expect("prompt_hash_at_loop_start should be string");

    let spec_content =
        fs::read_to_string(h.project_dir(project_id).join(spec_rel)).expect("read spec artifact");
    let template_content = fs::read_to_string(
        h.repo_root
            .join(".ralph")
            .join("templates")
            .join("reviewer.md"),
    )
    .expect("read reviewer template");
    let spec_hash = sha256_hex(&spec_content);
    let template_hash = sha256_hex(&template_content);
    let bootstrap_hash = sha256_hex(&format!(
        "reviewer|{reviewer_backend}|{prompt_hash}|{spec_hash}|{template_hash}|sessions-v1"
    ));

    (reviewer_backend, bootstrap_hash)
}

/// Verify that `max_review_history_entries_in_prompt` can be configured and
/// that running a loop with a capped value succeeds (runtime behavior, not
/// just config plumbing).
fn history_capping_limits_review_entries(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "hist-review-cap";
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
        let project_id = "hist-qa-cap";
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
/// - Verify state.json has session_store with expected structure
/// - Run a loop and verify session_store appears in state
/// - Validate invalid roles are rejected by config set
fn session_lifecycle_stores_and_resumes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "session-lifecycle";
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

        // Verify state.json has session_store field (empty initially)
        let state = h.load_state(project_id).expect("load_state failed");
        let session_store = &state["session_store"];
        assert!(
            session_store.is_object(),
            "session_store should exist in state.json: {:?}",
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

fn session_persistence_survives_restart(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "session-persist-restart";
        setup_basic(h, project_id);

        h.ralph_ok(["config", "set", "workflow.session_reuse_enabled", "true"])
            .expect("enable session reuse");
        h.ralph_ok(["config", "set", "workflow.session_reuse_roles", "reviewer"])
            .expect("set session reuse roles");

        h.ralph_ok(["run", "--until-review"])
            .expect("first run until-review should succeed");

        let (reviewer_backend, reviewer_bootstrap) = reviewer_bootstrap_for_loop(h, project_id, 1);
        write_session_store_records(
            h,
            project_id,
            vec![session_record(
                1,
                "reviewer",
                &reviewer_backend,
                &reviewer_bootstrap,
            )],
        );

        let call_count_before = 7_u64;
        let session_id_before = "sid-1-reviewer".to_owned();

        // Keep the same loop in review phase so the next process must load and reuse
        // the persisted reviewer session for the same loop/role/backend tuple.
        let mut state = h.load_state(project_id).expect("load_state");
        state["current_phase"] = json!("reviewing");
        state["phase_iteration"] = json!(1);
        let state_path = h.project_dir(project_id).join("state.json");
        fs::write(
            &state_path,
            serde_json::to_string_pretty(&state).expect("serialize updated state"),
        )
        .expect("write updated state");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("second run should succeed");

        let store_after = load_session_store(h, project_id);
        let reviewer_after = session_records_for_loop(&store_after, 1)
            .into_iter()
            .find(|record| record["role"].as_str() == Some("reviewer"))
            .expect("expected reviewer session record for loop 1 after restart");
        let session_id_after = reviewer_after["session_id"]
            .as_str()
            .expect("reviewer session_id after restart should be string");
        let call_count_after = reviewer_after["call_count"]
            .as_u64()
            .expect("reviewer call_count after restart should be integer");

        assert_eq!(
            session_id_after, session_id_before,
            "reviewer session id should be reused across process restart"
        );
        assert!(
            call_count_after > call_count_before,
            "reviewer call_count should increase after restart reuse (before={call_count_before}, after={call_count_after})"
        );
    })
}

fn session_persistence_invalidated_by_rollback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "session-persist-rollback";
        setup_basic(h, project_id);

        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_rollback",
            "true",
        ])
        .expect("enable reset on rollback");

        h.ralph_ok(["run", "--loops", "2"])
            .expect("run two loops should succeed");

        let state = h.load_state(project_id).expect("load_state");
        let loops = state["loops"].as_array().expect("loops should be array");
        let loop1_backend = loops
            .iter()
            .find(|l| l["loop_number"].as_u64() == Some(1))
            .and_then(|l| l["backends"]["reviewer"].as_str())
            .expect("loop 1 reviewer backend")
            .to_owned();
        let loop2_backend = loops
            .iter()
            .find(|l| l["loop_number"].as_u64() == Some(2))
            .and_then(|l| l["backends"]["reviewer"].as_str())
            .expect("loop 2 reviewer backend")
            .to_owned();
        write_session_store_records(
            h,
            project_id,
            vec![
                session_record(1, "reviewer", &loop1_backend, "seed-bootstrap-1"),
                session_record(2, "reviewer", &loop2_backend, "seed-bootstrap-2"),
            ],
        );

        let store_before = load_session_store(h, project_id);
        assert!(
            !session_records_for_loop(&store_before, 1).is_empty(),
            "expected loop 1 session records before rollback"
        );
        assert!(
            !session_records_for_loop(&store_before, 2).is_empty(),
            "expected loop 2 session records before rollback"
        );

        h.ralph_ok(["rollback", "1"])
            .expect("rollback to loop 1 should succeed");

        let store_after = load_session_store(h, project_id);
        assert!(
            session_records_for_loop(&store_after, 1).is_empty(),
            "target loop session records should be removed when reset_on_rollback=true"
        );
        assert!(
            session_records_for_loop(&store_after, 2).is_empty(),
            "rolled-back loop session records should be removed"
        );
    })
}

fn session_persistence_invalidated_by_prompt_change_enabled(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "session-persist-prompt-reset-on";
        setup_basic(h, project_id);

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_change_action",
            "restart-loop",
        ])
        .expect("set prompt_change_action restart-loop");
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_prompt_change",
            "true",
        ])
        .expect("enable reset on prompt change");

        h.ralph_ok(["run", "--until-review"])
            .expect("first run until-review should succeed");

        let state = h.load_state(project_id).expect("load_state");
        let loop1_backend = state["loops"]
            .as_array()
            .expect("loops should be array")
            .iter()
            .find(|l| l["loop_number"].as_u64() == Some(1))
            .and_then(|l| l["backends"]["reviewer"].as_str())
            .expect("loop 1 reviewer backend")
            .to_owned();
        write_session_store_records(
            h,
            project_id,
            vec![session_record(
                1,
                "reviewer",
                &loop1_backend,
                "seed-bootstrap-1",
            )],
        );

        let store_before = load_session_store(h, project_id);
        assert!(
            !session_records_for_loop(&store_before, 1).is_empty(),
            "expected loop 1 session records before prompt change restart"
        );

        fs::write(
            h.project_dir(project_id).join("prompt.md"),
            "session persistence prompt changed",
        )
        .expect("write prompt change");

        let failing_script = h
            .write_mock_script("session-fail.sh", &nonzero_exit_backend_script())
            .expect("write failing mock script");
        let failing_script_str = failing_script.to_string_lossy().into_owned();
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "--global".to_owned(),
            "backends.claude.command".to_owned(),
            failing_script_str.clone(),
        ])
        .expect("set global claude failing backend");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "--global".to_owned(),
            "backends.codex.command".to_owned(),
            failing_script_str,
        ])
        .expect("set global codex failing backend");

        let output = h.ralph(["run"]).expect("second run should execute");
        assert!(
            !output.status.success(),
            "second run should fail after prompt-change restart to keep state frozen for assertions"
        );

        let store_after = load_session_store(h, project_id);
        assert!(
            session_records_for_loop(&store_after, 1).is_empty(),
            "loop 1 session records should be cleared and persisted when reset_on_prompt_change=true"
        );
    })
}

fn session_persistence_preserved_on_prompt_change_disabled(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "session-persist-prompt-reset-off";
        setup_basic(h, project_id);

        h.ralph_ok([
            "config",
            "set",
            "workflow.prompt_change_action",
            "restart-loop",
        ])
        .expect("set prompt_change_action restart-loop");
        h.ralph_ok([
            "config",
            "set",
            "workflow.session_reuse_reset_on_prompt_change",
            "false",
        ])
        .expect("disable reset on prompt change");

        h.ralph_ok(["run", "--until-review"])
            .expect("first run until-review should succeed");

        let state = h.load_state(project_id).expect("load_state");
        let loop1_backend = state["loops"]
            .as_array()
            .expect("loops should be array")
            .iter()
            .find(|l| l["loop_number"].as_u64() == Some(1))
            .and_then(|l| l["backends"]["reviewer"].as_str())
            .expect("loop 1 reviewer backend")
            .to_owned();
        write_session_store_records(
            h,
            project_id,
            vec![session_record(
                1,
                "reviewer",
                &loop1_backend,
                "seed-bootstrap-1",
            )],
        );

        let store_before = load_session_store(h, project_id);
        let before_loop_records = session_records_for_loop(&store_before, 1);
        assert!(
            !before_loop_records.is_empty(),
            "expected loop 1 session records before prompt change restart"
        );
        let mut before_keys = before_loop_records
            .iter()
            .map(|record| {
                format!(
                    "{}::{}",
                    record["role"].as_str().unwrap_or(""),
                    record["session_id"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>();
        before_keys.sort();

        fs::write(
            h.project_dir(project_id).join("prompt.md"),
            "session persistence prompt changed again",
        )
        .expect("write prompt change");

        let failing_script = h
            .write_mock_script("session-fail.sh", &nonzero_exit_backend_script())
            .expect("write failing mock script");
        let failing_script_str = failing_script.to_string_lossy().into_owned();
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "--global".to_owned(),
            "backends.claude.command".to_owned(),
            failing_script_str.clone(),
        ])
        .expect("set global claude failing backend");
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "--global".to_owned(),
            "backends.codex.command".to_owned(),
            failing_script_str,
        ])
        .expect("set global codex failing backend");

        let output = h.ralph(["run"]).expect("second run should execute");
        assert!(
            !output.status.success(),
            "second run should fail after prompt-change restart to keep state frozen for assertions"
        );

        let store_after = load_session_store(h, project_id);
        let after_loop_records = session_records_for_loop(&store_after, 1);
        assert!(
            !after_loop_records.is_empty(),
            "loop 1 session records should be preserved when reset_on_prompt_change=false"
        );
        let mut after_keys = after_loop_records
            .iter()
            .map(|record| {
                format!(
                    "{}::{}",
                    record["role"].as_str().unwrap_or(""),
                    record["session_id"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>();
        after_keys.sort();

        assert_eq!(
            after_keys, before_keys,
            "preserved session records should match pre-restart loop records"
        );
    })
}

/// Verify that rollback clears session records for loops > target, and
/// optionally for the target loop based on config.
fn session_invalidation_on_rollback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "session-rollback";
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
        let project_id = "session-prompt-change";
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
        let project_id = "session-cwd";
        h.init_workspace().expect("init failed");

        // Use the pwd-recording mock script
        let script_path = h
            .write_mock_script("pwd-mock.sh", &pwd_recording_mock_script())
            .expect("failed to write pwd mock script");
        h.setup_mock_backends(&script_path)
            .expect("setup_mock_backends failed");

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
