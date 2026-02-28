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
        assert!(crate::daemon::runtime::should_mark_draft_pr_ready(
            true,
            true,
            "ralph:completed"
        ));
        assert!(!crate::daemon::runtime::should_mark_draft_pr_ready(
            false,
            true,
            "ralph:completed"
        ));
    })
}

fn no_diff_draft_pr_closed_transition(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        assert!(crate::daemon::runtime::should_close_no_diff_draft_pr(false, true));
        assert!(!crate::daemon::runtime::should_close_no_diff_draft_pr(true, true));
        assert!(!crate::daemon::runtime::should_close_no_diff_draft_pr(false, false));
    })
}

fn complete_task_retries_transient_up_to_three(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let transient = crate::error::RalphError::Orchestration(
            "network timeout from gh api".to_owned(),
        );
        assert!(crate::daemon::runtime::should_retry_complete_task(&transient, 1));
        assert!(crate::daemon::runtime::should_retry_complete_task(&transient, 2));
        assert!(!crate::daemon::runtime::should_retry_complete_task(&transient, 3));
    })
}

fn complete_task_no_retry_terminal(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let terminal = crate::error::RalphError::Validation("bad config".to_owned());
        assert!(!crate::daemon::runtime::should_retry_complete_task(&terminal, 1));
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
