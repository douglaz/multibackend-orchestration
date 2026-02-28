use std::fs;
use std::process::Command;

use super::*;

use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "pr_lifecycle::early_prompt_push_stages_only_prompt_files",
            func: early_prompt_push_stages_only_prompt_files,
        },
        ConformanceTest {
            name: "pr_lifecycle::early_prompt_push_fails_on_branch_mismatch",
            func: early_prompt_push_fails_on_branch_mismatch,
        },
        ConformanceTest {
            name: "pr_lifecycle::draft_pr_marked_ready_transition",
            func: draft_pr_marked_ready_transition,
        },
        ConformanceTest {
            name: "pr_lifecycle::no_diff_draft_pr_closed_transition",
            func: no_diff_draft_pr_closed_transition,
        },
        ConformanceTest {
            name: "pr_lifecycle::complete_task_retries_transient_up_to_three",
            func: complete_task_retries_transient_up_to_three,
        },
        ConformanceTest {
            name: "pr_lifecycle::complete_task_no_retry_terminal",
            func: complete_task_no_retry_terminal,
        },
    ]
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(super::panic_message(e)),
    }
}

fn early_prompt_push_stages_only_prompt_files(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let repo = &h.repo_root;
        git(repo, &["checkout", "-b", "ralph/issue-93"]);

        let project_dir = repo.join(".ralph/projects/issue-93");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "Prompt\n").expect("write prompt");
        fs::write(project_dir.join("project.toml"), "name = \"Issue 93\"\n")
            .expect("write metadata");
        fs::write(project_dir.join("config.toml"), "[workflow]\n").expect("write config");
        fs::write(repo.join("unrelated.txt"), "keep unstaged\n").expect("write unrelated");

        crate::git::commit::commit_and_push_initial_prompt(repo, "issue-93", "ralph/issue-93", false)
            .expect("early prompt push should succeed");

        let changed = git_output(repo, &["show", "--name-only", "--pretty=format:", "HEAD"]);
        let mut files: Vec<&str> = changed.lines().filter(|l| !l.trim().is_empty()).collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                ".ralph/projects/issue-93/config.toml",
                ".ralph/projects/issue-93/project.toml",
                ".ralph/projects/issue-93/prompt.md",
            ]
        );
    })
}

fn early_prompt_push_fails_on_branch_mismatch(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let repo = &h.repo_root;
        git(repo, &["checkout", "-b", "ralph/issue-93"]);
        let project_dir = repo.join(".ralph/projects/issue-93");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "Prompt\n").expect("write prompt");

        let before = git_output(repo, &["rev-parse", "HEAD"]);
        let err = crate::git::commit::commit_and_push_initial_prompt(
            repo,
            "issue-93",
            "ralph/issue-94",
            false,
        )
        .expect_err("branch mismatch should fail");

        match err {
            crate::error::RalphError::BranchMismatch { expected, actual } => {
                assert_eq!(expected, "ralph/issue-94");
                assert_eq!(actual, "ralph/issue-93");
            }
            other => panic!("expected BranchMismatch, got {other:?}"),
        }

        let after = git_output(repo, &["rev-parse", "HEAD"]);
        assert_eq!(before, after, "HEAD should remain unchanged");
    })
}

fn draft_pr_marked_ready_transition(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::daemon::runtime::{decide_draft_pr_transition, DraftPrTransition};

        let scenarios = [
            // completed + has changes + draft => mark ready
            (true, true, "ralph:completed", DraftPrTransition::MarkReady),
            // no changes + draft => close draft even on completed
            (false, true, "ralph:completed", DraftPrTransition::CloseNoDiff),
            // completed + has changes + not draft => no-op
            (true, false, "ralph:completed", DraftPrTransition::None),
            // terminal failed does not mark ready
            (true, true, "ralph:failed", DraftPrTransition::None),
            // non-terminal-ish label with no changes + draft still closes
            (false, true, "ralph:in-progress", DraftPrTransition::CloseNoDiff),
        ];

        for (has_changes, is_draft, terminal_label, expected) in scenarios {
            let actual = decide_draft_pr_transition(has_changes, is_draft, terminal_label);
            assert_eq!(
                actual, expected,
                "transition mismatch for has_changes={has_changes}, is_draft={is_draft}, terminal_label={terminal_label}"
            );
        }
    })
}

fn no_diff_draft_pr_closed_transition(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::daemon::runtime::{
            decide_draft_pr_transition, should_close_no_diff_draft_pr, DraftPrTransition,
        };

        let matrix = [
            (false, true, true, DraftPrTransition::CloseNoDiff),
            (true, true, false, DraftPrTransition::MarkReady),
            (false, false, false, DraftPrTransition::None),
            (true, false, false, DraftPrTransition::None),
        ];

        for (has_changes, is_draft, expect_close_predicate, expected_transition) in matrix {
            assert_eq!(
                should_close_no_diff_draft_pr(has_changes, is_draft),
                expect_close_predicate,
                "close predicate mismatch for has_changes={has_changes}, is_draft={is_draft}"
            );

            let transition = decide_draft_pr_transition(has_changes, is_draft, "ralph:completed");
            assert_eq!(
                transition, expected_transition,
                "decision mismatch for has_changes={has_changes}, is_draft={is_draft}"
            );
        }
    })
}

fn complete_task_retries_transient_up_to_three(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::daemon::runtime::{
            complete_task_retry_delay, complete_task_retry_limits, should_retry_complete_task,
        };

        let (max_attempts, retry_delay_secs) = complete_task_retry_limits();
        assert_eq!(max_attempts, 3, "spec requires exactly 3 attempts");
        assert_eq!(retry_delay_secs, 30, "spec requires 30s retry delay");

        let transient_cases = [
            crate::error::RalphError::Orchestration("network timeout from gh api".to_owned()),
            crate::error::RalphError::Orchestration("transport unavailable".to_owned()),
            crate::error::RalphError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "io timeout",
            )),
            crate::error::RalphError::BackendTimeout {
                backend: "claude".to_owned(),
                idle_seconds: 90,
                timeout_kind: crate::error::TimeoutKind::Idle,
            },
            crate::error::RalphError::BackendCommandFailed {
                backend: "codex".to_owned(),
                details: "subprocess broken pipe".to_owned(),
            },
        ];

        for err in transient_cases {
            assert!(err.is_transient(), "expected transient classification for {err:?}");
            assert!(should_retry_complete_task(&err, 1));
            assert!(should_retry_complete_task(&err, 2));
            assert!(!should_retry_complete_task(&err, 3));

            let d1 = complete_task_retry_delay(&err, 1).expect("attempt 1 should delay");
            let d2 = complete_task_retry_delay(&err, 2).expect("attempt 2 should delay");
            assert_eq!(d1.as_secs(), retry_delay_secs);
            assert_eq!(d2.as_secs(), retry_delay_secs);
            assert!(
                complete_task_retry_delay(&err, 3).is_none(),
                "attempt 3 should not schedule another retry"
            );
        }
    })
}

fn complete_task_no_retry_terminal(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::daemon::runtime::{complete_task_retry_delay, should_retry_complete_task};

        let terminal_cases = [
            crate::error::RalphError::Validation("bad config".to_owned()),
            crate::error::RalphError::BranchMismatch {
                expected: "ralph/issue-93".to_owned(),
                actual: "main".to_owned(),
            },
            crate::error::RalphError::GitConflict {
                details: "conflict in src/main.rs".to_owned(),
            },
            crate::error::RalphError::WorkspaceNotFound,
            crate::error::RalphError::ProjectNotFound("issue-93".to_owned()),
            crate::error::RalphError::PrdValidationFailed("schema mismatch".to_owned()),
            crate::error::RalphError::Unsupported("disabled in this mode".to_owned()),
        ];

        for err in terminal_cases {
            assert!(
                !err.is_transient(),
                "expected terminal classification for {err:?}"
            );
            assert!(
                !should_retry_complete_task(&err, 1),
                "terminal errors must not retry at attempt 1: {err:?}"
            );
            assert!(
                complete_task_retry_delay(&err, 1).is_none(),
                "terminal errors must not schedule retry delay: {err:?}"
            );
        }
    })
}

fn git(repo_root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repo_root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
