use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
        ConformanceTest {
            name: "pr_lifecycle::phase_transition_preserves_tracked_ralph_prompt_files",
            func: phase_transition_preserves_tracked_ralph_prompt_files,
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

        crate::git::commit::commit_and_push_initial_prompt(
            repo,
            "issue-93",
            "ralph/issue-93",
            false,
        )
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

fn draft_pr_marked_ready_transition(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = crate::validate::process_env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let repo = &h.repo_root;

        git(repo, &["checkout", "-b", "ralph/issue-ready"]);
        fs::write(repo.join("ready.txt"), "ready\n").expect("write file");
        git(repo, &["add", "ready.txt"]);
        git(repo, &["commit", "-m", "ready transition"]);

        let temp = tempfile::tempdir().expect("tempdir");
        let mock_bin = temp.path().join("bin");
        fs::create_dir_all(&mock_bin).expect("mkdir mock bin");
        let gh_log = temp.path().join("gh-ready.log");

        let gh_script = format!(
            "#!/bin/sh\nset -eu\ncase \"${{1:-}}\" in\n  pr)\n    case \"${{2:-}}\" in\n      list)\n        printf 'https://github.com/acme/widgets/pull/321'\n        exit 0\n        ;;\n      edit)\n        printf 'edit %s\\n' \"$*\" >> '{}'\n        exit 0\n        ;;\n      view)\n        printf '{{\"isDraft\":true}}'\n        exit 0\n        ;;\n      ready)\n        printf 'ready %s\\n' \"$*\" >> '{}'\n        exit 0\n        ;;\n      close)\n        printf 'close %s\\n' \"$*\" >> '{}'\n        exit 0\n        ;;\n    esac\n    ;;\n  issue)\n    case \"${{2:-}}\" in\n      view)\n        printf '{{\"title\":\"Mock issue\",\"body\":\"Mock body\"}}'\n        exit 0\n        ;;\n      comment|edit)\n        exit 0\n        ;;\n    esac\n    ;;\n  api)\n    [ \"${{2:-}}\" = \"user\" ] && printf 'ralph-bot\\n' && exit 0\n    ;;\nesac\necho 'unexpected gh call' >&2\nexit 1\n",
            gh_log.display(),
            gh_log.display(),
            gh_log.display(),
        );
        write_executable(&mock_bin.join("gh"), &gh_script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let _path_restore = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_bin.display(), original_path);
        unsafe { std::env::set_var("PATH", composed) };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        rt.block_on(async {
            crate::daemon::runtime::handle_pr_flow(
                &daemon_config(repo),
                "acme/widgets#321",
                321,
                repo,
            )
            .await
            .expect("handle_pr_flow should succeed for ready path");
        });

        let log = fs::read_to_string(&gh_log).expect("read gh log");
        assert!(
            log.contains("edit pr edit"),
            "expected PR edit call, got: {log}"
        );
        assert!(
            log.contains("ready pr ready"),
            "expected gh pr ready call, got: {log}"
        );
        assert!(
            !log.contains("close pr close"),
            "ready path should not close PR: {log}"
        );
    })
}

fn no_diff_draft_pr_closed_transition(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = crate::validate::process_env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let repo = &h.repo_root;
        git(repo, &["checkout", "master"]);

        let temp = tempfile::tempdir().expect("tempdir");
        let mock_bin = temp.path().join("bin");
        fs::create_dir_all(&mock_bin).expect("mkdir mock bin");
        let gh_log = temp.path().join("gh-close.log");

        let gh_script = format!(
            "#!/bin/sh\nset -eu\ncase \"${{1:-}}\" in\n  pr)\n    case \"${{2:-}}\" in\n      list)\n        printf 'https://github.com/acme/widgets/pull/654'\n        exit 0\n        ;;\n      view)\n        printf '{{\"isDraft\":true}}'\n        exit 0\n        ;;\n      close)\n        printf 'close %s\\n' \"$*\" >> '{log_a}'\n        exit 0\n        ;;\n      edit|ready|create)\n        printf '%s %s\\n' \"$2\" \"$*\" >> '{log_b}'\n        exit 0\n        ;;\n    esac\n    ;;\n  issue)\n    case \"${{2:-}}\" in\n      view)\n        printf '{{\"comments\":[]}}'\n        exit 0\n        ;;\n      comment|edit)\n        exit 0\n        ;;\n    esac\n    ;;\n  api)\n    [ \"${{2:-}}\" = \"user\" ] && printf 'ralph-bot\\n' && exit 0\n    ;;\nesac\nexit 0\n",
            log_a = gh_log.display(),
            log_b = gh_log.display(),
        );
        write_executable(&mock_bin.join("gh"), &gh_script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let _path_restore = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_bin.display(), original_path);
        unsafe { std::env::set_var("PATH", composed) };

        let task_id = "acme/widgets#654";
        let workspace_root = repo.join(".ralph");
        crate::daemon::runtime::save_task_metadata(
            &workspace_root,
            task_id,
            &crate::daemon::runtime::TaskMetadata {
                pr_url: Some("https://github.com/acme/widgets/pull/654".to_owned()),
            },
        );

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        rt.block_on(async {
            crate::daemon::runtime::handle_pr_flow(&daemon_config(repo), task_id, 654, repo)
                .await
                .expect("handle_pr_flow should succeed for no-diff close path");
        });

        let log = fs::read_to_string(&gh_log).expect("read gh close log");
        assert!(
            log.contains("close pr close"),
            "expected gh pr close call, got: {log}"
        );

        let meta = crate::daemon::runtime::load_task_metadata(&workspace_root, task_id);
        assert_eq!(
            meta.pr_url, None,
            "no-diff close path should clear persisted PR URL metadata"
        );
    })
}

fn complete_task_retries_transient_up_to_three(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sleeps = Arc::new(Mutex::new(Vec::<u64>::new()));

        let attempts_c = Arc::clone(&attempts);
        let sleeps_c = Arc::clone(&sleeps);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        let result = rt.block_on(async {
            crate::daemon::runtime::complete_task_with_retry_for_test(
                || {
                    let n = attempts_c.fetch_add(1, Ordering::SeqCst) + 1;
                    async move {
                        if n < 3 {
                            Err(crate::error::RalphError::Orchestration(
                                "network timeout while updating labels".to_owned(),
                            ))
                        } else {
                            Ok(())
                        }
                    }
                },
                |delay| {
                    let sleeps_inner = Arc::clone(&sleeps_c);
                    async move {
                        sleeps_inner
                            .lock()
                            .expect("sleep lock")
                            .push(delay.as_secs());
                    }
                },
            )
            .await
        });

        assert!(
            result.is_ok(),
            "transient failures should succeed by attempt 3: {result:?}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            3,
            "transient retry loop should execute exactly 3 attempts"
        );
        let sleep_values = sleeps.lock().expect("sleep lock").clone();
        assert_eq!(sleep_values, vec![30, 30], "expected two 30s retry delays");
    })
}

fn complete_task_no_retry_terminal(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sleeps = Arc::new(AtomicUsize::new(0));

        let attempts_c = Arc::clone(&attempts);
        let sleeps_c = Arc::clone(&sleeps);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        let result = rt.block_on(async {
            crate::daemon::runtime::complete_task_with_retry_for_test(
                || {
                    attempts_c.fetch_add(1, Ordering::SeqCst);
                    async {
                        Err(crate::error::RalphError::Validation(
                            "terminal validation failure".to_owned(),
                        ))
                    }
                },
                |_delay| {
                    let sleeps_inner = Arc::clone(&sleeps_c);
                    async move {
                        sleeps_inner.fetch_add(1, Ordering::SeqCst);
                    }
                },
            )
            .await
        });

        assert!(result.is_err(), "terminal error should return immediately");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "terminal failure must only run one attempt"
        );
        assert_eq!(
            sleeps.load(Ordering::SeqCst),
            0,
            "terminal failure must not schedule retries"
        );
    })
}

/// Verify that `commit_and_push_phase_transition` does not stage deletions for
/// tracked `.ralph/projects/<id>/` prompt/config files.  This ensures the
/// non-destructive `git reset HEAD -- .ralph` unstaging (replacing the old
/// `git rm --cached -r .ralph`) preserves tracked prompt inputs.
fn phase_transition_preserves_tracked_ralph_prompt_files(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let repo = &h.repo_root;

        // Create and track prompt inputs on a project branch.
        git(repo, &["checkout", "-b", "ralph/issue-preserve"]);
        let project_dir = repo.join(".ralph/projects/issue-preserve");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "Prompt content\n").expect("write prompt");
        fs::write(project_dir.join("project.toml"), "name = \"preserve\"\n")
            .expect("write project toml");
        fs::write(project_dir.join("config.toml"), "[workflow]\n").expect("write config");
        git(repo, &["add", "--", ".ralph/projects/issue-preserve/"]);
        git(repo, &["commit", "-m", "chore: track prompt inputs"]);
        git(repo, &["push", "-u", "origin", "ralph/issue-preserve"]);

        // Add implementation work (non-.ralph file).
        fs::write(repo.join("impl.txt"), "implementation\n").expect("write impl file");

        // Run phase transition commit.
        crate::git::commit::commit_and_push_phase_transition(
            repo,
            "issue-preserve",
            1,
            crate::project::state::Phase::Planning,
            crate::project::state::Phase::Implementing,
            "ralph/issue-preserve",
            false,
        )
        .expect("phase transition commit should succeed");

        // Verify that the tracked prompt files still exist in the latest commit.
        let tree_files = git_output(repo, &["ls-tree", "-r", "--name-only", "HEAD"]);
        for expected in [
            ".ralph/projects/issue-preserve/prompt.md",
            ".ralph/projects/issue-preserve/project.toml",
            ".ralph/projects/issue-preserve/config.toml",
        ] {
            assert!(
                tree_files.lines().any(|l| l == expected),
                "tracked prompt file {expected} must still be present in HEAD after phase transition, tree:\n{tree_files}"
            );
        }

        // Also verify no deletion was staged (check the diff of the last commit).
        let diff = git_output(repo, &["diff", "--name-status", "HEAD~1..HEAD"]);
        let deletions: Vec<&str> = diff
            .lines()
            .filter(|l| l.starts_with('D') && l.contains(".ralph/projects/"))
            .collect();
        assert!(
            deletions.is_empty(),
            "phase transition must not delete tracked .ralph/projects/ files:\n{}",
            deletions.join("\n")
        );
    })
}

fn daemon_config(repo_root: &Path) -> crate::daemon::runtime::DaemonRuntimeConfig {
    crate::daemon::runtime::DaemonRuntimeConfig {
        owner: "acme".to_owned(),
        repo: "widgets".to_owned(),
        base_branch: "master".to_owned(),
        poll_seconds: 1,
        max_concurrent: 1,
        labels: vec!["ralph:ready".to_owned()],
        single_iteration: true,
        verbose: false,
        repo_root: repo_root.to_path_buf(),
        refinement_enabled: false,
        refinement_backend: "claude".to_owned(),
        global_config: crate::config::GlobalConfig::default(),
        auto_rebase_enabled: false,
        rebase_interval_seconds: 60,
        max_rebases_per_cycle: 0,
        rebase_timeout_seconds: 60,
        rebase_agent_backend: "none".to_owned(),
        workspace_root: repo_root.join(".ralph"),
        prd_enabled: false,
        prd_question_backends: vec![],
        prd_writer_backend: "claude".to_owned(),
        prd_reviewer_backend: "claude".to_owned(),
        prd_max_revisions: 1,
        prd_backend_timeout_secs: 30,
        prd_shutdown_timeout_secs: 60,
        git_bin: "git".to_owned(),
        gh_bin: "gh".to_owned(),
        max_backend_retries: None,
    }
}

struct PathEnvGuard {
    original_path: String,
}

impl PathEnvGuard {
    fn new(original_path: String) -> Self {
        Self { original_path }
    }
}

impl Drop for PathEnvGuard {
    fn drop(&mut self) {
        unsafe { std::env::set_var("PATH", &self.original_path) };
    }
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("meta").permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(path, perms).expect("chmod");
}

fn git(repo_root: &Path, args: &[&str]) {
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

fn git_output(repo_root: &Path, args: &[&str]) -> String {
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
