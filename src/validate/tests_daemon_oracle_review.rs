use super::*;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::daemon::oracle_review::OracleReviewState;
use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use serde_json::Value;

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(err) => TestResult::Fail(super::panic_message(err)),
    }
}

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "daemon_oracle_review::config_defaults",
            func: config_defaults,
        },
        ConformanceTest {
            name: "daemon_oracle_review::config_set_get_roundtrip",
            func: config_set_get_roundtrip,
        },
        ConformanceTest {
            name: "daemon_oracle_review::bounds_rejection",
            func: bounds_rejection,
        },
        ConformanceTest {
            name: "daemon_oracle_review::config_show_includes_fields",
            func: config_show_includes_fields,
        },
        ConformanceTest {
            name: "daemon_oracle_review::disabled_noop",
            func: disabled_noop,
        },
        ConformanceTest {
            name: "daemon_oracle_review::eligible_pr_reviewed",
            func: eligible_pr_reviewed,
        },
        ConformanceTest {
            name: "daemon_oracle_review::draft_prs_skipped",
            func: draft_prs_skipped,
        },
        ConformanceTest {
            name: "daemon_oracle_review::dedup_same_sha_and_rereview_on_change",
            func: dedup_same_sha_and_rereview_on_change,
        },
        ConformanceTest {
            name: "daemon_oracle_review::author_allowlist_enforced",
            func: author_allowlist_enforced,
        },
        ConformanceTest {
            name: "daemon_oracle_review::author_allowlist_case_insensitive",
            func: author_allowlist_case_insensitive,
        },
        ConformanceTest {
            name: "daemon_oracle_review::per_cycle_max_enforced",
            func: per_cycle_max_enforced,
        },
        ConformanceTest {
            name: "daemon_oracle_review::existing_bot_marker_skips_oracle",
            func: existing_bot_marker_skips_oracle,
        },
        ConformanceTest {
            name: "daemon_oracle_review::oracle_timeout_does_not_advance_state",
            func: oracle_timeout_does_not_advance_state,
        },
        ConformanceTest {
            name: "daemon_oracle_review::oracle_non_zero_exit_isolated",
            func: oracle_non_zero_exit_isolated,
        },
        ConformanceTest {
            name: "daemon_oracle_review::missing_oracle_binary_does_not_advance_state",
            func: missing_oracle_binary_does_not_advance_state,
        },
        ConformanceTest {
            name: "daemon_oracle_review::oracle_spawn_failure_isolated",
            func: oracle_spawn_failure_isolated,
        },
        ConformanceTest {
            name: "daemon_oracle_review::comment_post_failure_does_not_advance_state",
            func: comment_post_failure_does_not_advance_state,
        },
        ConformanceTest {
            name: "daemon_oracle_review::comment_readback_failure_still_advances_state",
            func: comment_readback_failure_still_advances_state,
        },
        ConformanceTest {
            name: "daemon_oracle_review::overflow_warning_logged",
            func: overflow_warning_logged,
        },
    ]
}

fn config_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        assert_eq!(
            h.ralph_ok([
                "config",
                "get",
                "workspace.daemon_oracle_review_enabled",
                "--global"
            ])
            .expect("get enabled")
            .trim(),
            "false"
        );
        assert_eq!(
            h.ralph_ok([
                "config",
                "get",
                "workspace.daemon_oracle_review_timeout_secs",
                "--global"
            ])
            .expect("get timeout")
            .trim(),
            "900"
        );
        assert_eq!(
            h.ralph_ok([
                "config",
                "get",
                "workspace.daemon_oracle_review_authors",
                "--global"
            ])
            .expect("get authors")
            .trim(),
            "[]"
        );
        assert_eq!(
            h.ralph_ok([
                "config",
                "get",
                "workspace.daemon_oracle_review_max_per_cycle",
                "--global",
            ])
            .expect("get max per cycle")
            .trim(),
            "3"
        );
    })
}

fn config_set_get_roundtrip(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project("oracle-review", "Oracle Review", "prompt")
            .expect("project");

        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_enabled",
            "true",
            "--global",
        ])
        .expect("set enabled");
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_timeout_secs",
            "42",
            "--global",
        ])
        .expect("set timeout");
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_authors",
            r#"["Alice","bob"]"#,
            "--global",
        ])
        .expect("set authors");
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_max_per_cycle",
            "7",
            "--global",
        ])
        .expect("set max");

        assert_eq!(
            h.ralph_ok(["config", "get", "daemon.oracle_review_enabled"])
                .expect("project enabled")
                .trim(),
            "true"
        );
        assert_eq!(
            h.ralph_ok(["config", "get", "daemon.oracle_review_timeout_secs"])
                .expect("project timeout")
                .trim(),
            "42"
        );
        let authors = h
            .ralph_ok(["config", "get", "daemon.oracle_review_authors"])
            .expect("project authors");
        let parsed: Value = serde_json::from_str(&authors).expect("authors json");
        assert_eq!(
            parsed,
            serde_json::json!(["Alice", "bob"]),
            "authors should roundtrip"
        );
        assert_eq!(
            h.ralph_ok(["config", "get", "daemon.oracle_review_max_per_cycle"])
                .expect("project max")
                .trim(),
            "7"
        );
    })
}

fn bounds_rejection(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let timeout = h
            .ralph([
                "config",
                "set",
                "workspace.daemon_oracle_review_timeout_secs",
                "0",
                "--global",
            ])
            .expect("timeout rejection");
        assert_exit_code(&timeout, 2);
        assert!(
            combined_output(&timeout).contains("must be > 0"),
            "expected > 0 timeout validation, got:\n{}",
            combined_output(&timeout)
        );

        let cap = h
            .ralph([
                "config",
                "set",
                "workspace.daemon_oracle_review_max_per_cycle",
                "0",
                "--global",
            ])
            .expect("cap rejection");
        assert_exit_code(&cap, 2);
        assert!(
            combined_output(&cap).contains("must be > 0"),
            "expected > 0 cap validation, got:\n{}",
            combined_output(&cap)
        );
    })
}

fn config_show_includes_fields(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");
        h.create_project("oracle-show", "Oracle Show", "prompt")
            .expect("project");
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_enabled",
            "true",
            "--global",
        ])
        .expect("set enabled");
        h.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_timeout_secs",
            "99",
            "--global",
        ])
        .expect("set timeout");

        let shown = h.ralph_ok(["config", "show"]).expect("config show");
        let parsed: Value = serde_json::from_str(&shown).expect("show json");
        assert_eq!(parsed["daemon"]["oracle_review_enabled"], Value::Bool(true));
        assert_eq!(
            parsed["daemon"]["oracle_review_timeout_secs"],
            Value::from(99)
        );
        assert_eq!(
            parsed["daemon"]["oracle_review_authors"],
            serde_json::json!([])
        );
        assert_eq!(
            parsed["daemon"]["oracle_review_max_per_cycle"],
            Value::from(3)
        );
    })
}

fn disabled_noop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let command_log = path_in_temp(&dh, "gh-command.log");
        let path_env = script_path_env(&gh_path);
        let git_bin = system_git_bin();
        let open_prs = open_prs_json(&[open_pr(11, "sha-11", false, "alice")]);

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                ("MOCK_GH_OPEN_PRS".into(), open_prs),
                (
                    "MOCK_GH_COMMAND_LOG".into(),
                    command_log.to_string_lossy().into_owned(),
                ),
            ],
        );

        assert_exit_code(&output, 0);
        let command_log_contents = fs::read_to_string(&command_log).unwrap_or_default();
        assert!(
            !command_log_contents.contains("pr list"),
            "disabled oracle review should not even list PRs, got:\n{command_log_contents}"
        );
        assert!(
            load_oracle_state(&dh).is_none(),
            "disabled phase should not create state"
        );
    })
}

fn eligible_pr_reviewed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let oracle_log = path_in_temp(&dh, "oracle.log");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();
        let open_prs = open_prs_json(&[open_pr(11, "sha-11", false, "alice")]);

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                ("MOCK_GH_OPEN_PRS".into(), open_prs),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_LOG".into(),
                    oracle_log.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_OUTPUT".into(),
                    "Found a likely bug in the diff".into(),
                ),
            ],
        );

        assert_exit_code(&output, 0);

        let state = load_oracle_state(&dh).expect("state should exist");
        assert_eq!(state.reviewed.get("11"), Some(&"sha-11".to_owned()));

        let comments = read_comment_log(&comments_file);
        assert_eq!(comments.len(), 1, "should post one top-level PR comment");
        let body = comments[0]["body"].as_str().expect("comment body");
        assert!(
            body.starts_with("<!-- ralph:oracle-review:11:sha-11 -->\n"),
            "comment must start with marker, got:\n{body}"
        );
        assert!(
            body.contains("Found a likely bug in the diff"),
            "oracle body should be included, got:\n{body}"
        );

        let oracle_log_contents = fs::read_to_string(&oracle_log).expect("oracle log");
        let prompt = oracle_log_contents
            .lines()
            .find_map(|line| line.strip_prefix("prompt="))
            .expect("oracle log should include prompt");
        assert_eq!(
            prompt,
            "You are a senior code reviewer. Review this PR diff for bugs, security issues, performance problems, and code quality. Be concise and actionable. Focus on substantive issues, not style nits.",
            "oracle prompt should match spec exactly"
        );
        let diff_path = oracle_log_contents
            .lines()
            .find_map(|line| line.strip_prefix("file="))
            .expect("oracle log should include diff path");
        assert!(
            !Path::new(diff_path).exists(),
            "temp diff file should be cleaned up: {diff_path}"
        );
    })
}

fn draft_prs_skipped(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let oracle_log = path_in_temp(&dh, "oracle.log");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();
        let open_prs = open_prs_json(&[open_pr(11, "sha-11", true, "alice")]);

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                ("MOCK_GH_OPEN_PRS".into(), open_prs),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_LOG".into(),
                    oracle_log.to_string_lossy().into_owned(),
                ),
            ],
        );

        assert_exit_code(&output, 0);
        assert!(read_comment_log(&comments_file).is_empty());
        assert!(
            fs::read_to_string(&oracle_log)
                .unwrap_or_default()
                .is_empty(),
            "draft PR should never invoke oracle"
        );
        assert!(load_oracle_state(&dh).is_none());
    })
}

fn dedup_same_sha_and_rereview_on_change(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let oracle_log = path_in_temp(&dh, "oracle.log");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        save_oracle_state(&dh, &[("11", "sha-11")]);
        let first = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_LOG".into(),
                    oracle_log.to_string_lossy().into_owned(),
                ),
            ],
        );
        assert_exit_code(&first, 0);
        assert!(read_comment_log(&comments_file).is_empty());
        assert!(fs::read_to_string(&oracle_log)
            .unwrap_or_default()
            .is_empty());

        let second = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-22", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_LOG".into(),
                    oracle_log.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_OUTPUT".into(), "fresh review".into()),
            ],
        );
        assert_exit_code(&second, 0);
        let state = load_oracle_state(&dh).expect("state");
        assert_eq!(state.reviewed.get("11"), Some(&"sha-22".to_owned()));
        assert_eq!(read_comment_log(&comments_file).len(), 1);
    })
}

fn author_allowlist_enforced(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_authors",
            r#"["bob"]"#,
            "--global",
        ])
        .expect("set authors");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let oracle_log = path_in_temp(&dh, "oracle.log");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_LOG".into(),
                    oracle_log.to_string_lossy().into_owned(),
                ),
            ],
        );

        assert_exit_code(&output, 0);
        assert!(read_comment_log(&comments_file).is_empty());
        assert!(fs::read_to_string(&oracle_log)
            .unwrap_or_default()
            .is_empty());
    })
}

fn author_allowlist_case_insensitive(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_authors",
            r#"["ALICE"]"#,
            "--global",
        ])
        .expect("set authors");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_OUTPUT".into(), "case insensitive hit".into()),
            ],
        );

        assert_exit_code(&output, 0);
        assert_eq!(read_comment_log(&comments_file).len(), 1);
    })
}

fn per_cycle_max_enforced(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_max_per_cycle",
            "2",
            "--global",
        ])
        .expect("set cap");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[
                        open_pr(11, "sha-11", false, "alice"),
                        open_pr(12, "sha-12", false, "alice"),
                        open_pr(13, "sha-13", false, "alice"),
                    ]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_OUTPUT".into(), "cap test".into()),
            ],
        );

        assert_exit_code(&output, 0);
        let comments = read_comment_log(&comments_file);
        assert_eq!(comments.len(), 2, "only two comments should be posted");
        let state = load_oracle_state(&dh).expect("state");
        assert_eq!(state.reviewed.len(), 2, "only two PRs should be persisted");
    })
}

fn existing_bot_marker_skips_oracle(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let oracle_log = path_in_temp(&dh, "oracle.log");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        write_comment_log(
            &comments_file,
            &[comment_line(
                5001,
                "ralph-bot",
                "<!-- ralph:oracle-review:11:sha-11 -->\nexisting review",
            )],
        );

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_LOG".into(),
                    oracle_log.to_string_lossy().into_owned(),
                ),
            ],
        );

        assert_exit_code(&output, 0);
        assert!(fs::read_to_string(&oracle_log)
            .unwrap_or_default()
            .is_empty());
        let state = load_oracle_state(&dh).expect("state should self-heal");
        assert_eq!(state.reviewed.get("11"), Some(&"sha-11".to_owned()));
    })
}

fn oracle_timeout_does_not_advance_state(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_timeout_secs",
            "1",
            "--global",
        ])
        .expect("set timeout");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_MODE".into(), "timeout".into()),
            ],
        );

        assert_exit_code(&output, 0);
        assert!(
            combined_output(&output).contains("oracle timeout"),
            "timeout warning should be visible, got:\n{}",
            combined_output(&output)
        );
        assert!(read_comment_log(&comments_file).is_empty());
        assert!(load_oracle_state(&dh).is_none());
    })
}

fn oracle_non_zero_exit_isolated(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let fail_first_state = path_in_temp(&dh, "oracle-fail-first.marker");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[
                        open_pr(11, "sha-11", false, "alice"),
                        open_pr(12, "sha-12", false, "alice"),
                    ]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_FAIL_FIRST_MODE".into(), "exit".into()),
                (
                    "MOCK_ORACLE_FAIL_FIRST_STATE_FILE".into(),
                    fail_first_state.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_OUTPUT".into(), "second review succeeds".into()),
            ],
        );

        assert_exit_code(&output, 0);
        let combined = combined_output(&output);
        assert!(
            combined.contains("PR #11 oracle exit"),
            "first PR failure should be logged, got:\n{combined}"
        );
        let comments = read_comment_log(&comments_file);
        assert_eq!(comments.len(), 1, "second PR should still be processed");
        let state = load_oracle_state(&dh).expect("state");
        assert!(
            !state.reviewed.contains_key("11"),
            "failed PR should not advance state"
        );
        assert_eq!(state.reviewed.get("12"), Some(&"sha-12".to_owned()));
    })
}

fn missing_oracle_binary_does_not_advance_state(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let missing_oracle = write_missing_oracle_path(&dh).expect("missing oracle path");
        let path_env = isolated_script_path_env(&missing_oracle);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
            ],
        );

        assert_exit_code(&output, 0);
        assert!(
            combined_output(&output).contains("oracle spawn"),
            "missing oracle binary should surface as spawn failure, got:\n{}",
            combined_output(&output)
        );
        assert!(read_comment_log(&comments_file).is_empty());
        assert!(load_oracle_state(&dh).is_none());
    })
}

fn oracle_spawn_failure_isolated(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let oracle_dir = path_in_temp(&dh, "oracle-bin");
        fs::create_dir_all(&oracle_dir).expect("mkdir oracle dir");
        let deferred_target = oracle_dir.join("oracle");
        let path_env = isolated_script_path_env(&deferred_target);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[
                        open_pr(11, "sha-11", false, "alice"),
                        open_pr(12, "sha-12", false, "alice"),
                    ]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_GH_CREATE_ORACLE_ON_DIFF_PR".into(), "12".into()),
                (
                    "MOCK_GH_CREATE_ORACLE_TARGET".into(),
                    deferred_target.to_string_lossy().into_owned(),
                ),
                ("MOCK_ORACLE_OUTPUT".into(), "second review succeeds".into()),
            ],
        );

        assert_exit_code(&output, 0);
        let combined = combined_output(&output);
        assert!(
            combined.contains("PR #11 oracle spawn"),
            "first PR spawn failure should be logged, got:\n{combined}"
        );
        assert!(
            combined.contains("failed to spawn command"),
            "spawn failure should come from the process helper, got:\n{combined}"
        );
        let comments = read_comment_log(&comments_file);
        assert_eq!(comments.len(), 1, "second PR should still be processed");
        let state = load_oracle_state(&dh).expect("state");
        assert!(
            !state.reviewed.contains_key("11"),
            "spawn-failed PR should not advance state"
        );
        assert_eq!(state.reviewed.get("12"), Some(&"sha-12".to_owned()));
    })
}

fn comment_post_failure_does_not_advance_state(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[open_pr(11, "sha-11", false, "alice")]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_GH_FAIL_COMMENT_PR".into(), "11".into()),
            ],
        );

        assert_exit_code(&output, 0);
        assert!(
            combined_output(&output).contains("comment post failed"),
            "comment failure should be logged, got:\n{}",
            combined_output(&output)
        );
        assert!(read_comment_log(&comments_file).is_empty());
        assert!(load_oracle_state(&dh).is_none());
    })
}

fn comment_readback_failure_still_advances_state(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_max_per_cycle",
            "2",
            "--global",
        ])
        .expect("set cap");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let comments_file = path_in_temp(&dh, "comments.jsonl");
        let readback_flag = path_in_temp(&dh, "comment-readback.flag");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![
                (
                    "MOCK_GH_OPEN_PRS".into(),
                    open_prs_json(&[
                        open_pr(11, "sha-11", false, "alice"),
                        open_pr(12, "sha-12", false, "alice"),
                        open_pr(13, "sha-13", false, "alice"),
                    ]),
                ),
                (
                    "MOCK_GH_COMMENTS_FILE".into(),
                    comments_file.to_string_lossy().into_owned(),
                ),
                ("MOCK_GH_FAIL_COMMENT_READBACK_PR".into(), "11".into()),
                (
                    "MOCK_GH_FAIL_COMMENT_READBACK_STATE_FILE".into(),
                    readback_flag.to_string_lossy().into_owned(),
                ),
                (
                    "MOCK_ORACLE_OUTPUT".into(),
                    "readback failure recovery".into(),
                ),
            ],
        );

        assert_exit_code(&output, 0);
        let state = load_oracle_state(&dh).expect("state");
        assert_eq!(state.reviewed.get("11"), Some(&"sha-11".to_owned()));
        assert_eq!(state.reviewed.get("12"), Some(&"sha-12".to_owned()));
        assert!(
            !state.reviewed.contains_key("13"),
            "posted PRs should count toward the per-cycle cap"
        );

        let comments = read_comment_log(&comments_file);
        assert_eq!(
            comments.len(),
            2,
            "readback failure after a successful post should still consume the cap"
        );
        let bodies: Vec<&str> = comments
            .iter()
            .map(|comment| comment["body"].as_str().expect("comment body"))
            .collect();
        assert!(
            bodies
                .iter()
                .any(|body| body.starts_with("<!-- ralph:oracle-review:11:sha-11 -->\n")),
            "the successfully posted comment must still be present"
        );
        assert!(
            bodies
                .iter()
                .any(|body| body.starts_with("<!-- ralph:oracle-review:12:sha-12 -->\n")),
            "subsequent eligible PRs should still be processed"
        );
        assert!(
            bodies
                .iter()
                .all(|body| !body.starts_with("<!-- ralph:oracle-review:13:sha-13 -->\n")),
            "the third PR should be skipped once the cap is reached"
        );
    })
}

fn overflow_warning_logged(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = new_daemon_harness(h);
        dh.init_workspace().expect("init failed");
        enable_oracle_review(&dh);
        dh.ralph_ok([
            "config",
            "set",
            "workspace.daemon_oracle_review_authors",
            r#"["nobody"]"#,
            "--global",
        ])
        .expect("set authors");

        let gh_path = write_oracle_review_mock_gh(&dh).expect("mock gh");
        let oracle_path = write_oracle_mock(&dh).expect("mock oracle");
        let path_env = script_path_env(&oracle_path);
        let git_bin = system_git_bin();
        let open_prs = open_prs_json(
            &(1..=100)
                .map(|number| open_pr(number, &format!("sha-{number}"), false, "alice"))
                .collect::<Vec<_>>(),
        );

        let output = run_daemon_once(
            &dh,
            &gh_path,
            &git_bin,
            &path_env,
            vec![("MOCK_GH_OPEN_PRS".into(), open_prs)],
        );

        assert_exit_code(&output, 0);
        assert!(
            combined_output(&output).contains(
                "warning: oracle review: gh pr list returned 100 PRs, results may be truncated"
            ),
            "overflow warning must match exactly, got:\n{}",
            combined_output(&output)
        );
    })
}

fn new_daemon_harness(h: &RalphHarness) -> RalphHarness {
    RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness")
}

fn enable_oracle_review(dh: &RalphHarness) {
    dh.ralph_ok([
        "config",
        "set",
        "workspace.daemon_oracle_review_enabled",
        "true",
        "--global",
    ])
    .expect("enable oracle review");
}

fn run_daemon_once(
    dh: &RalphHarness,
    gh_bin: &Path,
    git_bin: &str,
    path_env: &str,
    mut env_vars: Vec<(String, String)>,
) -> Output {
    let git_wrapper = write_git_wrapper_script(dh, git_bin).expect("mock git wrapper");
    let git_wrapper_dir = script_dir(&git_wrapper);
    env_vars.push(("PATH".into(), format!("{git_wrapper_dir}:{path_env}")));
    env_vars.push(("MOCK_GH_ISSUES".into(), "[]".into()));

    let env_refs: Vec<(&str, &str)> = env_vars
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();

    dh.daemon_env(
        vec![
            "daemon".to_owned(),
            "start".to_owned(),
            "--repo".to_owned(),
            "acme/widgets".to_owned(),
            "--single-iteration".to_owned(),
            "--gh-bin".to_owned(),
            gh_bin.to_string_lossy().into_owned(),
            "--git-bin".to_owned(),
            git_bin.to_owned(),
        ],
        &env_refs,
    )
    .expect("daemon start")
}

fn write_git_wrapper_script(dh: &RalphHarness, git_bin: &str) -> crate::Result<PathBuf> {
    let git_for_wrapper = git_bin.replace('\\', "\\\\").replace('"', "\\\"");
    dh.write_mock_script(
        "git",
        &format!("#!/bin/sh\nexec \"{git_for_wrapper}\" \"$@\"\n"),
    )
}

fn write_bash_wrapper_script(
    dh: &RalphHarness,
    name: &str,
    bash_content: &str,
) -> crate::Result<PathBuf> {
    let bash_bin = system_command_bin("bash");
    let inner_name = format!("{name}.inner.bash");
    let inner_path = dh.write_mock_script(&inner_name, bash_content)?;
    let inner_for_wrapper = inner_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let bash_for_wrapper = bash_bin.replace('\\', "\\\\").replace('"', "\\\"");
    let wrapper =
        format!("#!/bin/sh\nexec \"{bash_for_wrapper}\" \"{inner_for_wrapper}\" \"$@\"\n");
    dh.write_mock_script(name, &wrapper)
}

fn write_missing_oracle_path(dh: &RalphHarness) -> crate::Result<PathBuf> {
    let oracle_dir = path_in_temp(dh, "missing-oracle-bin");
    fs::create_dir_all(&oracle_dir)?;
    Ok(oracle_dir.join("oracle"))
}

fn write_oracle_review_mock_gh(dh: &RalphHarness) -> crate::Result<PathBuf> {
    let chmod_bin = system_command_bin("chmod")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = r#"set -euo pipefail

json_escape() {
  local value="${1-}"
  value=${value//\\/\\\\}
  value=${value//\"/\\\"}
  value=${value//$'\n'/\\n}
  value=${value//$'\r'/\\r}
  value=${value//$'\t'/\\t}
  printf '%s' "$value"
}

log_command() {
  if [[ -n "${MOCK_GH_COMMAND_LOG:-}" ]]; then
    printf '%s\n' "$*" >> "$MOCK_GH_COMMAND_LOG"
  fi
}

emit_comments() {
  local file="${MOCK_GH_COMMENTS_FILE:-}"
  printf '{"comments":['
  local first=1
  if [[ -n "$file" && -f "$file" ]]; then
    while IFS= read -r line || [[ -n "${line:-}" ]]; do
      [[ -z "${line:-}" ]] && continue
      if [[ $first -eq 0 ]]; then
        printf ','
      fi
      first=0
      printf '%s' "$line"
    done < "$file"
  fi
  printf ']}'
}

append_comment() {
  local issue_number="$1"
  local body="$2"
  local file="${MOCK_GH_COMMENTS_FILE:-}"
  if [[ -n "${MOCK_GH_COMMENT_POST_LOG:-}" ]]; then
    printf '%s\t%s\n' "$issue_number" "$body" >> "$MOCK_GH_COMMENT_POST_LOG"
  fi
  [[ -z "$file" ]] && return 0
  : >> "$file"
  local next_id=1000
  while IFS= read -r existing || [[ -n "${existing:-}" ]]; do
    [[ -n "${existing:-}" ]] && next_id=$((next_id + 1))
  done < "$file"
  local body_json
  body_json=$(json_escape "$body")
  printf '{"id":%s,"author":{"login":"%s"},"body":"%s","createdAt":"2026-01-01T00:00:00Z"}\n' \
    "$next_id" "${MOCK_GH_BOT_LOGIN:-ralph-bot}" "$body_json" >> "$file"
}

log_command "$@"

case "${1:-}" in
  issue)
    case "${2:-}" in
      list)
        printf '%s' "${MOCK_GH_ISSUES:-[]}"
        exit 0
        ;;
      view)
        issue_number="${3:-0}"
        want_comments=0
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          [[ "$arg" == "comments" ]] && want_comments=1
          [[ "$arg" == "labels" ]] && want_labels=1
          [[ "$arg" == "title,body" ]] && want_title_body=1
        done
        if [[ $want_comments -eq 1 ]]; then
          if [[ "${MOCK_GH_FAIL_COMMENT_READBACK_PR:-}" == "$issue_number" \
             && -n "${MOCK_GH_FAIL_COMMENT_READBACK_STATE_FILE:-}" \
             && -e "${MOCK_GH_FAIL_COMMENT_READBACK_STATE_FILE:-}" ]]; then
            rm -f "$MOCK_GH_FAIL_COMMENT_READBACK_STATE_FILE"
            echo "mock comment readback failure" >&2
            exit 1
          fi
          emit_comments
          exit 0
        fi
        if [[ $want_labels -eq 1 ]]; then
          printf '{"labels":[]}'
          exit 0
        fi
        if [[ $want_title_body -eq 1 ]]; then
          printf '{"title":"Mock issue","body":"Mock body"}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment)
        issue_number="${3:-0}"
        body=""
        prev=""
        for arg in "$@"; do
          if [[ "$prev" == "--body" ]]; then
            body="$arg"
          fi
          prev="$arg"
        done
        if [[ "${MOCK_GH_FAIL_COMMENT_PR:-}" == "$issue_number" ]]; then
          echo "mock comment failure" >&2
          exit 1
        fi
        append_comment "$issue_number" "$body"
        if [[ "${MOCK_GH_FAIL_COMMENT_READBACK_PR:-}" == "$issue_number" \
           && -n "${MOCK_GH_FAIL_COMMENT_READBACK_STATE_FILE:-}" ]]; then
          : > "$MOCK_GH_FAIL_COMMENT_READBACK_STATE_FILE"
        fi
        exit 0
        ;;
      edit)
        exit 0
        ;;
    esac
    ;;
  pr)
    case "${2:-}" in
      list)
        printf '%s' "${MOCK_GH_OPEN_PRS:-[]}"
        exit 0
        ;;
      diff)
        pr_number="${3:-0}"
        if [[ "${MOCK_GH_DIFF_FAIL_PR:-}" == "$pr_number" ]]; then
          echo "mock diff failure" >&2
          exit 1
        fi
        if [[ -n "${MOCK_GH_CREATE_ORACLE_ON_DIFF_PR:-}" && "${MOCK_GH_CREATE_ORACLE_ON_DIFF_PR:-}" == "$pr_number" ]]; then
          target="${MOCK_GH_CREATE_ORACLE_TARGET:-}"
          if [[ -n "$target" && ! -e "$target" ]]; then
            printf '%s\n' \
              '#!/bin/sh' \
              'set -eu' \
              '' \
              'prompt=""' \
              'write_output=""' \
              'prev=""' \
              'for arg in "$@"; do' \
              '  if [ "$prev" = "--prompt" ]; then' \
              '    prompt="$arg"' \
              '  fi' \
              '  if [ "$prev" = "--write-output" ]; then' \
              '    write_output="$arg"' \
              '  fi' \
              '  prev="$arg"' \
              'done' \
              '' \
              'output="${MOCK_ORACLE_OUTPUT:-Oracle review body}"' \
              'if [ -n "$write_output" ]; then' \
              '  printf '\''%s'\'' "$output" > "$write_output"' \
              'fi' \
              'printf '\''%s'\'' "$output"' \
              > "$target"
            "__CHMOD_BIN__" +x "$target"
          fi
        fi
        printf 'diff for pr %s\n%s' "$pr_number" "${MOCK_GH_DIFF_TEXT:-}"
        exit 0
        ;;
      create)
        printf 'https://github.com/acme/widgets/pull/1\n'
        exit 0
        ;;
      edit|ready|close)
        exit 0
        ;;
      view)
        printf '{"isDraft":false}'
        exit 0
        ;;
    esac
    ;;
  api)
    if [[ "${2:-}" == "user" ]]; then
      printf '%s\n' "${MOCK_GH_BOT_LOGIN:-ralph-bot}"
      exit 0
    fi
    ;;
  label)
    if [[ "${2:-}" == "create" ]]; then
      exit 0
    fi
    ;;
  repo)
    if [[ "${2:-}" == "view" ]]; then
      printf 'acme/widgets\n'
      exit 0
    fi
    ;;
esac

echo "unexpected gh invocation: $*" >&2
exit 1
"#;
    let script = script.replace("__CHMOD_BIN__", &chmod_bin);
    write_bash_wrapper_script(dh, "gh", &script)
}

fn write_oracle_mock(dh: &RalphHarness) -> crate::Result<PathBuf> {
    write_bash_wrapper_script(
        dh,
        "oracle",
        r#"set -euo pipefail

prompt=""
file=""
write_output=""
prev=""
for arg in "$@"; do
  if [[ "$prev" == "--prompt" ]]; then
    prompt="$arg"
  fi
  if [[ "$prev" == "--file" ]]; then
    file="$arg"
  fi
  if [[ "$prev" == "--write-output" ]]; then
    write_output="$arg"
  fi
  prev="$arg"
done

if [[ -n "${MOCK_ORACLE_LOG:-}" ]]; then
  escaped_prompt="${prompt//$'\n'/\\n}"
  printf 'prompt=%s\nfile=%s\nwrite_output=%s\n' \
    "$escaped_prompt" "$file" "$write_output" >> "$MOCK_ORACLE_LOG"
fi

mode="${MOCK_ORACLE_MODE:-success}"
if [[ -n "${MOCK_ORACLE_FAIL_FIRST_MODE:-}" ]]; then
  state_file="${MOCK_ORACLE_FAIL_FIRST_STATE_FILE:-}"
  if [[ -n "$state_file" && ! -e "$state_file" ]]; then
    : > "$state_file"
    mode="${MOCK_ORACLE_FAIL_FIRST_MODE}"
  fi
fi

case "$mode" in
  success)
    output="${MOCK_ORACLE_OUTPUT:-Oracle review body}"
    if [[ -n "$write_output" ]]; then
      printf '%s' "$output" > "$write_output"
    fi
    printf '%s' "$output"
    ;;
  exit)
    echo "mock oracle exit" >&2
    exit 7
    ;;
  timeout)
    while :; do :; done
    ;;
  *)
    echo "unexpected oracle mode: $mode" >&2
    exit 2
    ;;
esac
"#,
    )
}

fn system_command_bin(command: &str) -> String {
    let output = Command::new("sh")
        .args(["-c", &format!("command -v {command}")])
        .output()
        .unwrap_or_else(|err| panic!("resolve {command} path: {err}"));
    assert!(
        output.status.success(),
        "command -v {command} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn system_git_bin() -> String {
    system_command_bin("git")
}

fn script_dir(script_path: &Path) -> String {
    script_path
        .parent()
        .expect("script should have parent")
        .to_string_lossy()
        .into_owned()
}

fn script_path_env(script_path: &Path) -> String {
    let script_dir = script_dir(script_path);
    let current_path = std::env::var("PATH").unwrap_or_default();
    format!("{script_dir}:{current_path}")
}

fn isolated_script_path_env(script_path: &Path) -> String {
    script_dir(script_path)
}

fn open_pr(number: u32, head_sha: &str, is_draft: bool, author: &str) -> String {
    format!(
        r#"{{"number":{number},"headRefOid":"{head_sha}","isDraft":{is_draft},"author":{{"login":"{author}"}}}}"#
    )
}

fn open_prs_json(prs: &[String]) -> String {
    format!("[{}]", prs.join(","))
}

fn comment_line(id: u64, author: &str, body: &str) -> String {
    format!(
        r#"{{"id":{id},"author":{{"login":"{author}"}},"body":"{}","createdAt":"2026-01-01T00:00:00Z"}}"#,
        escape_json(body)
    )
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn path_in_temp(dh: &RalphHarness, name: &str) -> PathBuf {
    dh.temp_dir.path().join(name)
}

fn write_comment_log(path: &Path, comments: &[String]) {
    if comments.is_empty() {
        fs::write(path, "").expect("write empty comments");
    } else {
        fs::write(path, comments.join("\n")).expect("write comments");
    }
}

fn read_comment_log(path: &Path) -> Vec<Value> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("comment line json"))
        .collect()
}

fn oracle_state_path(dh: &RalphHarness) -> PathBuf {
    dh.repo_root
        .join(".ralph")
        .join("daemon")
        .join("oracle-review-state")
        .join("state.json")
}

fn load_oracle_state(dh: &RalphHarness) -> Option<OracleReviewState> {
    let path = oracle_state_path(dh);
    let raw = fs::read_to_string(path).ok()?;
    Some(serde_json::from_str(&raw).expect("oracle review state json"))
}

fn save_oracle_state(dh: &RalphHarness, entries: &[(&str, &str)]) {
    let mut reviewed = HashMap::new();
    for (pr, sha) in entries {
        reviewed.insert((*pr).to_owned(), (*sha).to_owned());
    }
    let state = OracleReviewState { reviewed };
    let path = oracle_state_path(dh);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir state dir");
    fs::write(
        path,
        serde_json::to_string_pretty(&state).expect("serialize"),
    )
    .expect("write");
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
