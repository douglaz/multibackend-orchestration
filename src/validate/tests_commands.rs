use super::*;

use std::path::Path;
use std::process::Command;

use crate::validate::assertions::{
    assert_exit_code, assert_json_field, assert_stderr_contains, assert_stdout_contains,
    assert_stdout_eq, git_head_commit,
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
            name: "commands::rollback_dry_run",
            func: rollback_dry_run,
        },
        ConformanceTest {
            name: "commands::rollback_with_completion_attempts",
            func: rollback_with_completion_attempts,
        },
        ConformanceTest {
            name: "commands::rollback_force_push",
            func: rollback_force_push,
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
            name: "commands::config_show_global",
            func: config_show_global,
        },
        ConformanceTest {
            name: "commands::config_show_project",
            func: config_show_project,
        },
        ConformanceTest {
            name: "commands::config_edit_no_editor",
            func: config_edit_no_editor,
        },
        ConformanceTest {
            name: "commands::project_list_empty",
            func: project_list_empty,
        },
        ConformanceTest {
            name: "commands::project_list_multiple",
            func: project_list_multiple,
        },
        ConformanceTest {
            name: "commands::version_long_flag",
            func: version_long_flag,
        },
        ConformanceTest {
            name: "commands::version_short_flag",
            func: version_short_flag,
        },
        ConformanceTest {
            name: "commands::version_no_workspace",
            func: version_no_workspace,
        },
        ConformanceTest {
            name: "commands::exit_code_workspace_not_found",
            func: exit_code_workspace_not_found,
        },
        ConformanceTest {
            name: "commands::exit_code_project_not_found",
            func: exit_code_project_not_found,
        },
        ConformanceTest {
            name: "commands::no_checkpoint_status_defaults",
            func: no_checkpoint_status_defaults,
        },
        ConformanceTest {
            name: "commands::no_checkpoint_history_defaults",
            func: no_checkpoint_history_defaults,
        },
        ConformanceTest {
            name: "commands::config_set_global_preserves_comments",
            func: config_set_global_preserves_comments,
        },
        ConformanceTest {
            name: "commands::config_set_global_preserves_unknown_keys",
            func: config_set_global_preserves_unknown_keys,
        },
        ConformanceTest {
            name: "commands::config_set_global_clears_optional_key",
            func: config_set_global_clears_optional_key,
        },
        ConformanceTest {
            name: "commands::config_set_global_inline_table_set",
            func: config_set_global_inline_table_set,
        },
        ConformanceTest {
            name: "commands::config_set_global_inline_table_clear",
            func: config_set_global_inline_table_clear,
        },
    ]
}

fn status_shows_info(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project("status-test", "Status Test Project", "Status prompt")
            .expect("create_project failed");

        let output = h.ralph(["status"]).expect("ralph status should execute");
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

        let output = h.ralph(["status"]).expect("ralph status should execute");

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
        let project_id = "issue-501";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let output = h.ralph(["history"]).expect("ralph history should execute");
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

        let project_id = "issue-502";
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
        let project_id = "issue-503";
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
        let project_id = "issue-504";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        // Verify two loops exist
        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 2, "expected two loops before rollback");

        // Verify loop-2 dir exists
        let loop2_dir = h.loop_dir(project_id, 2).expect("loop_dir should succeed");
        assert!(
            loop2_dir.is_some(),
            "expected loop-2 directory to exist before rollback"
        );

        // Rollback to loop 1
        h.ralph_ok(["rollback", "1"])
            .expect("ralph rollback 1 should succeed");

        // After rollback: loop-1 should remain, loop-2 should be gone
        let state = h
            .load_state(project_id)
            .expect("load_state after rollback failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop after rollback");

        // loop-1 artifacts should still exist
        let loop1_dir = h.loop_dir(project_id, 1).expect("loop_dir should succeed");
        assert!(
            loop1_dir.is_some(),
            "expected loop-1 directory to remain after rollback"
        );

        // loop-2 artifacts should be removed
        let loop2_dir = h.loop_dir(project_id, 2).expect("loop_dir should succeed");
        assert!(
            loop2_dir.is_none(),
            "expected loop-2 directory to be removed after rollback"
        );
    })
}

fn rollback_resets_phase(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-505";
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

        let state = h
            .load_state(project_id)
            .expect("load_state after rollback failed");
        // With checkpoint-based state derivation, the rollback removes loop
        // artifacts but checkpoint commits on the remote may still influence
        // reconstruction.  Verify the no-checkpoint baseline tuple.
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert!(
            loops.is_empty(),
            "expected no loops after rollback to 0, got {}",
            loops.len()
        );
        assert_json_field(&state, "current_loop", &json!(1));
        assert_json_field(&state, "current_phase", &json!("planning"));
    })
}

fn rollback_hard(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-506";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        let head_before = git_head_commit(&h.repo_root);

        // Rollback --hard to loop 1
        h.ralph_ok(["rollback", "--hard", "1"])
            .expect("ralph rollback --hard 1 should succeed");

        // Verify state is rolled back
        let state = h
            .load_state(project_id)
            .expect("load_state after rollback failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop after hard rollback");

        // Verify git HEAD moved backward (hard reset performed)
        let head_after = git_head_commit(&h.repo_root);
        assert_ne!(
            head_before, head_after,
            "expected HEAD to change after --hard rollback"
        );

        // Verify artifact rollback: loop-2 artifacts removed, loop-1 artifacts remain
        let loop1_dir = h.loop_dir(project_id, 1).expect("loop_dir should succeed");
        assert!(
            loop1_dir.is_some(),
            "expected loop-1 artifacts to remain after --hard rollback"
        );

        let loop2_dir = h.loop_dir(project_id, 2).expect("loop_dir should succeed");
        assert!(
            loop2_dir.is_none(),
            "expected loop-2 artifacts to be removed after --hard rollback"
        );
    })
}

fn rollback_dry_run(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-506-dry-run";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        let head_before = git_head_commit(&h.repo_root);
        let loop2_before = h.loop_dir(project_id, 2).expect("loop_dir should succeed");
        assert!(
            loop2_before.is_some(),
            "expected loop-2 directory to exist before dry-run rollback"
        );

        let output = h
            .ralph(["rollback", "--dry-run", "1"])
            .expect("ralph rollback --dry-run 1 should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "dry-run");

        let head_after = git_head_commit(&h.repo_root);
        assert_eq!(
            head_before, head_after,
            "expected HEAD unchanged after rollback --dry-run"
        );

        let loop2_after = h.loop_dir(project_id, 2).expect("loop_dir should succeed");
        assert!(
            loop2_after.is_some(),
            "expected loop-2 directory to remain after rollback --dry-run"
        );

        let state = h.load_state(project_id).expect("load_state after dry-run");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(
            loops.len(),
            2,
            "rollback --dry-run should not mutate loop state"
        );
    })
}

fn rollback_with_completion_attempts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-506-completion-rollback";
        setup_with_standard_mock(h, project_id);

        // Create a completed feature loop first.
        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        // Trigger a completion-attempt loop.
        let completion_output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run with completion request should execute");
        assert_exit_code(&completion_output, 0);

        let state_before = h
            .load_state(project_id)
            .expect("load_state before rollback");
        let attempts_before = state_before["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be an array");
        assert!(
            !attempts_before.is_empty(),
            "expected at least one completion attempt before rollback"
        );

        let completion_loop_dir = h
            .loop_dir(project_id, 2)
            .expect("loop_dir should succeed before rollback");
        assert!(
            completion_loop_dir.is_some(),
            "expected completion attempt loop directory to exist before rollback"
        );

        let head_before = git_head_commit(&h.repo_root);
        let dry_run = h
            .ralph(["rollback", "--dry-run", "1"])
            .expect("rollback --dry-run 1 should execute");
        assert_exit_code(&dry_run, 0);
        let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
        let reset_ref = dry_stdout
            .split("git reset --hard ")
            .nth(1)
            .and_then(|tail| tail.lines().next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .expect("dry-run output should include reset reference");
        let target_commit = git_rev_parse(&h.repo_root, reset_ref);

        h.ralph_ok(["rollback", "1"])
            .expect("ralph rollback 1 should succeed");

        let head_after = git_head_commit(&h.repo_root);
        assert_ne!(
            head_before, head_after,
            "expected HEAD to move after rolling back completion attempt"
        );
        assert_eq!(
            head_after, target_commit,
            "expected HEAD to reset to rollback target after completion rollback"
        );

        let state_after = h.load_state(project_id).expect("load_state after rollback");
        let loops_after = state_after["loops"]
            .as_array()
            .expect("loops should be an array");
        assert_eq!(
            loops_after.len(),
            1,
            "expected one feature loop after rollback"
        );

        let attempts_after = state_after["completion_attempts"]
            .as_array()
            .expect("completion_attempts should be an array");
        assert!(
            attempts_after.is_empty(),
            "expected completion_attempts to be cleared by rollback"
        );

        let completion_loop_dir_after = h
            .loop_dir(project_id, 2)
            .expect("loop_dir should succeed after rollback");
        assert!(
            completion_loop_dir_after.is_none(),
            "expected completion attempt loop directory to be removed after rollback"
        );
    })
}

fn rollback_force_push(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-506-force-push";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        let branch = git_current_branch(&h.repo_root);
        let remote_head_before = git_remote_branch_head(&h.repo_root, &branch);

        // Capture the exact reset target from dry-run output.
        let dry_run = h
            .ralph(["rollback", "--dry-run", "1"])
            .expect("rollback --dry-run should execute");
        assert_exit_code(&dry_run, 0);
        let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
        let reset_ref = dry_stdout
            .split("git reset --hard ")
            .nth(1)
            .and_then(|tail| tail.lines().next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .expect("dry-run output should include reset reference");
        let target_commit = git_rev_parse(&h.repo_root, reset_ref);

        assert_ne!(
            remote_head_before, target_commit,
            "remote head before rollback should differ from rollback target"
        );

        h.ralph_ok(["rollback", "--hard", "1"])
            .expect("rollback --hard 1 should succeed");

        let local_head_after = git_head_commit(&h.repo_root);
        let remote_head_after = git_remote_branch_head(&h.repo_root, &branch);

        assert_eq!(
            local_head_after, target_commit,
            "local HEAD should match rollback target commit"
        );
        assert_eq!(
            remote_head_after, local_head_after,
            "remote HEAD should match local HEAD after force-push rollback"
        );
        assert_ne!(
            remote_head_before, remote_head_after,
            "remote HEAD should change after force-push rollback"
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

fn config_show_global(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let output = h
            .ralph(["config", "show", "--global"])
            .expect("ralph config show --global should execute");
        assert_exit_code(&output, 0);

        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
            .expect("config show --global output should be valid JSON");
        assert!(
            parsed.get("workspace").is_some(),
            "expected global config JSON to include 'workspace'"
        );
        assert!(
            parsed.get("backends").is_some(),
            "expected global config JSON to include 'backends'"
        );
        assert!(
            parsed.get("workflow").is_some(),
            "expected global config JSON to include 'workflow'"
        );
        assert!(
            parsed.get("templates").is_some(),
            "expected global config JSON to include 'templates'"
        );
    })
}

fn config_show_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project(
            "config-show-proj",
            "Config Show Project",
            "Project config show prompt",
        )
        .expect("create_project failed");

        let output = h
            .ralph(["config", "show", "--project", "config-show-proj"])
            .expect("ralph config show --project should execute");
        assert_exit_code(&output, 0);

        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
            .expect("config show --project output should be valid JSON");
        assert_json_field(&parsed, "scope.type", &json!("project"));
        assert_json_field(&parsed, "scope.project", &json!("config-show-proj"));
        assert!(
            parsed.get("workflow").is_some(),
            "expected project config JSON to include 'workflow'"
        );
        assert!(
            parsed.get("templates").is_some(),
            "expected project config JSON to include 'templates'"
        );
        assert!(
            parsed.get("backends").is_some(),
            "expected project config JSON to include 'backends'"
        );
    })
}

fn config_edit_no_editor(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let config_path = h.repo_root.join(".ralph").join("ralph.toml");
        let before = std::fs::read_to_string(&config_path).expect("read config before edit");

        let missing_editor = h.temp_dir.path().join("definitely-missing-editor");
        assert!(
            !missing_editor.exists(),
            "missing editor path should not exist"
        );
        let missing_editor_str = missing_editor.to_string_lossy().into_owned();

        let output = h
            .ralph_env_with_removals(
                ["config", "edit"],
                &[("EDITOR", &missing_editor_str)],
                &["VISUAL"],
            )
            .expect("ralph config edit should execute");
        assert!(
            !output.status.success(),
            "config edit should fail when editor cannot be launched"
        );
        assert_stderr_contains(&output, "failed to launch editor");

        let after = std::fs::read_to_string(&config_path).expect("read config after edit");
        assert_eq!(
            before, after,
            "config file should remain unchanged when editor launch fails"
        );
    })
}

fn project_list_empty(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let output = h
            .ralph(["project", "list"])
            .expect("ralph project list should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "PROJECTS IN WORKSPACE");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line_count = stdout.lines().count();
        assert!(
            line_count <= 4,
            "expected empty project list to include only header lines, got {line_count} lines:\n{stdout}"
        );
    })
}

fn project_list_multiple(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project("list-proj-a", "List Project A", "List project A prompt")
            .expect("create first project failed");
        h.create_project("list-proj-b", "List Project B", "List project B prompt")
            .expect("create second project failed");

        let output = h
            .ralph(["project", "list"])
            .expect("ralph project list should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "list-proj-a");
        assert_stdout_contains(&output, "list-proj-b");
    })
}

fn version_long_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let output = h
            .ralph(["--version"])
            .expect("ralph --version should execute");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert!(
            stdout.starts_with("ralph "),
            "expected output starting with 'ralph ', got: {stdout}"
        );
        let version_part = &stdout["ralph ".len()..];
        assert!(
            version_part
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit()),
            "expected semver version after 'ralph ', got: {stdout}"
        );
    })
}

fn version_short_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let long_output = h
            .ralph(["--version"])
            .expect("ralph --version should execute");
        assert_exit_code(&long_output, 0);

        let short_output = h.ralph(["-V"]).expect("ralph -V should execute");
        assert_exit_code(&short_output, 0);

        let long_stdout = String::from_utf8_lossy(&long_output.stdout);
        assert_stdout_eq(&short_output, &long_stdout);
    })
}

fn version_no_workspace(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let output = h
            .ralph(["--version"])
            .expect("ralph --version should execute");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert!(
            stdout.starts_with("ralph "),
            "expected output starting with 'ralph ', got: {stdout}"
        );
    })
}

fn exit_code_workspace_not_found(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Do NOT initialize workspace - run a workspace-dependent command
        let output = h.ralph(["status"]).expect("ralph status should execute");
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

fn no_checkpoint_status_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project(
            "nochk-status",
            "No Checkpoint Status",
            "No checkpoint status prompt",
        )
        .expect("create_project failed");

        // No `ralph run` — no checkpoint commits exist.
        let output = h.ralph(["status"]).expect("ralph status should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Status should reflect loop 1, phase planning (not loop 0).
        assert!(
            stdout.contains("Current Loop: 1"),
            "expected 'Current Loop: 1' in status output, got:\n{stdout}"
        );
        assert!(
            stdout.contains("planning"),
            "expected 'planning' phase in status output, got:\n{stdout}"
        );

        // Also verify via --json (project show --json)
        let state = h.load_state("nochk-status").expect("load_state failed");
        assert_json_field(&state, "current_loop", &json!(1));
        assert_json_field(&state, "current_phase", &json!("planning"));
    })
}

fn no_checkpoint_history_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project(
            "nochk-history",
            "No Checkpoint History",
            "No checkpoint history prompt",
        )
        .expect("create_project failed");

        // No `ralph run` — no checkpoint commits exist.
        let output = h.ralph(["history"]).expect("ralph history should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        // With no checkpoint commits and no loops, history should show "(no loops yet)".
        assert!(
            stdout.contains("(no loops yet)"),
            "expected '(no loops yet)' in history output for fresh project, got:\n{stdout}"
        );

        // Verify --json returns an empty array.
        let json_stdout = h
            .ralph_ok(["history", "--json"])
            .expect("ralph history --json should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(&json_stdout).expect("history --json output should be valid JSON");
        let arr = parsed
            .as_array()
            .expect("expected top-level JSON array from history --json");
        assert!(
            arr.is_empty(),
            "expected empty JSON array for no-checkpoint history, got: {parsed}"
        );
    })
}

fn config_set_global_preserves_comments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Write a config with a comment.
        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        std::fs::write(
            &toml_path,
            "# My workspace comment\n[workspace]\nversion = \"1.0\"\n",
        )
        .expect("write custom config");

        // Set a value via CLI.
        h.ralph_ok([
            "config",
            "set",
            "--global",
            "workspace.default_backend",
            "codex",
        ])
        .expect("config set should succeed");

        // Verify comment is preserved.
        let raw = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        assert!(
            raw.contains("# My workspace comment"),
            "comment should be preserved after config set, got:\n{raw}"
        );
        assert!(
            raw.contains("version = \"1.0\""),
            "unrelated key should be preserved after config set, got:\n{raw}"
        );
    })
}

fn config_set_global_preserves_unknown_keys(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Write a config with an unknown user key.
        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        std::fs::write(
            &toml_path,
            "[workspace]\nversion = \"1.0\"\n\n[my_custom_section]\nfoo = \"bar\"\n",
        )
        .expect("write custom config");

        // Set a known value via CLI.
        h.ralph_ok(["config", "set", "--global", "workflow.auto_commit", "false"])
            .expect("config set should succeed");

        // Verify unknown section is preserved.
        let raw = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        assert!(
            raw.contains("[my_custom_section]"),
            "unknown section should be preserved after config set, got:\n{raw}"
        );
        assert!(
            raw.contains("foo = \"bar\""),
            "unknown key should be preserved after config set, got:\n{raw}"
        );
    })
}

fn config_set_global_clears_optional_key(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Set a value first.
        h.ralph_ok(["config", "set", "--global", "workflow.qa_backend", "codex"])
            .expect("config set qa_backend should succeed");

        // Verify it was written.
        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let raw = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        assert!(
            raw.contains("qa_backend"),
            "qa_backend should be present after set, got:\n{raw}"
        );

        // Clear the optional value.
        h.ralph_ok(["config", "set", "--global", "workflow.qa_backend", "null"])
            .expect("config set qa_backend null should succeed");

        // Verify the key was removed from disk.
        let raw_after = std::fs::read_to_string(&toml_path).expect("read ralph.toml after clear");
        assert!(
            !raw_after.contains("qa_backend"),
            "qa_backend should be removed after setting to null, got:\n{raw_after}"
        );
    })
}

fn config_set_global_inline_table_set(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Write a config with inline-table workspace syntax.
        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        std::fs::write(&toml_path, "workspace = { version = \"1.0\" }\n")
            .expect("write inline-table config");

        // Set a value via CLI — this must navigate through the inline table.
        h.ralph_ok([
            "config",
            "set",
            "--global",
            "workspace.default_backend",
            "codex",
        ])
        .expect("config set should succeed through inline table");

        // Verify the value was written and the file is valid.
        let raw = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        let parsed: crate::config::GlobalConfig =
            toml::from_str(&raw).expect("config should parse after inline-table set");
        assert_eq!(
            parsed.workspace.default_backend, "codex",
            "value should be set through inline table"
        );
        assert_eq!(
            parsed.workspace.version, "1.0",
            "existing inline-table value should be preserved"
        );
    })
}

fn config_set_global_inline_table_clear(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        // Write a config with an inline-table workflow section containing qa_backend.
        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        std::fs::write(
            &toml_path,
            "[workspace]\nversion = \"1.0\"\n\n[workflow]\nqa_backend = \"codex\"\nauto_commit = true\n",
        )
        .expect("write config with qa_backend");

        // Clear the optional value.
        h.ralph_ok(["config", "set", "--global", "workflow.qa_backend", "null"])
            .expect("config set qa_backend null should succeed");

        // Verify the key was removed.
        let raw = std::fs::read_to_string(&toml_path).expect("read ralph.toml after clear");
        assert!(
            !raw.contains("qa_backend"),
            "qa_backend should be removed after setting to null, got:\n{raw}"
        );
        assert!(
            raw.contains("auto_commit"),
            "other keys should be preserved after clear, got:\n{raw}"
        );
    })
}

fn git_current_branch(repo_root: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_root)
        .output()
        .expect("git rev-parse --abbrev-ref should execute");
    assert!(
        output.status.success(),
        "failed to read current branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn git_remote_branch_head(repo_root: &Path, branch: &str) -> String {
    let refspec = format!("refs/heads/{branch}");
    let output = Command::new("git")
        .args(["ls-remote", "origin", &refspec])
        .current_dir(repo_root)
        .output()
        .expect("git ls-remote should execute");
    assert!(
        output.status.success(),
        "git ls-remote failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout
        .split_whitespace()
        .next()
        .unwrap_or_else(|| panic!("ls-remote returned no hash for {refspec}: {stdout}"));
    hash.to_owned()
}

fn git_rev_parse(repo_root: &Path, reference: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", reference])
        .current_dir(repo_root)
        .output()
        .expect("git rev-parse should execute");
    assert!(
        output.status.success(),
        "git rev-parse {reference} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
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
