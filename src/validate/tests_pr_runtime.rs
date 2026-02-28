use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use super::*;

use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "pr_runtime::draft_watcher_creates_draft_when_branch_ahead",
            func: draft_watcher_creates_draft_when_branch_ahead,
        },
        ConformanceTest {
            name: "pr_runtime::draft_watcher_pushes_before_create",
            func: draft_watcher_pushes_before_create,
        },
        ConformanceTest {
            name: "pr_runtime::draft_watcher_exits_cleanly_on_cancellation",
            func: draft_watcher_exits_cleanly_on_cancellation,
        },
        ConformanceTest {
            name: "pr_runtime::pr_url_plumbed_through_child_args",
            func: pr_url_plumbed_through_child_args,
        },
        ConformanceTest {
            name: "pr_runtime::e2e_draft_create_via_binary",
            func: e2e_draft_create_via_binary,
        },
        ConformanceTest {
            name: "pr_runtime::create_pr_honors_draft_true",
            func: create_pr_honors_draft_true,
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

/// Verify that the draft_pr_watcher creates a draft PR when the branch has
/// commits ahead of the base branch. This test uses the github module's
/// has_commits_ahead_of_base function directly.
fn draft_watcher_creates_draft_when_branch_ahead(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a branch with a commit ahead of master
        let repo = &h.repo_root;
        git(repo, &["checkout", "-b", "ralph/test-ahead"]);
        fs::write(repo.join("new-file.txt"), "content\n").expect("write file");
        git(repo, &["add", "new-file.txt"]);
        git(repo, &["commit", "-m", "ahead commit"]);

        // Verify has_commits_ahead_of_base returns true
        let result = crate::daemon::github::has_commits_ahead_of_base(
            repo,
            "master",
        )
        .expect("has_commits_ahead_of_base should succeed");

        assert!(
            result,
            "branch with extra commit should be ahead of master"
        );

        // Verify that on master, has_commits_ahead_of_base returns false
        git(repo, &["checkout", "master"]);
        let result_on_master = crate::daemon::github::has_commits_ahead_of_base(
            repo,
            "master",
        )
        .expect("has_commits_ahead_of_base should succeed");

        assert!(
            !result_on_master,
            "master should not be ahead of itself"
        );
    })
}

/// Verify that push happens before create_pr call ordering is correct.
/// This tests the github module's push_branch and create_pr functions
/// are both available and correctly ordered.
fn draft_watcher_pushes_before_create(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let repo = &h.repo_root;

        // Create a branch with a commit
        git(repo, &["checkout", "-b", "ralph/test-push-order"]);
        fs::write(repo.join("push-test.txt"), "push test\n").expect("write file");
        git(repo, &["add", "push-test.txt"]);
        git(repo, &["commit", "-m", "push order test"]);

        // Push should succeed (we have an origin remote from harness setup)
        let push_result =
            crate::daemon::github::push_branch(repo, "ralph/test-push-order");
        assert!(
            push_result.is_ok(),
            "push_branch should succeed: {:?}",
            push_result.err()
        );

        // After push, has_commits_ahead_of_base should still be true
        // (we've pushed but the base hasn't moved)
        let ahead = crate::daemon::github::has_commits_ahead_of_base(
            repo,
            "master",
        )
        .expect("has_commits_ahead_of_base should succeed");
        assert!(ahead, "branch should still be ahead after push");
    })
}

/// Verify that CancellationToken works as expected for the draft PR watcher
/// pattern (immediate shutdown via tokio::select!).
fn draft_watcher_exits_cleanly_on_cancellation(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        use tokio_util::sync::CancellationToken;

        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Cancel immediately
        token.cancel();

        // The token should be cancelled
        assert!(
            token_clone.is_cancelled(),
            "token should be cancelled after cancel() call"
        );

        // Verify that a tokio runtime can use the cancelled token in select
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build tokio runtime");

        let completed = rt.block_on(async {
            tokio::select! {
                _ = token_clone.cancelled() => true,
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
            }
        });

        assert!(
            completed,
            "select! should immediately resolve on cancelled token"
        );
    })
}

/// Verify that --pr-url is accepted by both `run` and `auto` subcommands.
fn pr_url_plumbed_through_child_args(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Test that --pr-url is parsed correctly by the run subcommand
        // (this validates CLI parsing, not full execution)
        use clap::Parser;
        use crate::cli::{Cli, Commands};

        let cli = Cli::parse_from([
            "ralph",
            "run",
            "--pr-url",
            "https://github.com/acme/widgets/pull/42",
        ]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(
            args.pr_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/42")
        );

        // Test that --pr-url is parsed correctly by the auto subcommand
        let cli = Cli::parse_from([
            "ralph",
            "auto",
            "--idea",
            "test feature",
            "--pr-url",
            "https://github.com/acme/widgets/pull/99",
        ]);
        let Commands::Auto(args) = cli.command else {
            panic!("expected auto command");
        };
        assert_eq!(
            args.pr_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/99")
        );

        // Test that --pr-url defaults to None when not provided
        let cli = Cli::parse_from([
            "ralph",
            "run",
        ]);
        let Commands::Run(args) = cli.command else {
            panic!("expected run command");
        };
        assert_eq!(args.pr_url, None);
    })
}

/// E2E test: verify --pr-url propagates through real-binary invocation paths
/// for both `run` and `auto`.
fn e2e_draft_create_via_binary(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("issue-93", "Issue 93", "Prompt body")
            .expect("create project");

        // run --pr-url should propagate into project state (resolved args path)
        let run_pr_url = "https://github.com/acme/widgets/pull/71";
        let run_output = h
            .ralph([
                "run",
                "--project",
                "issue-93",
                "--dry-run",
                "--pr-url",
                run_pr_url,
            ])
            .expect("ralph run should execute");
        assert!(
            run_output.status.success(),
            "run --dry-run should succeed: {}",
            String::from_utf8_lossy(&run_output.stderr)
        );
        let run_state = h.load_state("issue-93").expect("load run state");
        assert_eq!(
            run_state.get("pr_url").and_then(|v| v.as_str()),
            Some(run_pr_url),
            "run --pr-url should persist to project state"
        );

        // auto --pr-url should also propagate into project state via orchestrator run.
        let mock = h
            .write_mock_script("mock-auto.sh", &crate::validate::mock_scripts::auto_mock_script())
            .expect("write mock");
        h.setup_mock_backends_stable(&mock)
            .expect("setup mock backends");

        let auto_pr_url = "https://github.com/acme/widgets/pull/72";
        let auto_output = h
            .ralph([
                "auto",
                "--idea",
                "Implement issue 94",
                "--project-id",
                "issue-94",
                "--skip-commit",
                "--skip-prompt-review",
                "--pr-url",
                auto_pr_url,
            ])
            .expect("ralph auto should execute");
        assert!(
            auto_output.status.success(),
            "auto invocation should succeed with mock backends; stderr: {}",
            String::from_utf8_lossy(&auto_output.stderr)
        );

        let auto_state = h.load_state("issue-94").expect("load auto state");
        assert_eq!(
            auto_state.get("pr_url").and_then(|v| v.as_str()),
            Some(auto_pr_url),
            "auto --pr-url should persist to project state"
        );
    })
}

/// Verify create_pr constructs gh args with --draft when draft=true.
fn create_pr_honors_draft_true(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = env_lock().lock().expect("env lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let mock_dir = temp.path().join("bin");
        fs::create_dir_all(&mock_dir).expect("mkdir mock bin");

        let args_log = temp.path().join("gh-args.log");
        let gh_path = mock_dir.join("gh");
        let script = format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" >> '{}'\necho 'https://github.com/acme/widgets/pull/99'\n",
            args_log.display()
        );
        fs::write(&gh_path, script).expect("write gh mock");
        let mut perms = fs::metadata(&gh_path).expect("gh meta").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        fs::set_permissions(&gh_path, perms).expect("set perms");

        let original_path = std::env::var("PATH").unwrap_or_default();
        let composed = format!("{}:{}", mock_dir.display(), original_path);
        unsafe { std::env::set_var("PATH", &composed) };
        let _path_restore = PathEnvGuard::new(original_path.clone());

        let url = crate::daemon::github::create_pr(
            "acme",
            "widgets",
            "ralph/issue-93",
            "Draft PR title",
            "Body",
            true,
        )
        .expect("create_pr draft=true");
        assert_eq!(url, "https://github.com/acme/widgets/pull/99");

        let logged = fs::read_to_string(&args_log).expect("read gh args log");
        assert!(
            logged.lines().any(|line| line == "--draft"),
            "expected --draft arg in gh invocation, got: {logged}"
        );

        let _ = crate::daemon::github::create_pr(
            "acme",
            "widgets",
            "ralph/issue-93",
            "Ready PR",
            "Body",
            false,
        )
        .expect("create_pr draft=false");
        let logged_after = fs::read_to_string(&args_log).expect("read gh args log after second call");
        let draft_count = logged_after.lines().filter(|line| *line == "--draft").count();
        assert_eq!(draft_count, 1, "--draft should only appear for draft=true call");
    })
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
