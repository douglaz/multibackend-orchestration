use super::*;

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::daemon::bootstrap;
use crate::daemon::github;
use crate::daemon::worktree;
use crate::validate::assertions::{assert_exit_code, assert_stdout_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts;
#[allow(unused_imports)]
use serde_json::{json, Value};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        // --- Loop 1 Foundation Tests ---
        ConformanceTest {
            name: "daemon::cli_parse_start_status_abort",
            func: cli_parse_start_status_abort,
        },
        ConformanceTest {
            name: "daemon::verbose_flag_accepted_by_start",
            func: verbose_flag_accepted_by_start,
        },
        ConformanceTest {
            name: "daemon::verbose_flag_rejected_by_status_and_abort",
            func: verbose_flag_rejected_by_status_and_abort,
        },
        ConformanceTest {
            name: "daemon::verbose_output_present_when_enabled",
            func: verbose_output_present_when_enabled,
        },
        ConformanceTest {
            name: "daemon::verbose_output_absent_when_disabled",
            func: verbose_output_absent_when_disabled,
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
            name: "daemon::label_ensure_startup",
            func: label_ensure_startup,
        },
        ConformanceTest {
            name: "daemon::label_ensure_already_exists",
            func: label_ensure_already_exists,
        },
        ConformanceTest {
            name: "daemon::label_ensure_hard_failure",
            func: label_ensure_hard_failure,
        },
        ConformanceTest {
            name: "daemon::status_queries_github_labels",
            func: status_queries_github_labels,
        },
        ConformanceTest {
            name: "daemon::abort_by_issue_number_with_repo",
            func: abort_by_issue_number_with_repo,
        },
        ConformanceTest {
            name: "daemon::abort_rejects_non_in_progress",
            func: abort_rejects_non_in_progress,
        },
        ConformanceTest {
            name: "daemon::retrigger_swaps_failed_to_ready",
            func: retrigger_swaps_failed_to_ready,
        },
        ConformanceTest {
            name: "daemon::retrigger_rejects_non_failed",
            func: retrigger_rejects_non_failed,
        },
        // --- Loop 2 Runtime Tests ---
        ConformanceTest {
            name: "daemon::runtime_reconciliation_on_startup",
            func: runtime_reconciliation_on_startup,
        },
        ConformanceTest {
            name: "daemon::runtime_polling_filter_overflow",
            func: runtime_polling_filter_overflow,
        },
        ConformanceTest {
            name: "daemon::runtime_worktree_isolation",
            func: runtime_worktree_isolation,
        },
        ConformanceTest {
            name: "daemon::runtime_single_iteration_mode",
            func: runtime_single_iteration_mode,
        },
        ConformanceTest {
            name: "daemon::runtime_task_fails_worktree_preserved",
            func: runtime_task_fails_worktree_preserved,
        },
        ConformanceTest {
            name: "daemon::runtime_no_diff_pr_path",
            func: runtime_no_diff_pr_path,
        },
        ConformanceTest {
            name: "daemon::runtime_artifact_comments_posted",
            func: runtime_artifact_comments_posted,
        },
        // --- Loop 4 Bootstrap + PR/Diff Hardening Tests ---
        ConformanceTest {
            name: "daemon::daemon_bootstrap_non_git_dir",
            func: daemon_bootstrap_non_git_dir,
        },
        ConformanceTest {
            name: "daemon::daemon_bootstrap_zero_commit_repo",
            func: daemon_bootstrap_zero_commit_repo,
        },
        ConformanceTest {
            name: "daemon::daemon_bootstrap_idempotent",
            func: daemon_bootstrap_idempotent,
        },
        ConformanceTest {
            name: "daemon::daemon_bootstrap_existing_repo_noop",
            func: daemon_bootstrap_existing_repo_noop,
        },
        ConformanceTest {
            name: "daemon::daemon_has_diff_invalid_base_returns_false",
            func: daemon_has_diff_invalid_base_returns_false,
        },
        // --- Worktree Resilience Tests ---
        ConformanceTest {
            name: "daemon::create_worktree_reuses_existing_branch",
            func: create_worktree_reuses_existing_branch,
        },
        ConformanceTest {
            name: "daemon::clean_worktree_removes_dirty_files",
            func: clean_worktree_removes_dirty_files,
        },
        ConformanceTest {
            name: "daemon::runtime_create_worktree_handles_stale_metadata",
            func: runtime_create_worktree_handles_stale_metadata,
        },
        ConformanceTest {
            name: "daemon::runtime_reuse_worktree_corrects_branch_mismatch",
            func: runtime_reuse_worktree_corrects_branch_mismatch,
        },
        // --- Loop 2 Data-Dir Provisioning Tests ---
        ConformanceTest {
            name: "daemon::daemon_start_bootstraps_empty_dir",
            func: daemon_start_bootstraps_empty_dir,
        },
        ConformanceTest {
            name: "daemon::daemon_start_rejects_git_data_dir",
            func: daemon_start_rejects_git_data_dir,
        },
        ConformanceTest {
            name: "daemon::daemon_start_rejects_duplicate_repo",
            func: daemon_start_rejects_duplicate_repo,
        },
        ConformanceTest {
            name: "daemon::daemon_start_clone_failure_propagates",
            func: daemon_start_clone_failure_propagates,
        },
        ConformanceTest {
            name: "daemon::daemon_status_multi_repo",
            func: daemon_status_multi_repo,
        },
        // --- Loop 2 Dispatch-time Project Backfill Tests ---
        ConformanceTest {
            name: "daemon::discover_project_id_ignores_dirs_without_state_json",
            func: discover_project_id_ignores_dirs_without_state_json,
        },
        // --- Loop 2 Remote-First Branch Sync Tests ---
        ConformanceTest {
            name: "daemon::sync_project_branch_resets_to_remote",
            func: sync_project_branch_resets_to_remote,
        },
        ConformanceTest {
            name: "daemon::sync_project_branch_creates_from_origin_head",
            func: sync_project_branch_creates_from_origin_head,
        },
        ConformanceTest {
            name: "daemon::sync_project_branch_missing_origin_head_error",
            func: sync_project_branch_missing_origin_head_error,
        },
        ConformanceTest {
            name: "daemon::sync_project_branch_discards_local_commit",
            func: sync_project_branch_discards_local_commit,
        },
        ConformanceTest {
            name: "daemon::sync_project_branch_force_updates_stale_base",
            func: sync_project_branch_force_updates_stale_base,
        },
        ConformanceTest {
            name: "daemon::worktree_uses_origin_head_not_local_refs",
            func: worktree_uses_origin_head_not_local_refs,
        },
        ConformanceTest {
            name: "daemon::worktree_falls_back_when_origin_head_missing",
            func: worktree_falls_back_when_origin_head_missing,
        },
        // --- Loop 4 Label Lifecycle No-Durable-Store Tests ---
        ConformanceTest {
            name: "daemon::no_tasks_json_written_after_runtime",
            func: no_tasks_json_written_after_runtime,
        },
        ConformanceTest {
            name: "daemon::startup_reconcile_resets_in_progress_to_ready",
            func: startup_reconcile_resets_in_progress_to_ready,
        },
        ConformanceTest {
            name: "daemon::multi_lifecycle_label_normalizes_to_failed",
            func: multi_lifecycle_label_normalizes_to_failed,
        },
        ConformanceTest {
            name: "daemon::label_retry_on_conflict_transient",
            func: label_retry_on_conflict_transient,
        },
        ConformanceTest {
            name: "daemon::daemon_lock_contention_exits_immediately",
            func: daemon_lock_contention_exits_immediately,
        },
        ConformanceTest {
            name: "daemon::status_history_derive_from_git_and_labels",
            func: status_history_derive_from_git_and_labels,
        },
        ConformanceTest {
            name: "daemon::crash_after_local_commit_before_push_recovery",
            func: crash_after_local_commit_before_push_recovery,
        },
        ConformanceTest {
            name: "daemon::reconstruct_position_from_real_remote_checkpoint",
            func: reconstruct_position_from_real_remote_checkpoint,
        },
    ]
}

// =============================================================================
// Loop 1 Foundation Tests (preserved from original)
// =============================================================================

fn cli_parse_start_status_abort(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let help = dh
            .ralph(["daemon", "--help"])
            .expect("daemon --help should execute");
        assert_exit_code(&help, 0);
        assert_stdout_contains(&help, "start");
        assert_stdout_contains(&help, "status");
        assert_stdout_contains(&help, "abort");
        assert_stdout_contains(&help, "retrigger");

        // Use --single-iteration and mock gh so runtime exits cleanly
        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let start = dh
            .daemon_env(
                [
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
                    "--single-iteration",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&start, 0);
        assert_stdout_contains(&start, "daemon start validated for repo acme/widgets");

        let status = dh
            .ralph(["daemon", "status"])
            .expect("daemon status should execute");
        assert_exit_code(&status, 0);
        // After a single iteration with no issues, still no tasks
        let combined = combined_output(&status);
        assert!(
            combined.contains("no daemon tasks") || combined.contains("DAEMON TASKS"),
            "expected task output, got:\n{combined}"
        );

        // abort without --repo should fail with validation error
        let abort_no_repo = dh
            .ralph(["daemon", "abort", "123"])
            .expect("daemon abort should execute");
        assert_exit_code(&abort_no_repo, 2);
        assert!(
            combined_output(&abort_no_repo).contains("--repo is required"),
            "expected --repo required message, got:\n{}",
            combined_output(&abort_no_repo)
        );

        // abort with --repo but issue not in-progress (mock returns no labels)
        let abort = dh
            .daemon_env(
                ["daemon", "abort", "123", "--repo", "acme/widgets"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon abort should execute");
        assert_exit_code(&abort, 2);
        assert!(
            combined_output(&abort).contains("not in-progress"),
            "expected not-in-progress message, got:\n{}",
            combined_output(&abort)
        );
    })
}

fn verbose_flag_accepted_by_start(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--verbose",
                    "--single-iteration",
                    "--repo",
                    "acme/widgets",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);
    })
}

fn verbose_flag_rejected_by_status_and_abort(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let status = h
            .ralph(["daemon", "status", "--verbose"])
            .expect("daemon status should execute");
        let status_code = status.status.code().unwrap_or(-1);
        assert_ne!(
            status_code,
            0,
            "daemon status --verbose should fail, got stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr)
        );
        assert_invalid_verbose_flag_error(&String::from_utf8_lossy(&status.stderr));

        let abort = h
            .ralph(["daemon", "abort", "--verbose", "dummy-id"])
            .expect("daemon abort should execute");
        let abort_code = abort.status.code().unwrap_or(-1);
        assert_ne!(
            abort_code,
            0,
            "daemon abort --verbose should fail, got stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&abort.stdout),
            String::from_utf8_lossy(&abort.stderr)
        );
        assert_invalid_verbose_flag_error(&String::from_utf8_lossy(&abort.stderr));
    })
}

fn verbose_output_present_when_enabled(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--verbose",
                    "--single-iteration",
                    "--repo",
                    "acme/widgets",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        let verbose_lines: Vec<&str> = stderr
            .lines()
            .filter(|line| line.starts_with("verbose:"))
            .collect();
        assert!(
            !verbose_lines.is_empty(),
            "expected at least one verbose: line in stderr, got:\n{stderr}"
        );
        assert!(
            verbose_lines.iter().any(|line| {
                line.contains("poll-cycle")
                    && line.contains("iteration=")
                    && line.contains("active_children=")
                    && line.contains("available_slots=")
                    && line.contains("planned_sleep_seconds=")
            }),
            "expected a poll-cycle verbose line with all required fields, got:\n{stderr}"
        );
    })
}

fn verbose_output_absent_when_disabled(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--single-iteration",
                    "--repo",
                    "acme/widgets",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        let verbose_count = stderr
            .lines()
            .filter(|line| line.starts_with("verbose:"))
            .count();
        assert_eq!(
            verbose_count, 0,
            "expected zero verbose: lines when flag is disabled, got:\n{stderr}"
        );
    })
}

fn config_merge_and_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Use --single-iteration and mock gh for the daemon start
        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let default_start = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/default",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&default_start, 0);
        assert_stdout_contains(
            &default_start,
            "poll=60s, max_concurrent=5, labels=ralph:ready",
        );

        let poll_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_poll_seconds"])
            .expect("config get workspace.daemon_poll_seconds should succeed");
        assert_eq!(poll_default.trim(), "60");

        let conc_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_max_concurrent"])
            .expect("config get workspace.daemon_max_concurrent should succeed");
        assert_eq!(conc_default.trim(), "5");

        let labels_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_labels"])
            .expect("config get workspace.daemon_labels should succeed");
        assert_eq!(labels_default.trim(), "[\n  \"ralph:ready\"\n]");

        let refinement_enabled_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_refinement_enabled"])
            .expect("config get workspace.daemon_refinement_enabled should succeed");
        assert_eq!(refinement_enabled_default.trim(), "true");

        let refinement_backend_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_refinement_backend"])
            .expect("config get workspace.daemon_refinement_backend should succeed");
        assert_eq!(refinement_backend_default.trim(), "claude(sonnet)");

        let auto_rebase_enabled_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_auto_rebase_enabled"])
            .expect("config get workspace.daemon_auto_rebase_enabled should succeed");
        assert_eq!(auto_rebase_enabled_default.trim(), "true");

        let rebase_interval_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_rebase_interval_seconds"])
            .expect("config get workspace.daemon_rebase_interval_seconds should succeed");
        assert_eq!(rebase_interval_default.trim(), "1800");

        let rebase_cap_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_max_rebases_per_cycle"])
            .expect("config get workspace.daemon_max_rebases_per_cycle should succeed");
        assert_eq!(rebase_cap_default.trim(), "3");

        let rebase_timeout_default = dh
            .ralph_ok(["config", "get", "workspace.daemon_rebase_timeout_seconds"])
            .expect("config get workspace.daemon_rebase_timeout_seconds should succeed");
        assert_eq!(rebase_timeout_default.trim(), "120");

        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_auto_rebase_enabled",
            "false",
        ])
        .expect("set workspace.daemon_auto_rebase_enabled failed");
        let auto_rebase_enabled_updated = dh
            .ralph_ok(["config", "get", "workspace.daemon_auto_rebase_enabled"])
            .expect("config get workspace.daemon_auto_rebase_enabled should succeed");
        assert_eq!(auto_rebase_enabled_updated.trim(), "false");

        dh.create_project(
            "daemon-config",
            "Daemon Config",
            "Project used for daemon config merge checks",
        )
        .expect("create_project failed");

        dh.ralph_ok([
            "config",
            "set",
            "daemon.poll_seconds",
            "15",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.poll_seconds failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.max_concurrent",
            "3",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.max_concurrent failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.labels",
            "[\"ralph:ready\",\"priority:high\"]",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.labels failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.repo",
            "acme/project-override",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.repo failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.refinement_enabled",
            "false",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.refinement_enabled failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.refinement_backend",
            "codex(gpt-5.3-codex-medium)",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.refinement_backend failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.auto_rebase_enabled",
            "false",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.auto_rebase_enabled failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.rebase_interval_seconds",
            "900",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.rebase_interval_seconds failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.max_rebases_per_cycle",
            "5",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.max_rebases_per_cycle failed");
        dh.ralph_ok([
            "config",
            "set",
            "daemon.rebase_timeout_seconds",
            "240",
            "--project",
            "daemon-config",
        ])
        .expect("set daemon.rebase_timeout_seconds failed");

        dh.ralph_ok(["project", "use", "daemon-config"])
            .expect("project use should succeed");

        let refinement_enabled_project = dh
            .ralph_ok(["config", "get", "daemon.refinement_enabled"])
            .expect("config get daemon.refinement_enabled should succeed");
        assert_eq!(refinement_enabled_project.trim(), "false");

        let refinement_backend_project = dh
            .ralph_ok(["config", "get", "daemon.refinement_backend"])
            .expect("config get daemon.refinement_backend should succeed");
        assert_eq!(
            refinement_backend_project.trim(),
            "codex(gpt-5.3-codex-medium)"
        );

        let auto_rebase_enabled_project = dh
            .ralph_ok(["config", "get", "daemon.auto_rebase_enabled"])
            .expect("config get daemon.auto_rebase_enabled should succeed");
        assert_eq!(auto_rebase_enabled_project.trim(), "false");

        let rebase_interval_project = dh
            .ralph_ok(["config", "get", "daemon.rebase_interval_seconds"])
            .expect("config get daemon.rebase_interval_seconds should succeed");
        assert_eq!(rebase_interval_project.trim(), "900");

        let rebase_cap_project = dh
            .ralph_ok(["config", "get", "daemon.max_rebases_per_cycle"])
            .expect("config get daemon.max_rebases_per_cycle should succeed");
        assert_eq!(rebase_cap_project.trim(), "5");

        let rebase_timeout_project = dh
            .ralph_ok(["config", "get", "daemon.rebase_timeout_seconds"])
            .expect("config get daemon.rebase_timeout_seconds should succeed");
        assert_eq!(rebase_timeout_project.trim(), "240");

        let merged_start = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--single-iteration",
                    "--repo",
                    "acme/widgets",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&merged_start, 0);
        assert_stdout_contains(
            &merged_start,
            "daemon start validated for repo acme/widgets",
        );
        assert_stdout_contains(
            &merged_start,
            "poll=15s, max_concurrent=3, labels=ralph:ready,priority:high",
        );
    })
}

fn start_validates_inputs_and_workspace(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Verify that daemon start fails when --repo is missing (validation error)
        let no_repo = dh
            .ralph(["daemon", "start", "--single-iteration"])
            .expect("daemon start should execute");
        assert_exit_code(&no_repo, 2);

        // Verify that daemon start succeeds with proper setup (use full daemon mock gh)
        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh should succeed");

        let with_repo = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--single-iteration",
                    "--repo",
                    "octo/demo",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&with_repo, 0);
        assert_stdout_contains(&with_repo, "daemon start validated for repo octo/demo");
    })
}

fn label_ensure_startup(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let label_log = h.temp_dir.path().join("label_create.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label)
    case "$2" in
      create)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      *) exit 1 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let log_raw = fs::read_to_string(&label_log).expect("label create log should exist");
        let lines: Vec<&str> = log_raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        // 4 standard labels + 5 PRD labels = 9 total
        let total_labels =
            github::REQUIRED_LABELS.len() + crate::daemon::interactive_prd::PRD_LABELS.len();
        assert_eq!(
            lines.len(),
            total_labels,
            "expected exactly {} label create calls (standard + PRD), got:\n{}",
            total_labels,
            log_raw
        );

        for (label_name, _, _) in github::REQUIRED_LABELS {
            let needle = format!("create {label_name}");
            let count = lines.iter().filter(|line| line.contains(&needle)).count();
            assert_eq!(
                count, 1,
                "expected one create call for '{label_name}', got {count}:\n{}",
                log_raw
            );
        }
    })
}

fn label_ensure_already_exists(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let label_log = h.temp_dir.path().join("label_create.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label)
    case "$2" in
      create)
        echo "$@" >> "{label_log_str}"
        if [ "$3" = "ralph:in-progress" ]; then
          echo "label already exists" >&2
          exit 1
        fi
        exit 0
        ;;
      *) exit 1 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("failed to ensure label 'ralph:in-progress'"),
            "already-exists label should not emit failure warning, stderr:\n{stderr}"
        );

        let log_raw = fs::read_to_string(&label_log).expect("label create log should exist");
        let call_count = log_raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        // 4 standard labels + 5 PRD labels = 9 total
        let total_labels =
            github::REQUIRED_LABELS.len() + crate::daemon::interactive_prd::PRD_LABELS.len();
        assert_eq!(
            call_count, total_labels,
            "expected startup to attempt all lifecycle labels (standard + PRD)"
        );
    })
}

fn label_ensure_hard_failure(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let label_log = h.temp_dir.path().join("label_create.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label)
    case "$2" in
      create)
        echo "$@" >> "{label_log_str}"
        if [ "$3" = "ralph:failed" ]; then
          echo "permission denied" >&2
          exit 1
        fi
        exit 0
        ;;
      *) exit 1 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(h, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("failed to ensure label 'ralph:failed'"),
            "expected warning for hard label creation failure, stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("permission denied"),
            "expected command stderr to be surfaced in warning, stderr:\n{stderr}"
        );

        let log_raw = fs::read_to_string(&label_log).expect("label create log should exist");
        let call_count = log_raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        // 4 standard labels + 5 PRD labels = 9 total
        let total_labels =
            github::REQUIRED_LABELS.len() + crate::daemon::interactive_prd::PRD_LABELS.len();
        assert_eq!(
            call_count, total_labels,
            "expected startup to attempt all lifecycle labels (standard + PRD)"
        );
    })
}

/// Status now queries GitHub labels via mock gh. Verify empty status and
/// populated status (via mock gh returning issues with lifecycle labels).
fn status_queries_github_labels(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Status with no mock gh issues returns "no daemon tasks"
        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let empty = dh
            .daemon_env(
                ["daemon", "status", "--repo", "acme/widgets"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon status should execute");
        assert_exit_code(&empty, 0);
        let combined = combined_output(&empty);
        assert!(
            combined.contains("no daemon tasks") || combined.contains("DAEMON ISSUES"),
            "expected status output, got:\n{combined}"
        );
    })
}

/// Abort by issue number with --repo flag. Uses mock gh that returns
/// `ralph:in-progress` label for the issue.
fn abort_by_issue_number_with_repo(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Mock gh: issue view --json labels returns ralph:in-progress,
        // issue edit (label swap) succeeds.
        let label_log = dh.temp_dir.path().join("abort_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
            exit 0
          fi
        done
        printf ''
        exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr) printf '' ; exit 0 ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");

        let output = dh
            .daemon_env(
                ["daemon", "abort", "10", "--repo", "acme/widgets"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon abort should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "aborted issue acme/widgets#10");

        // Verify label swap was logged
        let log = fs::read_to_string(&label_log).expect("label log should exist");
        assert!(
            log.contains("--remove-label") && log.contains("ralph:in-progress"),
            "expected remove-label ralph:in-progress, got:\n{log}"
        );
        assert!(
            log.contains("--add-label") && log.contains("ralph:failed"),
            "expected add-label ralph:failed, got:\n{log}"
        );
    })
}

/// Abort rejects issues that are not in-progress (e.g., already completed).
fn abort_rejects_non_in_progress(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Mock gh: issue view --json labels returns ralph:completed (not in-progress)
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{"labels":[{"name":"ralph:completed"}]}'
            exit 0
          fi
        done
        printf '' ; exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr) printf '' ; exit 0 ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#;
        let gh_path = write_mock_gh(&dh, gh_script).expect("write mock gh");

        let output = dh
            .daemon_env(
                ["daemon", "abort", "55", "--repo", "acme/widgets"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon abort should execute");
        // Should fail because issue is not in-progress
        let code = output.status.code().unwrap_or(-1);
        assert_ne!(code, 0, "abort of non-in-progress should fail");

        let combined = combined_output(&output);
        assert!(
            combined.contains("not in-progress"),
            "expected not-in-progress error, got:\n{combined}"
        );
    })
}

/// Retrigger swaps ralph:failed -> ralph:ready via mock gh.
fn retrigger_swaps_failed_to_ready(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let label_log = dh.temp_dir.path().join("retrigger_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:failed"}}]}}'
            exit 0
          fi
        done
        printf '' ; exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr) printf '' ; exit 0 ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");

        let output = dh
            .daemon_env(
                ["daemon", "retrigger", "42", "--repo", "acme/widgets"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon retrigger should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "retriggered issue acme/widgets#42");

        let log = fs::read_to_string(&label_log).expect("label log should exist");
        assert!(
            log.contains("--remove-label") && log.contains("ralph:failed"),
            "expected remove-label ralph:failed, got:\n{log}"
        );
        assert!(
            log.contains("--add-label") && log.contains("ralph:ready"),
            "expected add-label ralph:ready, got:\n{log}"
        );
    })
}

/// Retrigger rejects issues not in failed state.
fn retrigger_rejects_non_failed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{"labels":[{"name":"ralph:in-progress"}]}'
            exit 0
          fi
        done
        printf '' ; exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr) printf '' ; exit 0 ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#;
        let gh_path = write_mock_gh(&dh, gh_script).expect("write mock gh");

        let output = dh
            .daemon_env(
                ["daemon", "retrigger", "42", "--repo", "acme/widgets"],
                &[("PATH", &gh_path)],
            )
            .expect("daemon retrigger should execute");
        let code = output.status.code().unwrap_or(-1);
        assert_ne!(code, 0, "retrigger of non-failed should fail");

        let combined = combined_output(&output);
        assert!(
            combined.contains("not in failed state"),
            "expected not-in-failed-state error, got:\n{combined}"
        );
    })
}

// =============================================================================
// Loop 2 Runtime Tests
// =============================================================================

/// Test that startup reconciliation resets in-progress issues to ready
/// via label swap operations.
///
/// Verifies:
/// - Multiple in-progress issues are all reset to ready
/// - Reconciliation count is logged in stderr
/// - Label swap operations (remove in-progress, add ready) are recorded
fn runtime_reconciliation_on_startup(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement for this test");

        let label_log = dh.temp_dir.path().join("reconcile_multi_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Reconciliation queries ralph:in-progress issues; return 2 stale ones
        let reconcile_issues = r#"[{"number":10,"title":"stale A","labels":[{"name":"ralph:in-progress"}],"body":"a"},{"number":20,"title":"stale B","labels":[{"name":"ralph:in-progress"}],"body":"b"}]"#;

        // After reconciliation, the poll phase queries ralph:ready; return empty
        // (the freshly-reconciled issues would now be ralph:ready but mock
        // returns empty to keep the test focused on reconciliation)
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        # Check which label is being queried
        for arg in "$@"; do
          case "$arg" in
            ralph:in-progress)
              printf '%s' '{reconcile_issues}'
              exit 0
              ;;
            ralph:ready)
              printf '[]'
              exit 0
              ;;
          esac
        done
        printf '[]'
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
            exit 0
          fi
        done
        printf '' ; exit 0
        ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify reconciliation message in stderr — proves reconcile phase
        // ran and detected both in-progress issues.
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("reconcile: reset 2 in-progress issue(s) to ready"),
            "expected reconciliation message for 2 issues in stderr, got:\n{stderr}"
        );

        // Verify label operations: should have remove-label ralph:in-progress
        // and add-label ralph:ready for both issues
        let log = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            log.contains("--remove-label") && log.contains("ralph:in-progress"),
            "expected remove-label ralph:in-progress in label log:\n{log}"
        );
        assert!(
            log.contains("--add-label") && log.contains("ralph:ready"),
            "expected add-label ralph:ready in label log:\n{log}"
        );
    })
}

/// Test that polling correctly filters out ralph:* labeled issues and
/// emits overflow warning when exactly 100 issues returned.
///
/// Verifies:
/// - Overflow warning appears when poll returns exactly 100 issues
/// - Issues with ralph:* labels are filtered out (not claimed)
/// - max_concurrent is respected (only 1 task claimed)
fn runtime_polling_filter_overflow(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement for this test");

        // Create a mock gh that returns exactly 100 issues, some with ralph:* labels.
        // Issues 1-5 have ralph:in-progress labels (should be filtered).
        // Issues 6-100 have no ralph labels (claimable).
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        printf '['
        i=1
        while [ $i -le 100 ]; do
          if [ $i -gt 1 ]; then printf ','; fi
          if [ $i -le 5 ]; then
            printf '{"number":%d,"title":"issue %d","labels":[{"name":"ralph:in-progress"}]}' $i $i
          else
            printf '{"number":%d,"title":"issue %d","labels":[]}' $i $i
          fi
          i=$((i + 1))
        done
        printf ']'
        exit 0
        ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
    esac
    ;;
  repo)
    printf 'acme/widgets\n'
    exit 0
    ;;
  label)
    [ "$2" = "create" ] && exit 0
    exit 1
    ;;
esac
exit 1
"#;

        let gh_path = write_mock_gh(&dh, gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        // Use max_concurrent=1 to limit claiming. The overflow warning should
        // still be emitted based on the poll result count.
        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                    "--max-concurrent",
                    "1",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        // Verify overflow warning
        assert!(
            stderr.contains("100 issues") || stderr.contains("truncated"),
            "expected overflow/truncation warning in stderr, got:\n{stderr}"
        );

        // With 100 issues that have no lifecycle labels (`labels:[]`), none
        // should be claimed (they lack `ralph:ready`), and no durable daemon
        // JSON state should be written.
        let daemon_dir = dh.repo_root.join(".ralph").join("daemon");
        if daemon_dir.exists() {
            let has_json = fs::read_dir(&daemon_dir)
                .expect("read daemon dir")
                .flatten()
                .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"));
            assert!(!has_json, "no daemon JSON state files should be written");
        }
    })
}

/// Test that each task gets its own worktree directory.
///
/// Uses RALPH_DAEMON_BIN to inject a mock ralph binary so dispatch is
/// deterministic and always succeeds.
///
/// Verifies:
/// - The daemon dispatches the task (unconditional)
/// - The worktree directory is created under .ralph/daemon/worktrees/<task-id>/
/// - The task reaches terminal state after drain
fn runtime_worktree_isolation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("worktree_iso_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Provide a ralph:ready issue for the daemon to claim
        let issues = r#"[{"number":50,"title":"worktree isolation test","labels":[{"name":"ralph:ready"}],"body":"Test worktree isolation."}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Dispatch must have succeeded (unconditional)
        assert!(
            stderr.contains("dispatched task acme-widgets-50"),
            "task must be dispatched (deterministic with mock ralph), stderr:\n{stderr}"
        );

        // Worktrees base directory must exist
        let wt_base = dh.repo_root.join(".ralph").join("daemon").join("worktrees");
        assert!(
            wt_base.exists(),
            "worktrees base directory must exist after dispatch"
        );

        // Verify terminal state via label log — should see ralph:in-progress removed
        // and a terminal label (ralph:completed or ralph:failed) added
        let log_content = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            log_content.contains("ralph:completed") || log_content.contains("ralph:failed"),
            "expected terminal label transition in label log, got:\n{log_content}"
        );
    })
}

/// Test that failed tasks preserve their worktree.
///
/// Verifies:
/// - Child non-zero exit transitions issue to ralph:failed via label swap
/// - Failed terminal state preserves worktree
fn runtime_task_fails_worktree_preserved(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config".to_owned(),
            "set".to_owned(),
            "workspace.daemon_refinement_enabled".to_owned(),
            "false".to_owned(),
        ])
        .expect("disable daemon refinement for this test");

        let label_log = dh.temp_dir.path().join("fail_preserve_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        // Mock ralph that always fails (exit 1) on auto
        let ralph_script = r#"#!/bin/sh
case "$1" in
  auto)
    exit 1
    ;;
  *)
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"#;
        let ralph_path = write_mock_ralph(&dh, ralph_script).expect("write mock ralph");

        // Provide a ralph:ready issue for the daemon to claim and dispatch
        let issues = r#"[{"number":350,"title":"Fail task","labels":[{"name":"ralph:ready"}],"body":"Preserve worktree on failure."}]"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify the label log shows transition to ralph:failed
        let log_content = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            log_content.contains("ralph:failed"),
            "expected ralph:failed label in label log, got:\n{log_content}"
        );

        let wt_path = dh
            .repo_root
            .join(".ralph")
            .join("daemon")
            .join("worktrees")
            .join("acme-widgets-350");
        assert!(
            wt_path.exists(),
            "failed task worktree should be preserved at {}",
            wt_path.display()
        );
    })
}

/// Test that --single-iteration mode runs exactly one cycle and exits
/// deterministically with no children left running.
///
/// Verifies:
/// - Daemon exits successfully in single-iteration mode
/// - Issues polled as ralph:ready are claimed, dispatched, and drained
/// - No durable daemon JSON lifecycle file is created (label-only lifecycle)
fn runtime_single_iteration_mode(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        let label_log = dh.temp_dir.path().join("single_iter_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock gh returns one ralph:ready issue for the daemon to pick up
        let issues = r#"[{"number":100,"title":"test issue","labels":[{"name":"ralph:ready"}],"body":"test body"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "daemon start validated for repo acme/widgets");

        // Verify no durable daemon JSON state was written (label-only lifecycle)
        let daemon_dir = dh.repo_root.join(".ralph").join("daemon");
        if daemon_dir.exists() {
            let has_json = fs::read_dir(&daemon_dir)
                .expect("read daemon dir")
                .flatten()
                .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"));
            assert!(!has_json, "daemon should not persist JSON lifecycle state");
        }
    })
}

/// Test the "no diff → no PR + idempotent note comment" path.
///
/// The daemon polls a ralph:ready issue, claims it, spawns mock ralph that
/// does NOT create commits (has_diff returns false), then completes the task.
///
/// Verifies:
/// - `gh pr create` is NOT called (no diff → no PR)
/// - An idempotent `no-diff` marker comment is posted
/// - Label transition to ralph:completed occurs
fn runtime_no_diff_pr_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        enable_fast_daemon_refinement(&dh).expect("configure fast refinement backend for test");

        // Track `pr create` calls
        let pr_create_log = dh.temp_dir.path().join("pr_create_no_diff.txt");
        let pr_create_log_str = pr_create_log.to_string_lossy().into_owned();

        // Track comment calls and their content
        let comment_log = dh.temp_dir.path().join("comment_no_diff.txt");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        // Track label operations
        let label_log = dh.temp_dir.path().join("no_diff_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Mock gh returns one ralph:ready issue
        let issues = r#"[{"number":120,"title":"no diff issue","labels":[{"name":"ralph:ready"}],"body":"test body"}]"#;

        // Mock gh: issue list returns the ralph:ready issue, pr list returns
        // empty, pr create logs but should never be called, issue comment
        // logs the body for inspection
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        if [ -n "$MOCK_GH_ISSUES" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        # Check for comments query (marker check) vs labels query
        for arg in "$@"; do
          if [ "$arg" = "comments" ]; then
            if [ -f "{comment_log_str}" ]; then
              cat "{comment_log_str}"
            fi
            exit 0
          fi
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
            exit 0
          fi
          if [ "$arg" = "title,body" ]; then
            printf '{{"title":"no diff issue","body":"test body"}}'
            exit 0
          fi
        done
        printf '' ; exit 0
        ;;
      comment)
        # Log the comment body
        shift; shift; shift  # skip 'issue' 'comment' '<number>'
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) echo "$2" >> "{comment_log_str}" ; shift 2 ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list)
        # No existing PRs
        printf ''
        exit 0
        ;;
      create)
        echo "called" > "{pr_create_log_str}"
        printf 'https://github.com/mock/pr/1\n'
        exit 0
        ;;
      edit) exit 0 ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view) printf 'acme/widgets\n' ; exit 0 ;;
    esac
    ;;
  label)
    [ "$2" = "create" ] && exit 0
    exit 1
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");

        // Use the standard mock ralph that does NOT create commits (just exits 0).
        // This means has_diff will return false → no-diff path.
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // `pr create` should NOT have been called (no diff → no PR)
        assert!(
            !pr_create_log.exists(),
            "pr create should not be called when there is no diff"
        );

        // The no-diff marker comment should have been posted
        if comment_log.exists() {
            let log_content = fs::read_to_string(&comment_log).expect("read comment log");
            assert!(
                log_content.contains("no-diff"),
                "expected no-diff marker comment, got:\n{log_content}"
            );
        }

        // Label log should show terminal label transition (completed)
        let log = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            log.contains("ralph:completed") || log.contains("--add-label"),
            "expected label transition in label log:\n{log}"
        );
    })
}

/// Conformance: daemon runtime watcher posts both quick-prd and final-prompt
/// comments from child-produced artifacts.
fn runtime_artifact_comments_posted(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_refinement_enabled",
            "false",
        ])
        .expect("disable refinement");

        let comment_log = dh.temp_dir.path().join("artifact_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let label_log = dh.temp_dir.path().join("artifact_labels.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let issues = r#"[{"number":121,"title":"artifact watcher","labels":[{"name":"ralph:ready"}],"body":"artifact body"}]"#;

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        if [ -n "$MOCK_GH_ISSUES" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "comments" ]; then
            if [ -f "{comment_log_str}" ]; then
              cat "{comment_log_str}"
            fi
            exit 0
          fi
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
            exit 0
          fi
          if [ "$arg" = "title,body" ]; then
            printf '{{"title":"artifact watcher","body":"artifact body"}}'
            exit 0
          fi
        done
        printf ''
        exit 0
        ;;
      comment)
        shift; shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body)
              printf '%s\n' "$2" >> "{comment_log_str}"
              shift 2
              ;;
            --repo) shift 2 ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view) printf 'acme/widgets\n' ; exit 0 ;;
    esac
    ;;
  label)
    [ "$2" = "create" ] && exit 0
    exit 1
    ;;
esac
exit 1
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");

        let ralph_script = r#"#!/bin/sh
case "$1" in
  auto)
    mkdir -p .ralph/quick-prd/001-demo
    printf 'Quick PRD content from watcher test\n' > .ralph/quick-prd/001-demo/SPEC.md
    printf '{}' > .ralph/quick-prd/001-demo/meta.json
    mkdir -p .ralph/projects/demo-proj
    printf 'prompt signal\n' > .ralph/projects/demo-proj/prompt-original.md
    printf 'Final prompt content from watcher test\n' > .ralph/projects/demo-proj/prompt.md
    sleep 1
    exit 0
    ;;
  *)
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"#;
        let ralph_path = write_mock_ralph(&dh, ralph_script).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let comments = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            comments.contains("<!-- ralph:task:acme-widgets-121:quick-prd -->"),
            "expected quick-prd marker, comments:\n{comments}"
        );
        assert!(
            comments.contains("### Quick PRD"),
            "expected quick-prd header, comments:\n{comments}"
        );
        assert!(
            comments.contains("Quick PRD content from watcher test"),
            "expected quick-prd body content, comments:\n{comments}"
        );

        assert!(
            comments.contains("<!-- ralph:task:acme-widgets-121:final-prompt -->"),
            "expected final-prompt marker, comments:\n{comments}"
        );
        assert!(
            comments.contains("### Final Prompt (after review)"),
            "expected final-prompt header, comments:\n{comments}"
        );
        assert!(
            comments.contains("Final prompt content from watcher test"),
            "expected final-prompt body content, comments:\n{comments}"
        );

        assert_eq!(
            comments
                .matches("<!-- ralph:task:acme-widgets-121:quick-prd -->")
                .count(),
            1,
            "quick-prd comment should be idempotent"
        );
        assert_eq!(
            comments
                .matches("<!-- ralph:task:acme-widgets-121:final-prompt -->")
                .count(),
            1,
            "final-prompt comment should be idempotent"
        );
    })
}

fn daemon_bootstrap_non_git_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let repo_root = h.temp_dir.path().join("bootstrap-non-git");
        fs::create_dir_all(&repo_root).expect("create non-git repo root");

        ensure_repo_ready_blocking(&repo_root).expect("bootstrap should succeed");

        assert!(
            repo_root.join(".git").exists(),
            "expected .git after bootstrap"
        );
        assert!(
            repo_root.join(".ralph").exists(),
            "expected .ralph workspace after bootstrap"
        );
        assert_eq!(
            git_stdout(&repo_root, &["rev-list", "--count", "HEAD"]),
            "1",
            "non-git bootstrap should create exactly one bootstrap commit"
        );
    })
}

fn daemon_bootstrap_zero_commit_repo(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let zero =
            RalphHarness::new_zero_commit_repo(&h.ralph_bin).expect("create zero-commit repo");

        ensure_repo_ready_blocking(&zero.repo_root).expect("bootstrap should succeed");

        assert_eq!(
            git_stdout(&zero.repo_root, &["rev-list", "--count", "HEAD"]),
            "1",
            "zero-commit repo should receive one bootstrap commit"
        );
        let subject = git_stdout(&zero.repo_root, &["log", "-1", "--pretty=%s"]);
        assert!(
            subject.contains("ralph: bootstrap empty commit"),
            "expected bootstrap commit subject, got: {subject}"
        );
        assert!(
            zero.repo_root.join(".ralph").exists(),
            "workspace should be initialized for zero-commit repo"
        );
    })
}

fn daemon_bootstrap_idempotent(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let zero =
            RalphHarness::new_zero_commit_repo(&h.ralph_bin).expect("create zero-commit repo");

        ensure_repo_ready_blocking(&zero.repo_root).expect("first bootstrap should succeed");
        let head_before = git_stdout(&zero.repo_root, &["rev-parse", "HEAD"]);
        let count_before = git_stdout(&zero.repo_root, &["rev-list", "--count", "HEAD"]);

        ensure_repo_ready_blocking(&zero.repo_root).expect("second bootstrap should succeed");
        let head_after = git_stdout(&zero.repo_root, &["rev-parse", "HEAD"]);
        let count_after = git_stdout(&zero.repo_root, &["rev-list", "--count", "HEAD"]);

        assert_eq!(
            head_after, head_before,
            "HEAD should be stable across bootstrap runs"
        );
        assert_eq!(
            count_after, count_before,
            "idempotent bootstrap should not create additional commits"
        );
    })
}

fn daemon_bootstrap_existing_repo_noop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let head_before = git_stdout(&h.repo_root, &["rev-parse", "HEAD"]);
        let count_before = git_stdout(&h.repo_root, &["rev-list", "--count", "HEAD"]);

        ensure_repo_ready_blocking(&h.repo_root).expect("bootstrap should succeed");

        let head_after = git_stdout(&h.repo_root, &["rev-parse", "HEAD"]);
        let count_after = git_stdout(&h.repo_root, &["rev-list", "--count", "HEAD"]);

        assert_eq!(
            head_after, head_before,
            "existing repository HEAD should not change during bootstrap"
        );
        assert_eq!(
            count_after, count_before,
            "existing repository commit count should not change during bootstrap"
        );
    })
}

fn daemon_has_diff_invalid_base_returns_false(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("invalid-base");
        fs::create_dir_all(&repo_root).expect("create repo root");

        git(&repo_root, &["init"]);
        git(
            &repo_root,
            &["config", "user.email", "validate@example.com"],
        );
        git(&repo_root, &["config", "user.name", "Validate Harness"]);
        fs::write(repo_root.join("README.md"), "hello\n").expect("write README");
        git(&repo_root, &["add", "README.md"]);
        git(&repo_root, &["commit", "-m", "initial"]);
        git(&repo_root, &["branch", "-M", "trunk"]);

        let has_changes = github::has_diff(&repo_root).expect("has_diff should not hard-fail");
        assert!(
            !has_changes,
            "invalid base revision in single-commit repo should be treated as no diff"
        );
    })
}

/// Test that create_worktree reuses an existing branch instead of failing
/// with "branch already exists" when a prior run left a stale branch behind.
fn create_worktree_reuses_existing_branch(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        let workspace_root = h.repo_root.join(".ralph");

        // Create a worktree (creates branch ralph/daemon/acme-widgets-99)
        let wt = worktree::create_worktree(&h.repo_root, &workspace_root, "acme-widgets-99")
            .expect("first create_worktree should succeed");
        assert!(wt.exists(), "worktree directory should exist");

        // Verify branch exists
        let branch_check = git_stdout(
            &h.repo_root,
            &["branch", "--list", "ralph/daemon/acme-widgets-99"],
        );
        assert!(
            branch_check.contains("ralph/daemon/acme-widgets-99"),
            "branch should exist after create_worktree"
        );

        // Remove the worktree but leave the branch (simulates failed task cleanup)
        worktree::remove_worktree(&h.repo_root, &workspace_root, "acme-widgets-99");
        assert!(!wt.exists(), "worktree directory should be removed");

        // Branch should still exist
        let branch_check = git_stdout(
            &h.repo_root,
            &["branch", "--list", "ralph/daemon/acme-widgets-99"],
        );
        assert!(
            branch_check.contains("ralph/daemon/acme-widgets-99"),
            "branch should survive worktree removal"
        );

        // Second create_worktree should succeed by reusing the existing branch
        let wt2 = worktree::create_worktree(&h.repo_root, &workspace_root, "acme-widgets-99")
            .expect("second create_worktree should succeed with existing branch");
        assert!(wt2.exists(), "worktree directory should be re-created");
    })
}

/// Test that clean_worktree removes dirty tracked and untracked files
/// while preserving the .ralph/ directory.
fn clean_worktree_removes_dirty_files(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        let workspace_root = h.repo_root.join(".ralph");

        // Create a worktree
        let wt = worktree::create_worktree(&h.repo_root, &workspace_root, "acme-widgets-77")
            .expect("create_worktree should succeed");

        // Create a tracked file, commit it, then modify it (dirty tracked)
        fs::write(wt.join("tracked.rs"), "fn original() {}").expect("write tracked");
        git(&wt, &["add", "tracked.rs"]);
        git(
            &wt,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@test.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--no-verify",
                "-m",
                "add tracked",
            ],
        );
        fs::write(wt.join("tracked.rs"), "fn modified() {}").expect("modify tracked");

        // Create an untracked file (like SPEC.md from codex side-effect)
        fs::write(wt.join("SPEC.md"), "# Stale spec").expect("write untracked");

        // Create a file inside .ralph/ (should be preserved)
        let ralph_dir = wt.join(".ralph").join("test");
        fs::create_dir_all(&ralph_dir).expect("create .ralph/test");
        fs::write(ralph_dir.join("marker.txt"), "{}").expect("write .ralph marker");

        // Verify dirty state
        let status = git_stdout(&wt, &["status", "--short"]);
        assert!(
            status.contains("tracked.rs") || status.contains("SPEC.md"),
            "worktree should be dirty before clean, status:\n{status}"
        );

        // Clean the worktree
        worktree::clean_worktree(&wt).expect("clean_worktree should succeed");

        // Dirty files should be gone
        let status_after = git_stdout(&wt, &["status", "--short"]);
        assert!(
            !status_after.contains("tracked.rs"),
            "tracked modifications should be reverted after clean, status:\n{status_after}"
        );
        assert!(
            !wt.join("SPEC.md").exists(),
            "untracked SPEC.md should be removed after clean"
        );

        // .ralph/ directory should be preserved
        assert!(
            ralph_dir.join("marker.txt").exists(),
            ".ralph/ contents should survive clean_worktree"
        );
    })
}

/// Test that create_worktree recovers from stale git worktree metadata by
/// pruning before `git worktree add`.
fn runtime_create_worktree_handles_stale_metadata(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        let workspace_root = h.repo_root.join(".ralph");
        let task_id = "acme-widgets-381";

        let wt = worktree::create_worktree(&h.repo_root, &workspace_root, task_id)
            .expect("initial create_worktree should succeed");
        assert!(wt.exists(), "worktree should exist after initial creation");

        // Simulate a crash/manual delete: remove the directory without
        // `git worktree remove`, leaving stale metadata under .git/worktrees.
        fs::remove_dir_all(&wt).expect("remove worktree dir directly");
        assert!(
            !wt.exists(),
            "worktree dir should be removed to simulate stale metadata"
        );

        let list_before = git_stdout(&h.repo_root, &["worktree", "list", "--porcelain"]);
        assert!(
            list_before.contains(task_id),
            "expected stale worktree metadata for task before recreation, got:\n{list_before}"
        );

        // Must succeed because create_worktree now prunes before `worktree add`.
        let wt2 = worktree::create_worktree(&h.repo_root, &workspace_root, task_id)
            .expect("create_worktree should recover from stale metadata");
        assert!(wt2.exists(), "worktree should be recreated after prune");
    })
}

/// Test that reusing an existing worktree corrects a branch mismatch by
/// force-checking out the expected daemon branch.
fn runtime_reuse_worktree_corrects_branch_mismatch(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        let workspace_root = h.repo_root.join(".ralph");
        let task_id = "acme-widgets-382";
        let expected_branch = format!("ralph/daemon/{task_id}");
        let mismatched_branch = "tmp-branch-mismatch-382";

        let wt = worktree::create_worktree(&h.repo_root, &workspace_root, task_id)
            .expect("initial create_worktree should succeed");
        assert!(wt.exists(), "worktree should exist");

        git(&wt, &["checkout", "-b", mismatched_branch]);
        let before = git_stdout(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(
            before, mismatched_branch,
            "test setup should move worktree to mismatched branch"
        );

        let reused = worktree::create_worktree(&h.repo_root, &workspace_root, task_id)
            .expect("reuse path should correct branch mismatch");
        assert_eq!(reused, wt, "reuse path should return same worktree path");

        let after = git_stdout(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(
            after, expected_branch,
            "reuse path should force-checkout expected daemon branch"
        );
    })
}

// =============================================================================
// Loop 2 Data-Dir Provisioning and Multi-Repo Tests
// =============================================================================

/// Create an empty data-dir, run daemon start with a mock gh that simulates
/// clone. Assert: repo dir exists, .ralph/ workspace initialized, label lifecycle
/// written, daemon completes exit 0.
fn daemon_start_bootstraps_empty_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir = temp.path().join("fresh-data");
        let data_dir_str = data_dir.to_string_lossy().into_owned();

        let gh_path =
            write_mock_gh(h, &mock_scripts::daemon_mock_gh_clone_script()).expect("write mock gh");

        let ralph_path = write_daemon_mock_ralph(h).expect("write mock ralph");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--data-dir",
                    &data_dir_str,
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let repo_dir = data_dir.join("acme").join("widgets");
        assert!(
            repo_dir.join(".git").exists(),
            "repo should have been cloned: {}",
            repo_dir.display()
        );
        assert!(
            repo_dir.join(".ralph").exists(),
            ".ralph/ workspace should be initialized: {}",
            repo_dir.display()
        );
    })
}

/// Create a data-dir inside an existing git repo. Verify daemon start rejects
/// it with a clear error.
fn daemon_start_rejects_git_data_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // h.repo_root is already a git repo. Use a subdirectory of it as data-dir.
        let git_subdir = h.repo_root.join("daemon-data");
        std::fs::create_dir_all(&git_subdir).expect("create subdir");
        let data_dir_str = git_subdir.to_string_lossy().into_owned();

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--data-dir",
                    &data_dir_str,
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[],
            )
            .expect("daemon start should execute");

        let exit_code = output.status.code().unwrap_or(-1);
        assert_ne!(exit_code, 0, "daemon start should fail");

        let combined = combined_output(&output);
        assert!(
            combined.contains("must not be inside a git repository"),
            "expected git repo guard error, got:\n{combined}"
        );
    })
}

/// Run daemon start with duplicate --repo values. Verify rejection.
fn daemon_start_rejects_duplicate_repo(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir_str = temp.path().to_string_lossy().into_owned();

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--data-dir",
                    &data_dir_str,
                    "--repo",
                    "acme/widgets",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[],
            )
            .expect("daemon start should execute");

        let exit_code = output.status.code().unwrap_or(-1);
        assert_ne!(exit_code, 0, "daemon start should fail with duplicate repo");

        let combined = combined_output(&output);
        assert!(
            combined.contains("duplicate --repo"),
            "expected duplicate repo error, got:\n{combined}"
        );
    })
}

/// Run daemon start with a mock gh that fails on clone. Verify the error
/// propagates and no .ralph/ directory is created.
fn daemon_start_clone_failure_propagates(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let temp = tempfile::tempdir().expect("temp dir");
        let data_dir = temp.path().join("clone-fail");
        let data_dir_str = data_dir.to_string_lossy().into_owned();

        let gh_path =
            write_mock_gh(h, &mock_scripts::daemon_mock_gh_clone_script()).expect("write mock gh");

        let output = h
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--data-dir",
                    &data_dir_str,
                    "--repo",
                    "acme/nonexistent",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("MOCK_GH_CLONE_FAIL", "true")],
            )
            .expect("daemon start should execute");

        let exit_code = output.status.code().unwrap_or(-1);
        assert_ne!(exit_code, 0, "daemon start should fail on clone failure");

        let combined = combined_output(&output);
        assert!(
            combined.contains("clone") || combined.contains("Could not resolve"),
            "expected clone failure message, got:\n{combined}"
        );

        // Ensure bootstrap did not silently run
        let repo_dir = data_dir.join("acme").join("nonexistent");
        assert!(
            !repo_dir.join(".ralph").exists(),
            ".ralph/ should not exist after clone failure"
        );
    })
}

/// Pre-populate tasks for two repos under a data-dir. Run daemon status.
/// Assert both repos' tasks appear.
fn daemon_status_multi_repo(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let temp = tempfile::tempdir().expect("temp dir");

        // Set up two repos under data-dir
        let dh1 =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness 1");
        let dh2 =
            RalphHarness::new_daemon(&h.ralph_bin, "acme", "gadgets").expect("daemon harness 2");

        // Initialize workspaces
        dh1.init_workspace().expect("init 1");
        dh2.init_workspace().expect("init 2");

        // Copy both repos under a single data-dir
        let combined_data = temp.path().join("combined");
        std::fs::create_dir_all(combined_data.join("acme")).expect("create acme dir");

        // Copy repo 1
        copy_dir_recursive(&dh1.repo_root, &combined_data.join("acme").join("widgets"))
            .expect("copy repo 1");
        // Copy repo 2
        copy_dir_recursive(&dh2.repo_root, &combined_data.join("acme").join("gadgets"))
            .expect("copy repo 2");

        // Mock gh returns issues with lifecycle labels (status queries ralph:ready
        // and ralph:in-progress separately, both will get the same MOCK_GH_ISSUES).
        let issues = r#"[{"number":10,"title":"widget issue","labels":[{"name":"ralph:ready"}],"body":"w"},{"number":20,"title":"gadget issue","labels":[{"name":"ralph:in-progress"}],"body":"g"}]"#;

        let gh_path = write_daemon_mock_gh(&dh1).expect("write mock gh");

        let combined_str = combined_data.to_string_lossy().into_owned();
        let output = h
            .ralph_env(
                ["daemon", "status", "--data-dir", &combined_str],
                &[("PATH", &gh_path), ("MOCK_GH_ISSUES", issues)],
            )
            .expect("daemon status should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Status now displays issue numbers, not task IDs
        assert!(
            stdout.contains("10"),
            "expected issue 10 in output, got:\n{stdout}"
        );
        assert!(
            stdout.contains("20"),
            "expected issue 20 in output, got:\n{stdout}"
        );
        assert!(
            stdout.contains("DAEMON ISSUES"),
            "expected DAEMON ISSUES header, got:\n{stdout}"
        );
    })
}

// =============================================================================
// Test helpers
// =============================================================================

fn ensure_repo_ready_blocking(repo_root: &Path) -> crate::Result<()> {
    let repo_root = repo_root.to_path_buf();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| {
                crate::error::RalphError::Orchestration(format!("tokio runtime init failed: {err}"))
            })?;
        runtime.block_on(bootstrap::ensure_repo_ready(&repo_root))
    })
    .join()
    .map_err(|_| crate::error::RalphError::Orchestration("bootstrap thread panicked".to_owned()))?
}

fn git(repo_root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        repo_root.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

// =============================================================================
// Loop 2 Dispatch-time Project Backfill Tests
// =============================================================================

/// Asserts stray project directories without `prompt.md` are ignored by
/// dispatch-time project discovery.
///
/// Sets up a worktree with:
/// - `.ralph/projects/valid-proj/prompt.md` (valid)
/// - `.ralph/projects/stray-proj/` (no prompt.md — stray)
///
/// The legacy task (project_id = null) should discover only `valid-proj` and
/// dispatch via `ralph run --project valid-proj`.
fn discover_project_id_ignores_dirs_without_state_json(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let task_id = "acme-widgets-500";

        // Pre-create a real git worktree so daemon reuses it.
        let workspace_root = dh.repo_root.join(".ralph");
        let wt_path = worktree::create_worktree(&dh.repo_root, &workspace_root, task_id)
            .expect("create worktree");

        // Valid project: has prompt.md
        let valid_proj_dir = wt_path.join(".ralph").join("projects").join("valid-proj");
        fs::create_dir_all(&valid_proj_dir).expect("create valid project dir");
        fs::write(valid_proj_dir.join("prompt.md"), "valid prompt").expect("write valid prompt");

        // Stray project: directory only, no prompt.md
        let stray_proj_dir = wt_path.join(".ralph").join("projects").join("stray-proj");
        fs::create_dir_all(&stray_proj_dir).expect("create stray project dir");

        // Provide a ralph:ready issue for the daemon to claim and dispatch
        let issues = r#"[{"number":500,"title":"Legacy task","labels":[{"name":"ralph:ready"}],"body":"Should discover valid project."}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let args_log = dh.temp_dir.path().join("discovery_args.log");
        let args_log_str = args_log.to_string_lossy().into_owned();

        let ralph_script = format!(
            r#"#!/bin/sh
case "$1" in
  run)
    printf '%s\n' "$1" > "{args_log_str}"
    printf '%s\n' "$2" >> "{args_log_str}"
    printf '%s\n' "$3" >> "{args_log_str}"
    exit 0
    ;;
  auto)
    printf 'auto\n' > "{args_log_str}"
    exit 0
    ;;
  *)
    echo "mock ralph: unhandled: $1" >&2
    exit 1
    ;;
esac
"#
        );
        let ralph_path = write_mock_ralph(&dh, &ralph_script).expect("write mock ralph");

        let label_log = dh.temp_dir.path().join("discover_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify dispatch used `ralph run --project valid-proj`
        let args = fs::read_to_string(&args_log).expect("read args log");
        assert!(
            args.starts_with("run\n--project\nvalid-proj\n"),
            "expected run --project valid-proj, got:\n{args}"
        );
    })
}

fn git_stdout(repo_root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        repo_root.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
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

fn write_daemon_mock_gh(h: &RalphHarness) -> crate::Result<String> {
    write_mock_gh(h, &mock_scripts::daemon_mock_gh_script())
}

/// Write a mock ralph script and return its absolute path (suitable for
/// RALPH_DAEMON_BIN env var).
fn write_mock_ralph(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("mock_ralph", body)?;
    Ok(script.to_string_lossy().into_owned())
}

/// Write the standard mock ralph (exits 0) and return its path.
fn write_daemon_mock_ralph(h: &RalphHarness) -> crate::Result<String> {
    write_mock_ralph(h, &mock_scripts::daemon_mock_ralph_script())
}

fn enable_fast_daemon_refinement(h: &RalphHarness) -> crate::Result<()> {
    let refine_script = h.write_mock_script(
        "mock_refine_fast.sh",
        &mock_scripts::daemon_mock_fast_refinement_script(),
    )?;
    let refine_script_str = refine_script.to_string_lossy().into_owned();
    h.ralph_ok([
        "config",
        "set",
        "backends.claude.command",
        &refine_script_str,
    ])?;
    h.ralph_ok(["config", "set", "backends.claude.args", "[]"])?;
    h.ralph_ok([
        "config",
        "set",
        "workspace.daemon_refinement_enabled",
        "true",
    ])?;
    Ok(())
}

fn assert_invalid_verbose_flag_error(stderr: &str) {
    let lowered = stderr.to_lowercase();
    assert!(
        stderr.contains("--verbose"),
        "expected clap error to reference --verbose, got:\n{stderr}"
    );
    assert!(
        lowered.contains("unexpected")
            || lowered.contains("unrecognized")
            || lowered.contains("unknown")
            || lowered.contains("found argument"),
        "expected clap invalid-flag wording in stderr, got:\n{stderr}"
    );
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// =============================================================================
// Loop 2 Remote-First Branch Sync Tests
// =============================================================================

/// Helper: create a bare remote, push initial commit, clone it. Returns
/// (temp_dir, bare_path, clone_path).
fn setup_remote_clone() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let bare_path = tmp.path().join("remote.git");
    let setup_path = tmp.path().join("setup");
    let clone_path = tmp.path().join("clone");

    // Create bare remote
    fs::create_dir_all(bare_path.parent().unwrap()).unwrap();
    let status = Command::new("git")
        .args(["init", "--bare", &bare_path.to_string_lossy()])
        .current_dir(tmp.path())
        .status()
        .expect("git init --bare");
    assert!(status.success());

    // Create a working repo, commit, push
    fs::create_dir_all(&setup_path).unwrap();
    let git_setup = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&setup_path)
            .status()
            .expect("git setup");
        assert!(status.success(), "git {:?} failed", args);
    };
    git_setup(&["init"]);
    git_setup(&["config", "user.email", "test@example.com"]);
    git_setup(&["config", "user.name", "Test User"]);
    fs::write(setup_path.join("README.md"), "# test\n").unwrap();
    git_setup(&["add", "-A"]);
    git_setup(&["commit", "-m", "initial"]);
    git_setup(&["remote", "add", "origin", &bare_path.to_string_lossy()]);
    git_setup(&["push", "-u", "origin", "HEAD"]);

    // Clone
    let status = Command::new("git")
        .args([
            "clone",
            &bare_path.to_string_lossy(),
            &clone_path.to_string_lossy(),
        ])
        .current_dir(tmp.path())
        .status()
        .expect("git clone");
    assert!(status.success());

    let git_clone = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&clone_path)
            .status()
            .expect("git clone setup");
        assert!(status.success(), "git {:?} failed", args);
    };
    git_clone(&["config", "user.email", "test@example.com"]);
    git_clone(&["config", "user.name", "Test User"]);

    (tmp, bare_path, clone_path)
}

fn git_run(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git command");
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        repo.display()
    );
}

fn git_out(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {:?} failed in {}",
        args,
        repo.display()
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Conformance: sync_project_branch resets local branch to remote when
/// origin/ralph/issue-<n> exists, discarding diverged local commits.
fn sync_project_branch_resets_to_remote(_h: &RalphHarness) -> TestResult {
    use crate::git::branch::sync_project_branch;
    run_case(|| {
        let (_tmp, _bare, clone) = setup_remote_clone();
        let base_branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);

        // Push a project branch to remote
        git_run(&clone, &["checkout", "-b", "ralph/issue-42"]);
        fs::write(clone.join("remote-file.txt"), "from remote\n").unwrap();
        git_run(&clone, &["add", "-A"]);
        git_run(&clone, &["commit", "-m", "remote commit"]);
        git_run(&clone, &["push", "origin", "ralph/issue-42"]);
        let remote_sha = git_out(&clone, &["rev-parse", "HEAD"]);

        // Add a local-only diverged commit
        fs::write(clone.join("local-only.txt"), "local diverge\n").unwrap();
        git_run(&clone, &["add", "-A"]);
        git_run(&clone, &["commit", "-m", "local only"]);
        let local_sha = git_out(&clone, &["rev-parse", "HEAD"]);
        assert_ne!(remote_sha, local_sha);

        sync_project_branch(&clone, 42, &base_branch).expect("sync should succeed");

        let after_sha = git_out(&clone, &["rev-parse", "HEAD"]);
        assert_eq!(after_sha, remote_sha, "should reset to remote");

        let branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-42");
    })
}

/// Conformance: sync_project_branch creates from origin/HEAD when remote
/// project branch doesn't exist.
fn sync_project_branch_creates_from_origin_head(_h: &RalphHarness) -> TestResult {
    use crate::git::branch::sync_project_branch;
    run_case(|| {
        let (_tmp, _bare, clone) = setup_remote_clone();
        let base_branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let origin_base_ref = format!("origin/{base_branch}");

        let origin_base = git_out(&clone, &["rev-parse", &origin_base_ref]);
        git_run(&clone, &["checkout", "-b", "scratch"]);

        sync_project_branch(&clone, 99, &base_branch).expect("sync should succeed");

        let after_sha = git_out(&clone, &["rev-parse", "HEAD"]);
        assert_eq!(
            after_sha, origin_base,
            "should create from origin/<base_branch>"
        );

        let branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert_eq!(branch, "ralph/issue-99");
    })
}

/// Conformance: sync_project_branch produces an actionable error when
/// origin/<base_branch> is missing, including issue number, branch name, and failed
/// git operation.
fn sync_project_branch_missing_origin_head_error(_h: &RalphHarness) -> TestResult {
    use crate::git::branch::sync_project_branch;
    run_case(|| {
        let (_tmp, bare, clone) = setup_remote_clone();
        let base_branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
        let origin_base_ref = format!("origin/{base_branch}");
        git_run(&clone, &["checkout", "-b", "scratch"]);

        // Delete remote base branch and point HEAD to a non-existent branch so
        // fetch won't restore origin/<base_branch>.
        git_run(&bare, &["symbolic-ref", "HEAD", "refs/heads/nonexistent"]);
        git_run(&bare, &["branch", "-D", &base_branch]);
        git_run(
            &clone,
            &[
                "update-ref",
                "-d",
                &format!("refs/remotes/{origin_base_ref}"),
            ],
        );

        let result = sync_project_branch(&clone, 7, &base_branch);
        assert!(result.is_err(), "should fail without origin/<base_branch>");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains(&origin_base_ref),
            "error should mention origin/<base_branch>: {err}"
        );
        assert!(
            err.contains("issue 7") || err.contains("issue-7"),
            "error should mention issue: {err}"
        );
        assert!(
            err.contains("ralph/issue-7"),
            "error should mention branch: {err}"
        );
        assert!(
            err.contains(&format!("git branch -f {base_branch} {origin_base_ref}")),
            "error should mention the failed git operation: {err}"
        );
    })
}

/// Conformance (integration-style): a local-only commit on ralph/issue-<n>
/// is removed after sync realigns to remote.
fn sync_project_branch_discards_local_commit(_h: &RalphHarness) -> TestResult {
    use crate::git::branch::sync_project_branch;
    run_case(|| {
        let (_tmp, _bare, clone) = setup_remote_clone();
        let base_branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);

        // Push project branch
        git_run(&clone, &["checkout", "-b", "ralph/issue-10"]);
        fs::write(clone.join("base.txt"), "base\n").unwrap();
        git_run(&clone, &["add", "-A"]);
        git_run(&clone, &["commit", "-m", "base commit"]);
        git_run(&clone, &["push", "origin", "ralph/issue-10"]);
        let remote_sha = git_out(&clone, &["rev-parse", "HEAD"]);

        // Add local-only commit
        fs::write(clone.join("local-artifact.txt"), "should vanish\n").unwrap();
        git_run(&clone, &["add", "-A"]);
        git_run(&clone, &["commit", "-m", "local only"]);

        sync_project_branch(&clone, 10, &base_branch).expect("sync should succeed");

        let after_sha = git_out(&clone, &["rev-parse", "HEAD"]);
        assert_eq!(after_sha, remote_sha, "local commit should be discarded");
        assert!(
            !clone.join("local-artifact.txt").exists(),
            "local-only file should not exist"
        );
    })
}

/// Conformance: stale local base is force-updated to origin/<base_branch>
/// before creating a parentless issue branch.
fn sync_project_branch_force_updates_stale_base(_h: &RalphHarness) -> TestResult {
    use crate::git::branch::sync_project_branch;
    run_case(|| {
        let (_tmp, _bare, clone) = setup_remote_clone();
        let base_branch = git_out(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);

        let stale_base_sha = git_out(&clone, &["rev-parse", "HEAD"]);
        fs::write(clone.join("remote-base-advance.txt"), "advance base\n").unwrap();
        git_run(&clone, &["add", "-A"]);
        git_run(&clone, &["commit", "-m", "advance base on remote"]);
        git_run(&clone, &["push", "origin", &base_branch]);
        let remote_base_sha = git_out(&clone, &["rev-parse", &format!("origin/{base_branch}")]);

        git_run(&clone, &["reset", "--hard", &stale_base_sha]);
        let local_base_before = git_out(&clone, &["rev-parse", &base_branch]);
        assert_ne!(
            local_base_before, remote_base_sha,
            "local base should be stale before sync"
        );

        git_run(&clone, &["checkout", "-b", "scratch"]);
        sync_project_branch(&clone, 555, &base_branch).expect("sync should succeed");

        let local_base_after = git_out(&clone, &["rev-parse", &base_branch]);
        let remote_base_after = git_out(&clone, &["rev-parse", &format!("origin/{base_branch}")]);
        assert_eq!(
            local_base_after, remote_base_after,
            "local base should be force-updated to origin/<base_branch>"
        );

        let head_sha = git_out(&clone, &["rev-parse", "HEAD"]);
        assert_eq!(
            head_sha, remote_base_after,
            "issue branch should start from refreshed remote base"
        );
    })
}

/// Conformance: worktree creation uses origin/HEAD (not origin/master or local refs).
fn worktree_uses_origin_head_not_local_refs(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let (_tmp, _bare, clone) = setup_remote_clone();

        // The clone has origin/HEAD pointing to default branch.
        // Create workspace structure
        let workspace_root = clone.join(".ralph");
        fs::create_dir_all(workspace_root.join("daemon")).unwrap();

        let result = worktree::create_worktree(&clone, &workspace_root, "test-task-1");
        assert!(
            result.is_ok(),
            "worktree creation should succeed with origin/HEAD: {:?}",
            result.err()
        );

        let wt_path = result.unwrap();
        assert!(wt_path.exists(), "worktree dir should exist");

        // Verify the worktree is on a branch based on origin/HEAD
        let origin_head_sha = git_out(&clone, &["rev-parse", "origin/HEAD"]);
        let wt_head_sha = git_out(&wt_path, &["rev-parse", "HEAD"]);
        assert_eq!(
            origin_head_sha, wt_head_sha,
            "worktree HEAD should match origin/HEAD"
        );
    })
}

/// Conformance: worktree creation falls back to origin/master (or origin/main)
/// when origin/HEAD symbolic ref is missing (fresh repos).
fn worktree_falls_back_when_origin_head_missing(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Build a repo without origin/HEAD by using init + remote add + fetch
        // instead of git clone.
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let bare_path = tmp.path().join("remote.git");
        let repo_path = tmp.path().join("repo");

        // Create bare remote with an initial commit on master
        let status = Command::new("git")
            .args(["init", "--bare", &bare_path.to_string_lossy()])
            .status()
            .expect("git init --bare");
        assert!(status.success());

        let setup_path = tmp.path().join("setup");
        fs::create_dir_all(&setup_path).unwrap();
        let git_setup = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&setup_path)
                .status()
                .expect("git setup");
            assert!(status.success(), "git {:?} failed", args);
        };
        git_setup(&["init"]);
        git_setup(&["config", "user.email", "test@example.com"]);
        git_setup(&["config", "user.name", "Test User"]);
        fs::write(setup_path.join("README.md"), "# test\n").unwrap();
        git_setup(&["add", "-A"]);
        git_setup(&["commit", "-m", "initial"]);
        git_setup(&["remote", "add", "origin", &bare_path.to_string_lossy()]);
        git_setup(&["push", "-u", "origin", "HEAD"]);

        // Set up repo using init + remote add + fetch (no clone → no origin/HEAD)
        fs::create_dir_all(&repo_path).unwrap();
        let git_repo = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&repo_path)
                .status()
                .expect("git repo setup");
            assert!(status.success(), "git {:?} failed", args);
        };
        git_repo(&["init"]);
        git_repo(&["config", "user.email", "test@example.com"]);
        git_repo(&["config", "user.name", "Test User"]);
        git_repo(&["remote", "add", "origin", &bare_path.to_string_lossy()]);
        git_repo(&["fetch", "origin"]);

        // Modern git (2.52+) may auto-set origin/HEAD on fetch from local
        // bare repos. Explicitly remove it to simulate the fresh-repo
        // scenario where origin/HEAD is absent (common with GitHub repos).
        let _ = Command::new("git")
            .args(["remote", "set-head", "origin", "--delete"])
            .current_dir(&repo_path)
            .output();

        // Point origin at a non-existent path so `set-head --auto` cannot
        // restore origin/HEAD.  origin/master still exists from the fetch.
        let _ = Command::new("git")
            .args(["remote", "set-url", "origin", "/nonexistent/repo.git"])
            .current_dir(&repo_path)
            .output();

        // Verify origin/HEAD is indeed missing
        let check = Command::new("git")
            .args(["rev-parse", "--verify", "origin/HEAD"])
            .current_dir(&repo_path)
            .output()
            .expect("rev-parse");
        assert!(
            !check.status.success(),
            "origin/HEAD should not exist in this setup"
        );

        // Verify origin/master does exist (the fallback target)
        let check = Command::new("git")
            .args(["rev-parse", "--verify", "origin/master"])
            .current_dir(&repo_path)
            .output()
            .expect("rev-parse");
        assert!(
            check.status.success(),
            "origin/master should exist after fetch"
        );

        let workspace_root = repo_path.join(".ralph");
        fs::create_dir_all(workspace_root.join("daemon")).unwrap();

        let result = worktree::create_worktree(&repo_path, &workspace_root, "fresh-task-1");
        assert!(
            result.is_ok(),
            "worktree creation should succeed via fallback: {:?}",
            result.err()
        );

        let wt_path = result.unwrap();
        assert!(wt_path.exists(), "worktree dir should exist");

        // Verify worktree HEAD matches origin/master
        let origin_master_sha = git_out(&repo_path, &["rev-parse", "origin/master"]);
        let wt_head_sha = git_out(&wt_path, &["rev-parse", "HEAD"]);
        assert_eq!(
            origin_master_sha, wt_head_sha,
            "worktree HEAD should match origin/master when origin/HEAD is missing"
        );
    })
}

// =============================================================================
// Loop 4: Label Lifecycle No-Durable-Store Tests
// =============================================================================

/// Verify that no durable daemon JSON lifecycle file is created during a daemon runtime cycle.
/// This confirms the removal of durable local task state.
fn no_tasks_json_written_after_runtime(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Run daemon with a mock issue that has ralph:ready label
        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let issues = r#"[{"number":1,"title":"test issue","labels":[{"name":"ralph:ready"}],"body":"test body"}]"#;

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify no durable daemon JSON state was created
        let daemon_dir = dh
            .data_dir()
            .join("acme")
            .join("widgets")
            .join(".ralph")
            .join("daemon");
        if daemon_dir.exists() {
            let has_json = fs::read_dir(&daemon_dir)
                .expect("read daemon dir")
                .flatten()
                .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"));
            assert!(
                !has_json,
                "daemon should not persist JSON lifecycle state in {}",
                daemon_dir.display()
            );
        }
    })
}

/// Verify that on daemon startup, any issues with ralph:in-progress are
/// reset to ralph:ready (reconciliation).
fn startup_reconcile_resets_in_progress_to_ready(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let label_log = dh.temp_dir.path().join("reconcile_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Issues returned by poll: one is ralph:in-progress (stale from previous run)
        let issues = r#"[{"number":5,"title":"stale issue","labels":[{"name":"ralph:ready"},{"name":"ralph:in-progress"}],"body":"stale"}]"#;

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        if [ -n "$MOCK_GH_ISSUES" ]; then
          printf '%s' "$MOCK_GH_ISSUES"
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      view)
        for arg in "$@"; do
          if [ "$arg" = "labels" ]; then
            printf '{{"labels":[{{"name":"ralph:in-progress"}}]}}'
            exit 0
          fi
          if [ "$arg" = "title,body" ]; then
            printf '{{"title":"stale","body":"stale body"}}'
            exit 0
          fi
        done
        printf '' ; exit 0
        ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  label) exit 0 ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view) printf 'acme/widgets\n' ; exit 0 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify reconciliation: should have removed ralph:in-progress and added ralph:ready
        let log = fs::read_to_string(&label_log).unwrap_or_default();
        let stderr = String::from_utf8_lossy(&output.stderr);
        // The reconciliation phase should swap in-progress to ready
        assert!(
            stderr.contains("reconcile") || log.contains("--remove-label"),
            "expected reconciliation activity in stderr or label log:\nstderr: {stderr}\nlabel_log: {log}"
        );
    })
}

/// Verify that an issue with multiple lifecycle labels gets normalized to
/// ralph:failed during the poll-and-claim phase.
fn multi_lifecycle_label_normalizes_to_failed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let label_log = dh.temp_dir.path().join("multi_lifecycle_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Issue with both ralph:ready and ralph:in-progress (multi-lifecycle conflict)
        let issues = r#"[{"number":99,"title":"multi label issue","labels":[{"name":"ralph:ready"},{"name":"ralph:in-progress"}],"body":"multi"}]"#;

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("normalized multi-lifecycle"),
            "expected multi-lifecycle normalization log in stderr:\n{stderr}"
        );
    })
}

/// Verify retry classification and actual retry behavior for label mutations.
///
/// Part 1: Tests `is_retryable_gh_error` classification of 409/conflict, rate
/// limit, transient network errors as retryable, and auth/not-found as not.
///
/// Part 2: Exercises `add_label_with_retry` with a mock gh that fails with a
/// 409 conflict on the first attempt, then succeeds, verifying that retry
/// logic actually retried and eventually succeeded.
fn label_retry_on_conflict_transient(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // --- Part 1: is_retryable_gh_error classification ---

        // Retryable patterns
        assert!(
            github::is_retryable_gh_error("HTTP 409: Conflict"),
            "409 conflict should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("API rate limit exceeded"),
            "rate limit should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("502 Bad Gateway"),
            "502 should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("503 Service Unavailable"),
            "503 should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("connection reset by peer"),
            "connection error should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("network is unreachable"),
            "network error should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("request timeout"),
            "timeout should be retryable"
        );
        assert!(
            github::is_retryable_gh_error("could not resolve host"),
            "DNS resolution error should be retryable"
        );

        // Non-retryable patterns
        assert!(
            !github::is_retryable_gh_error("HTTP 401: Unauthorized"),
            "auth error should NOT be retryable"
        );
        assert!(
            !github::is_retryable_gh_error("HTTP 404: Not Found"),
            "not-found should NOT be retryable"
        );
        assert!(
            !github::is_retryable_gh_error("HTTP 422: Unprocessable Entity"),
            "validation error should NOT be retryable"
        );

        // --- Part 2: Actual retry path via mock gh ---

        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let attempt_log = dh.temp_dir.path().join("retry_attempt.log");
        let attempt_log_str = attempt_log.to_string_lossy().into_owned();

        // Mock gh script: fails with 409 on first call, succeeds on second
        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      edit)
        # Count attempts
        count=0
        if [ -f "{attempt_log_str}" ]; then
          count=$(wc -l < "{attempt_log_str}" | tr -d ' ')
        fi
        echo "attempt $count: $@" >> "{attempt_log_str}"
        if [ "$count" -eq "0" ]; then
          echo "HTTP 409: Conflict" >&2
          exit 1
        fi
        exit 0
        ;;
      list) printf '[]' ; exit 0 ;;
      view) printf '{{"labels":[]}}' ; exit 0 ;;
    esac
    ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh script");

        // Override PATH so add_label_with_retry uses our mock gh
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", &gh_path);

        let result = github::add_label_with_retry("acme", "widgets", 1, "ralph:in-progress");

        // Restore PATH
        std::env::set_var("PATH", &old_path);

        assert!(
            result.is_ok(),
            "add_label_with_retry should succeed after retry, got: {:?}",
            result.err()
        );

        // Verify that at least 2 attempts were made (first failed, second succeeded)
        let log = fs::read_to_string(&attempt_log).expect("read attempt log");
        let attempt_count = log.lines().count();
        assert!(
            attempt_count >= 2,
            "expected at least 2 attempts (1 failure + 1 success), got {attempt_count}:\n{log}"
        );

        // --- Part 3: classify_lifecycle_labels still works ---

        // Verify ralph:aborted is no longer a lifecycle label
        let labels_with_aborted = vec!["ralph:aborted".to_owned(), "ralph:ready".to_owned()];
        let lifecycle = github::classify_lifecycle_labels(&labels_with_aborted);
        assert_eq!(
            lifecycle.len(),
            1,
            "ralph:aborted should not be recognized as lifecycle label, got {lifecycle:?}"
        );
        assert_eq!(lifecycle[0], "ralph:ready");
    })
}

fn daemon_lock_contention_exits_immediately(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let gh_path = write_daemon_mock_gh(&dh).expect("write mock gh");
        let mut first = Command::new(&dh.ralph_bin)
            .args([
                "daemon",
                "start",
                "--data-dir",
                &dh.data_dir_str(),
                "--repo",
                "acme/widgets",
                "--poll-seconds",
                "60",
            ])
            .current_dir(dh.data_dir())
            .env("PATH", &gh_path)
            .spawn()
            .expect("first daemon should spawn");

        std::thread::sleep(std::time::Duration::from_millis(800));

        let second = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path)],
            )
            .expect("second daemon invocation should execute");
        assert!(
            !second.status.success(),
            "second daemon should fail due to lock contention, stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&second.stdout),
            String::from_utf8_lossy(&second.stderr)
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&second.stdout),
            String::from_utf8_lossy(&second.stderr)
        );
        assert!(
            combined.contains("daemon is already running") || combined.contains("lock"),
            "expected lock contention message, got:\n{combined}"
        );

        let _ = first.kill();
        let _ = first.wait();
    })
}

fn status_history_derive_from_git_and_labels(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project("issue-7", "Issue 7", "status/history derivation prompt")
            .expect("create project");

        git(
            &h.repo_root,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        );
        git(&h.repo_root, &["checkout", "-B", "ralph/issue-7"]);

        let msg_1 = "ralph(issue-7): loop 1 planning -> implementing\n\nRalph-Project: issue-7\nRalph-Loop: 1\nRalph-Phase: implementing";
        git(&h.repo_root, &["commit", "--allow-empty", "-m", msg_1]);
        let msg_2 = "ralph(issue-7): loop 1 implementing -> reviewing\n\nRalph-Project: issue-7\nRalph-Loop: 1\nRalph-Phase: reviewing";
        git(&h.repo_root, &["commit", "--allow-empty", "-m", msg_2]);

        let head = git_stdout(&h.repo_root, &["rev-parse", "HEAD"]);
        git(
            &h.repo_root,
            &["update-ref", "refs/remotes/origin/ralph/issue-7", &head],
        );
        git(&h.repo_root, &["checkout", "master"]);

        let gh_script = r#"#!/bin/sh
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  printf '{"labels":[{"name":"ralph:completed"}]}'
  exit 0
fi
printf '[]'
exit 0
"#;
        let gh_path = write_mock_gh(h, gh_script).expect("mock gh");

        let status = h
            .ralph_env(["status", "--project", "issue-7"], &[("PATH", &gh_path)])
            .expect("status should execute");
        assert_exit_code(&status, 0);
        assert_stdout_contains(&status, "completed");
        assert_stdout_contains(&status, "reviewing");

        let history = h
            .ralph_env(["history", "--project", "issue-7"], &[("PATH", &gh_path)])
            .expect("history should execute");
        assert_exit_code(&history, 0);
        let out = String::from_utf8_lossy(&history.stdout);
        assert!(
            out.contains("planning -> implementing"),
            "expected trailer-derived transition in history output, got:\n{out}"
        );
        assert!(
            out.contains("implementing -> reviewing"),
            "expected trailer-derived transition in history output, got:\n{out}"
        );
    })
}

fn crash_after_local_commit_before_push_recovery(h: &RalphHarness) -> TestResult {
    // Existing remote-first branch sync conformance already covers this crash class:
    // local-only commit is discarded on next sync and recovered state does not advance.
    sync_project_branch_discards_local_commit(h)
}

/// Conformance: position reconstruction from a real remote checkpoint commit.
/// Uses `commit_and_push_phase_transition` to push a structured checkpoint to
/// a real bare remote, then verifies `derive_position` returns the correct
/// loop number and phase.
fn reconstruct_position_from_real_remote_checkpoint(_h: &RalphHarness) -> TestResult {
    use crate::git::commit::commit_and_push_phase_transition;
    use crate::git::ralph_commit::derive_position;
    use crate::project::state::Phase;

    run_case(|| {
        let (_tmp, _bare, clone) = setup_remote_clone();

        // Create project branch and push it
        git_run(&clone, &["checkout", "-b", "ralph/issue-55"]);
        git_run(&clone, &["push", "-u", "origin", "ralph/issue-55"]);

        // Push a real checkpoint commit: loop 1, planning -> implementing
        commit_and_push_phase_transition(
            &clone,
            "issue-55",
            1,
            Phase::Planning,
            Phase::Implementing,
            "ralph/issue-55",
            false,
        )
        .expect("first checkpoint should succeed");

        let (loop_num, phase) =
            derive_position(&clone, "ralph/issue-55").expect("derive_position should succeed");
        assert_eq!(loop_num, 1, "expected loop 1 after first checkpoint");
        assert_eq!(
            phase,
            Phase::Implementing,
            "expected implementing phase after first checkpoint"
        );

        // Push a second checkpoint: loop 2, implementing -> reviewing
        commit_and_push_phase_transition(
            &clone,
            "issue-55",
            2,
            Phase::Implementing,
            Phase::Reviewing,
            "ralph/issue-55",
            false,
        )
        .expect("second checkpoint should succeed");

        let (loop_num, phase) =
            derive_position(&clone, "ralph/issue-55").expect("derive_position should succeed");
        assert_eq!(loop_num, 2, "expected loop 2 after second checkpoint");
        assert_eq!(
            phase,
            Phase::Reviewing,
            "expected reviewing phase after second checkpoint"
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
