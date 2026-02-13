use super::*;

use std::fs;
use std::path::PathBuf;

use crate::validate::assertions::{assert_exit_code, assert_stdout_contains};
use crate::validate::harness::RalphHarness;
use serde_json::{json, Value};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "daemon::cli_parse_start_status_abort",
            func: cli_parse_start_status_abort,
        },
        ConformanceTest {
            name: "daemon::config_merge_and_defaults",
            func: config_merge_and_defaults,
        },
        ConformanceTest {
            name: "daemon::start_validates_inputs_and_workspace",
            func: start_validates_inputs_and_workspace,
        },
        ConformanceTest {
            name: "daemon::status_reads_store_with_locking",
            func: status_reads_store_with_locking,
        },
        ConformanceTest {
            name: "daemon::abort_by_full_task_id",
            func: abort_by_full_task_id,
        },
        ConformanceTest {
            name: "daemon::abort_by_bare_number_ambiguous_error",
            func: abort_by_bare_number_ambiguous_error,
        },
        ConformanceTest {
            name: "daemon::abort_when_daemon_not_running",
            func: abort_when_daemon_not_running,
        },
        ConformanceTest {
            name: "daemon::abort_stale_pid_and_terminal_state_handling",
            func: abort_stale_pid_and_terminal_state_handling,
        },
    ]
}

fn cli_parse_start_status_abort(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let help = h
            .ralph(["daemon", "--help"])
            .expect("daemon --help should execute");
        assert_exit_code(&help, 0);
        assert_stdout_contains(&help, "start");
        assert_stdout_contains(&help, "status");
        assert_stdout_contains(&help, "abort");

        let start = h
            .ralph([
                "daemon",
                "start",
                "--repo",
                "acme/widgets",
                "--poll-seconds",
                "30",
                "--max-concurrent",
                "2",
                "--label",
                "ralph:ready",
                "--label",
                "triage",
            ])
            .expect("daemon start should execute");
        assert_exit_code(&start, 0);
        assert_stdout_contains(&start, "daemon start validated for repo acme/widgets");

        let status = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&status, 0);
        assert_stdout_contains(&status, "no daemon tasks");

        let abort = h
            .ralph(["daemon", "abort", "123"])
            .expect("daemon abort should execute");
        assert_exit_code(&abort, 2);
        assert!(
            combined_output(&abort).contains("no task found for issue number 123"),
            "expected missing task message, got:\n{}",
            combined_output(&abort)
        );
    })
}

fn config_merge_and_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let default_start = h
            .ralph(["daemon", "start", "--repo", "acme/default"])
            .expect("daemon start should execute");
        assert_exit_code(&default_start, 0);
        assert_stdout_contains(
            &default_start,
            "poll=60s, max_concurrent=1, labels=ralph:ready",
        );

        let poll_default = h
            .ralph_ok(["config", "get", "workspace.daemon_poll_seconds"])
            .expect("config get workspace.daemon_poll_seconds should succeed");
        assert_eq!(poll_default.trim(), "60");

        let conc_default = h
            .ralph_ok(["config", "get", "workspace.daemon_max_concurrent"])
            .expect("config get workspace.daemon_max_concurrent should succeed");
        assert_eq!(conc_default.trim(), "1");

        let labels_default = h
            .ralph_ok(["config", "get", "workspace.daemon_labels"])
            .expect("config get workspace.daemon_labels should succeed");
        assert_eq!(labels_default.trim(), "[\n  \"ralph:ready\"\n]");

        h.create_project(
            "daemon-config",
            "Daemon Config",
            "Project used for daemon config merge checks",
        )
        .expect("create_project failed");

        h.ralph_ok([
            "config",
            "set",
            "daemon.poll_seconds",
            "15",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.poll_seconds failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.max_concurrent",
            "3",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.max_concurrent failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.labels",
            "[\"ralph:ready\",\"priority:high\"]",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.labels failed");
        h.ralph_ok([
            "config",
            "set",
            "daemon.repo",
            "acme/project-override",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.repo failed");

        h.ralph_ok(["project", "use", "daemon-config"])
            .expect("project use should succeed");

        let merged_start = h
            .ralph(["daemon", "start"])
            .expect("daemon start should execute");
        assert_exit_code(&merged_start, 0);
        assert_stdout_contains(
            &merged_start,
            "daemon start validated for repo acme/project-override",
        );
        assert_stdout_contains(
            &merged_start,
            "poll=15s, max_concurrent=3, labels=ralph:ready,priority:high",
        );
    })
}

fn start_validates_inputs_and_workspace(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let no_workspace = h
            .ralph(["daemon", "start", "--repo", "acme/widgets"])
            .expect("daemon start should execute");
        assert_exit_code(&no_workspace, 2);

        h.init_workspace().expect("init failed");

        let gh_path = write_mock_gh(h, "#!/bin/sh\necho \"octo/demo\"\n")
            .expect("write mock gh should succeed");

        let with_workspace = h
            .ralph_env(["daemon", "start"], &[("PATH", &gh_path)])
            .expect("daemon start should execute");
        assert_exit_code(&with_workspace, 0);
        assert_stdout_contains(&with_workspace, "daemon start validated for repo octo/demo");
    })
}

fn status_reads_store_with_locking(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let empty = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&empty, 0);
        assert_stdout_contains(&empty, "no daemon tasks");

        write_tasks(
            h,
            vec![
                task_json("acme-widgets-41", "pending", 41, "acme", "widgets", None, None),
                task_json(
                    "acme-widgets-42",
                    "in_progress",
                    42,
                    "acme",
                    "widgets",
                    Some(2222),
                    Some(2222),
                ),
            ],
        )
        .expect("write_tasks failed");

        let populated = h
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&populated, 0);
        assert_stdout_contains(&populated, "acme-widgets-41");
        assert_stdout_contains(&populated, "pending");
        assert_stdout_contains(&populated, "acme-widgets-42");
        assert_stdout_contains(&populated, "in_progress");
    })
}

fn abort_by_full_task_id(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-10",
                "in_progress",
                10,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph(["daemon", "abort", "acme-widgets-10"])
            .expect("daemon abort should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "aborted task acme-widgets-10");

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["state"], json!("aborted"));
    })
}

fn abort_by_bare_number_ambiguous_error(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![
                task_json("acme-widgets-7", "pending", 7, "acme", "widgets", None, None),
                task_json("other-api-7", "pending", 7, "other", "api", None, None),
            ],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph(["daemon", "abort", "7"])
            .expect("daemon abort should execute");
        assert_exit_code(&output, 2);

        let combined = combined_output(&output);
        assert!(
            combined.contains("ambiguous"),
            "expected ambiguous error, got:\n{combined}"
        );

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks[0]["state"], json!("pending"));
        assert_eq!(tasks[1]["state"], json!("pending"));
    })
}

fn abort_when_daemon_not_running(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-99",
                "pending",
                99,
                "acme",
                "widgets",
                None,
                None,
            )],
        )
        .expect("write_tasks failed");

        let output = h
            .ralph(["daemon", "abort", "99"])
            .expect("daemon abort should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "aborted task acme-widgets-99");

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks[0]["state"], json!("aborted"));
    })
}

fn abort_stale_pid_and_terminal_state_handling(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        write_tasks(
            h,
            vec![task_json(
                "acme-widgets-55",
                "in_progress",
                55,
                "acme",
                "widgets",
                Some(999_999),
                Some(999_999),
            )],
        )
        .expect("write_tasks failed");

        let first_abort = h
            .ralph(["daemon", "abort", "acme-widgets-55"])
            .expect("daemon abort should execute");
        assert_exit_code(&first_abort, 0);

        let tasks = load_tasks(h).expect("load_tasks failed");
        assert_eq!(tasks[0]["state"], json!("aborted"));

        let second_abort = h
            .ralph(["daemon", "abort", "acme-widgets-55"])
            .expect("daemon abort should execute");
        assert_exit_code(&second_abort, 2);

        let combined = combined_output(&second_abort);
        assert!(
            combined.contains("already terminal"),
            "expected terminal-state error, got:\n{combined}"
        );
    })
}

fn write_tasks(h: &RalphHarness, tasks: Vec<Value>) -> crate::Result<()> {
    let path = tasks_path(h);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(&tasks)?)?;
    Ok(())
}

fn load_tasks(h: &RalphHarness) -> crate::Result<Vec<Value>> {
    let raw = fs::read_to_string(tasks_path(h))?;
    Ok(serde_json::from_str(&raw)?)
}

fn tasks_path(h: &RalphHarness) -> PathBuf {
    h.repo_root.join(".ralph").join("daemon").join("tasks.json")
}

fn task_json(
    task_id: &str,
    state: &str,
    issue_number: u32,
    owner: &str,
    repo: &str,
    child_pid: Option<u32>,
    child_pgid: Option<u32>,
) -> Value {
    json!({
        "task_id": task_id,
        "state": state,
        "issue_number": issue_number,
        "owner": owner,
        "repo": repo,
        "child_pid": child_pid,
        "child_pgid": child_pgid,
        "branch": null,
        "pr_url": null,
        "created_at": "2026-02-13T00:00:00Z",
        "updated_at": "2026-02-13T00:00:00Z"
    })
}

fn write_mock_gh(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("gh", body)?;
    let base = script
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    Ok(format!("{base}:{existing}"))
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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
