use super::*;
use std::fs;
use std::process::Command;

use crate::validate::assertions::{
    assert_exit_code, assert_git_tag_exists, assert_json_field, assert_stdout_contains,
    assert_path_not_exists, assert_stdout_eq, git_head_commit, git_tag_commit,
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
            name: "commands::rollback_reconstruction_marker_boundary",
            func: rollback_reconstruction_marker_boundary,
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
        let project_id = "history-loops";
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
        let head_before = git_head_commit(&h.repo_root);

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
        let head_after = git_head_commit(&h.repo_root);
        assert_eq!(head_after, head_before, "soft rollback should not change HEAD");

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

        let marker_path = h.project_dir(project_id).join(".rollback-target");
        let marker = fs::read_to_string(&marker_path).expect("marker should exist");
        assert_eq!(marker.trim(), "1", "soft rollback should write marker");
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

        let state = h
            .load_state(project_id)
            .expect("load_state after rollback failed");
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
        setup_remote_tracking(h, project_id);

        // Get the git reference for loop-1 tag
        let tag_name = format!("ralph/{project_id}/loop-1");
        assert_git_tag_exists(&h.repo_root, &tag_name);
        let loop1_commit = git_tag_commit(&h.repo_root, &tag_name);
        let marker_path = h.project_dir(project_id).join(".rollback-target");
        fs::write(&marker_path, "1\n").expect("write marker");

        // Rollback --hard to loop 1
        h.ralph_ok(["rollback", "--hard", "1"])
            .expect("ralph rollback --hard 1 should succeed");

        // Verify state is rolled back
        let state = h
            .load_state(project_id)
            .expect("load_state after rollback failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop after hard rollback");
        assert_json_field(&state, "current_phase", &json!("planning"));

        // Verify git HEAD matches loop-1 tag commit
        let head = git_head_commit(&h.repo_root);
        assert_eq!(
            head, loop1_commit,
            "expected HEAD to be at loop-1 tag after --hard rollback"
        );
        assert_path_not_exists(&marker_path);

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

        let remote_branch = format!("refs/heads/ralph/{project_id}");
        let remote_line = git_stdout(
            &h.repo_root,
            &["ls-remote", "--heads", "origin", &remote_branch],
        );
        let remote_commit = remote_line
            .split_whitespace()
            .next()
            .expect("remote branch should exist");
        assert_eq!(
            remote_commit, loop1_commit,
            "expected hard rollback to force-push remote branch"
        );
    })
}

fn rollback_reconstruction_marker_boundary(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "rollback-reconstruct";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");
        h.ralph_ok(["rollback", "1"])
            .expect("soft rollback should succeed");

        let state_path = h.project_dir(project_id).join("state.json");
        fs::write(&state_path, "{ not valid json").expect("corrupt state");

        let output = h.ralph(["status"]).expect("status should execute");
        assert_exit_code(&output, 0);

        let recovered = h.load_state(project_id).expect("load recovered state");
        let loops = recovered["loops"].as_array().expect("loops should be array");
        assert_eq!(
            loops.len(),
            1,
            "marker boundary should clamp recovered state to loop 1"
        );
        assert_json_field(&recovered, "current_loop", &json!(1));
        assert_json_field(&recovered, "current_phase", &json!("planning"));
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
                .map_or(false, |c| c.is_ascii_digit()),
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

fn setup_remote_tracking(h: &RalphHarness, project_id: &str) {
    let origin = h.temp_dir.path().join("origin.git");
    git_ok(
        &h.repo_root,
        &["init", "--bare", origin.to_string_lossy().as_ref()],
    );
    git_ok(
        &h.repo_root,
        &["remote", "add", "origin", origin.to_string_lossy().as_ref()],
    );
    git_ok(&h.repo_root, &["push", "-u", "origin", "master"]);

    let project_branch = format!("ralph/{project_id}");
    git_ok(&h.repo_root, &["checkout", &project_branch]);
    git_ok(
        &h.repo_root,
        &["push", "-u", "origin", project_branch.as_str()],
    );
}

fn git_ok(repo_root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git command failed: git {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repo_root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git command failed: git {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
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
