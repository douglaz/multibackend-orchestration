use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

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
        ConformanceTest {
            name: "pr_runtime::draft_watcher_fallback_base_when_configured_missing",
            func: draft_watcher_fallback_base_when_configured_missing,
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

fn draft_watcher_creates_draft_when_branch_ahead(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = env_lock().lock().expect("env lock");
        let repo = &h.repo_root;

        git(repo, &["checkout", "-b", "ralph/test-draft-create"]);
        fs::write(repo.join("draft-create.txt"), "content\n").expect("write file");
        git(repo, &["add", "draft-create.txt"]);
        git(repo, &["commit", "-m", "ahead commit"]);

        let temp = tempfile::tempdir().expect("tempdir");
        let mock_bin = temp.path().join("bin");
        fs::create_dir_all(&mock_bin).expect("mkdir mock bin");
        let gh_log = temp.path().join("gh.log");

        let gh_script = format!(
            "#!/bin/sh\nset -eu\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then\n  printf 'create %s\\n' \"$*\" >> '{}'\n  echo 'https://github.com/acme/widgets/pull/123'\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n  printf ''\n  exit 0\nfi\nprintf 'unexpected gh call: %s\\n' \"$*\" >&2\nexit 1\n",
            gh_log.display()
        );
        write_executable(&mock_bin.join("gh"), &gh_script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let path_guard = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_bin.display(), original_path);
        unsafe { std::env::set_var("PATH", composed) };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        rt.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(5),
                crate::daemon::runtime::draft_pr_watcher(
                    "acme".to_owned(),
                    "widgets".to_owned(),
                    "master".to_owned(),
                    repo.to_path_buf(),
                    "ralph/test-draft-create".to_owned(),
                    "acme/widgets#123".to_owned(),
                    123,
                    tokio_util::sync::CancellationToken::new(),
                    repo.join(".ralph"),
                ),
            )
            .await
            .expect("watcher should complete after creating draft PR");
        });

        let log = fs::read_to_string(&gh_log).expect("read gh log");
        assert!(
            log.contains("pr create"),
            "expected gh pr create invocation, got: {log}"
        );
        assert!(log.contains("--draft"), "expected --draft flag, got: {log}");

        drop(path_guard);
    })
}

fn draft_watcher_pushes_before_create(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = env_lock().lock().expect("env lock");
        let repo = &h.repo_root;

        git(repo, &["checkout", "-b", "ralph/test-push-before-create"]);
        fs::write(repo.join("push-before-create.txt"), "content\n").expect("write file");
        git(repo, &["add", "push-before-create.txt"]);
        git(repo, &["commit", "-m", "ahead commit"]);

        let temp = tempfile::tempdir().expect("tempdir");
        let mock_bin = temp.path().join("bin");
        fs::create_dir_all(&mock_bin).expect("mkdir mock bin");
        let ordering_log = temp.path().join("ordering.log");

        let real_git = resolve_bin("git");
        let git_script = format!(
            "#!/bin/sh\nset -eu\nif [ \"${{1:-}}\" = \"push\" ]; then\n  printf 'push\\n' >> '{}'\nfi\nexec '{}' \"$@\"\n",
            ordering_log.display(),
            real_git
        );
        write_executable(&mock_bin.join("git"), &git_script);

        let gh_script = format!(
            "#!/bin/sh\nset -eu\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then\n  printf 'create\\n' >> '{}'\n  echo 'https://github.com/acme/widgets/pull/124'\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n  printf ''\n  exit 0\nfi\nprintf 'unexpected gh call: %s\\n' \"$*\" >&2\nexit 1\n",
            ordering_log.display()
        );
        write_executable(&mock_bin.join("gh"), &gh_script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let path_guard = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_bin.display(), original_path);
        unsafe { std::env::set_var("PATH", composed) };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        rt.block_on(async {
            crate::daemon::runtime::draft_pr_watcher_single_iteration_for_test(
                "acme".to_owned(),
                "widgets".to_owned(),
                "master".to_owned(),
                repo.to_path_buf(),
                "ralph/test-push-before-create".to_owned(),
                "acme/widgets#124".to_owned(),
                124,
                repo.join(".ralph"),
            )
            .await;
        });

        let ordering = fs::read_to_string(&ordering_log).expect("read ordering log");
        let lines: Vec<&str> = ordering.lines().collect();
        let push_pos = lines.iter().position(|line| *line == "push");
        let create_pos = lines.iter().position(|line| *line == "create");
        assert!(
            push_pos.is_some(),
            "expected git push log entry, got: {ordering}"
        );
        assert!(
            create_pos.is_some(),
            "expected gh pr create log entry, got: {ordering}"
        );
        assert!(
            push_pos.expect("push pos") < create_pos.expect("create pos"),
            "expected push before create, got: {ordering}"
        );

        drop(path_guard);
    })
}

fn draft_watcher_exits_cleanly_on_cancellation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = env_lock().lock().expect("env lock");
        let repo = &h.repo_root;
        git(repo, &["checkout", "master"]);

        let temp = tempfile::tempdir().expect("tempdir");
        let mock_bin = temp.path().join("bin");
        fs::create_dir_all(&mock_bin).expect("mkdir mock bin");

        let gh_script = "#!/bin/sh\nset -eu\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then printf ''; exit 0; fi\nexit 0\n";
        write_executable(&mock_bin.join("gh"), gh_script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let path_guard = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_bin.display(), original_path);
        unsafe { std::env::set_var("PATH", composed) };

        let original_poll = std::env::var("RALPH_DRAFT_PR_WATCH_POLL_SECS").ok();
        unsafe { std::env::set_var("RALPH_DRAFT_PR_WATCH_POLL_SECS", "60") };
        let poll_guard = EnvVarRestoreGuard::new("RALPH_DRAFT_PR_WATCH_POLL_SECS", original_poll);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        rt.block_on(async {
            let cancel = tokio_util::sync::CancellationToken::new();
            let join = tokio::spawn(crate::daemon::runtime::draft_pr_watcher(
                "acme".to_owned(),
                "widgets".to_owned(),
                "master".to_owned(),
                repo.to_path_buf(),
                "master".to_owned(),
                "acme/widgets#125".to_owned(),
                125,
                cancel.clone(),
                repo.join(".ralph"),
            ));

            cancel.cancel();

            let result = tokio::time::timeout(Duration::from_secs(2), join)
                .await
                .expect("watcher join should resolve within timeout");
            assert!(
                result.is_ok(),
                "watcher should exit without panic: {result:?}"
            );
        });

        drop(poll_guard);
        drop(path_guard);
    })
}

/// Verify runtime PR URL resolution and forwarding order:
/// 1) `gh pr list --head <daemon-branch>` exact-match lookup is used,
/// 2) resolved PR URL is forwarded into spawned child args,
/// 3) PR URL resolution happens before child spawn.
fn pr_url_plumbed_through_child_args(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init workspace");

        let issue_number = 77_u32;
        let task_id = crate::daemon::format_task_id("acme", "widgets", issue_number);
        let expected_branch = format!("ralph/daemon/{task_id}");
        let expected_pr_url = "https://github.com/acme/widgets/pull/777";

        let ordering_log = dh.temp_dir.path().join("pr-url-ordering.log");
        let gh_log = dh.temp_dir.path().join("pr-url-gh.log");

        let gh_script = format!(
            "#!/bin/sh\nset -eu\ncase \"${{1:-}}\" in\n  issue)\n    case \"${{2:-}}\" in\n      list)\n        printf '%s' \"${{MOCK_GH_ISSUES:-[]}}\"\n        exit 0\n        ;;\n      edit|comment)\n        exit 0\n        ;;\n      view)\n        if [ \"${{6:-}}\" = \"title,body\" ] || [ \"${{7:-}}\" = \"title,body\" ]; then\n          printf '{{\"title\":\"Runtime PR URL test\",\"body\":\"body\"}}'\n          exit 0\n        fi\n        printf ''\n        exit 0\n        ;;\n    esac\n    ;;\n  label)\n    [ \"${{2:-}}\" = \"create\" ] && exit 0\n    ;;\n  pr)\n    case \"${{2:-}}\" in\n      list)\n        printf 'lookup %s\\n' \"$*\" >> '{ordering_log_a}'\n        head=''\n        prev=''\n        for arg in \"$@\"; do\n          if [ \"$prev\" = \"--head\" ]; then\n            head=\"$arg\"\n          fi\n          prev=\"$arg\"\n        done\n        if [ \"$head\" = '{expected_branch}' ]; then\n          printf 'resolved {expected_pr_url}\\n' >> '{ordering_log_b}'\n          printf '{expected_pr_url}\\n'\n          exit 0\n        fi\n        printf 'resolved none for-head=%s\\n' \"$head\" >> '{ordering_log_c}'\n        printf ''\n        exit 0\n        ;;\n      create)\n        printf 'unexpected-create %s\\n' \"$*\" >> '{gh_log_a}'\n        exit 1\n        ;;\n      edit|ready|close)\n        printf ''\n        exit 0\n        ;;\n      view)\n        printf '{{\"isDraft\":false}}'\n        exit 0\n        ;;\n    esac\n    ;;\n  api)\n    [ \"${{2:-}}\" = \"user\" ] && printf 'ralph-bot\\n' && exit 0\n    ;;\n  repo)\n    [ \"${{2:-}}\" = \"view\" ] && printf 'acme/widgets\\n' && exit 0\n    ;;\nesac\necho \"unexpected gh invocation: $*\" >&2\nexit 1\n",
            ordering_log_a = ordering_log.display(),
            ordering_log_b = ordering_log.display(),
            ordering_log_c = ordering_log.display(),
            gh_log_a = gh_log.display(),
            expected_branch = expected_branch,
            expected_pr_url = expected_pr_url,
        );
        let gh_path = write_mock_gh_path(&dh, &gh_script).expect("write mock gh");

        let ralph_script = format!(
            "#!/bin/sh\nset -eu\ncase \"${{1:-}}\" in\n  auto|run)\n    pr_url='none'\n    prev=''\n    for arg in \"$@\"; do\n      if [ \"$prev\" = \"--pr-url\" ]; then\n        pr_url=\"$arg\"\n      fi\n      prev=\"$arg\"\n    done\n    printf 'spawn-pr-url %s\\n' \"$pr_url\" >> '{ordering_log}'\n    exit 0\n    ;;\n  *)\n    echo \"mock ralph: unhandled command: $1\" >&2\n    exit 1\n    ;;\nesac\n",
            ordering_log = ordering_log.display(),
        );
        let ralph_path = dh
            .write_mock_script("mock-daemon-ralph-pr-url", &ralph_script)
            .expect("write mock daemon ralph");
        let ralph_path_str = ralph_path.to_string_lossy().into_owned();

        let issues = format!(
            "[{{\"number\":{issue_number},\"title\":\"PR URL dispatch\",\"labels\":[{{\"name\":\"ralph:ready\"}}],\"body\":\"runtime check\"}}]"
        );

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
                    ("RALPH_DAEMON_BIN", &ralph_path_str),
                    ("MOCK_GH_ISSUES", &issues),
                ],
            )
            .expect("daemon start should execute");
        crate::validate::assertions::assert_exit_code(&output, 0);

        let ordering = fs::read_to_string(&ordering_log).expect("read ordering log");
        assert!(
            ordering.contains(&format!("--head {expected_branch}")),
            "expected exact --head branch lookup in gh call, got: {ordering}"
        );
        assert!(
            ordering.contains(&format!("resolved {expected_pr_url}")),
            "expected resolved PR URL marker in ordering log, got: {ordering}"
        );
        assert!(
            ordering.contains(&format!("spawn-pr-url {expected_pr_url}")),
            "expected spawned child to receive --pr-url value, got: {ordering}"
        );

        let resolved_line = format!("resolved {expected_pr_url}");
        let spawned_line = format!("spawn-pr-url {expected_pr_url}");
        let lines: Vec<&str> = ordering.lines().collect();
        let resolved_pos = lines.iter().position(|line| *line == resolved_line);
        let spawned_pos = lines.iter().position(|line| *line == spawned_line);
        assert!(
            resolved_pos.is_some(),
            "missing resolved marker: {ordering}"
        );
        assert!(spawned_pos.is_some(), "missing spawn marker: {ordering}");
        assert!(
            resolved_pos.expect("resolved pos") < spawned_pos.expect("spawn pos"),
            "expected resolution before child spawn marker, got: {ordering}"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        let resolved_log = format!("dispatch: resolved PR URL for {task_id}: {expected_pr_url}");
        let spawn_log = format!(
            "dispatch: task {task_id} starting fresh with ralph auto --project-id issue-{issue_number} pr_url={expected_pr_url}"
        );
        let resolved_idx = stderr.find(&resolved_log);
        let spawn_idx = stderr.find(&spawn_log);
        assert!(
            resolved_idx.is_some(),
            "expected runtime resolution log line in stderr, got:\n{stderr}"
        );
        assert!(
            spawn_idx.is_some(),
            "expected runtime child-spawn log line in stderr, got:\n{stderr}"
        );
        assert!(
            resolved_idx.expect("resolved idx") < spawn_idx.expect("spawn idx"),
            "expected runtime resolution log to precede child-spawn log, got:\n{stderr}"
        );

        let unexpected_create = fs::read_to_string(&gh_log).unwrap_or_default();
        assert!(
            unexpected_create.trim().is_empty(),
            "did not expect draft PR creation when existing PR URL is resolved, got: {unexpected_create}"
        );
    })
}

fn e2e_draft_create_via_binary(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init workspace");

        let _guard = env_lock().lock().expect("env lock");
        let gh_log = dh.temp_dir.path().join("gh-e2e.log");
        let pr_state = dh.temp_dir.path().join("pr-created.state");

        let gh_script = format!(
            "#!/bin/sh\nset -eu\ncase \"${{1:-}}\" in\n  issue)\n    case \"${{2:-}}\" in\n      list)\n        printf '%s' \"${{MOCK_GH_ISSUES:-[]}}\"\n        exit 0\n        ;;\n      edit|comment)\n        exit 0\n        ;;\n      view)\n        if [ \"${{6:-}}\" = \"title,body\" ] || [ \"${{7:-}}\" = \"title,body\" ]; then\n          printf '{{\"title\":\"E2E issue\",\"body\":\"body\"}}'\n          exit 0\n        fi\n        printf ''\n        exit 0\n        ;;\n    esac\n    ;;\n  label)\n    [ \"${{2:-}}\" = \"create\" ] && exit 0\n    ;;\n  pr)\n    case \"${{2:-}}\" in\n      list)\n        if [ -f '{pr_state_a}' ]; then cat '{pr_state_b}'; fi\n        exit 0\n        ;;\n      create)\n        printf 'create %s\\n' \"$*\" >> '{gh_log_a}'\n        url='https://github.com/acme/widgets/pull/901'\n        printf '%s' \"$url\" > '{pr_state_c}'\n        printf '%s\\n' \"$url\"\n        exit 0\n        ;;\n      edit)\n        printf 'edit %s\\n' \"$*\" >> '{gh_log_b}'\n        exit 0\n        ;;\n      view)\n        printf '{{\"isDraft\":true}}'\n        exit 0\n        ;;\n      ready)\n        printf 'ready %s\\n' \"$*\" >> '{gh_log_c}'\n        exit 0\n        ;;\n      close)\n        printf 'close %s\\n' \"$*\" >> '{gh_log_d}'\n        exit 0\n        ;;\n    esac\n    ;;\n  api)\n    [ \"${{2:-}}\" = \"user\" ] && printf 'ralph-bot\\n' && exit 0\n    ;;\n  repo)\n    [ \"${{2:-}}\" = \"view\" ] && printf 'acme/widgets\\n' && exit 0\n    ;;\nesac\necho \"unexpected gh invocation: $*\" >&2\nexit 1\n",
            pr_state_a = pr_state.display(),
            pr_state_b = pr_state.display(),
            gh_log_a = gh_log.display(),
            pr_state_c = pr_state.display(),
            gh_log_b = gh_log.display(),
            gh_log_c = gh_log.display(),
            gh_log_d = gh_log.display(),
        );
        let gh_path = write_mock_gh_path(&dh, &gh_script).expect("write mock gh");

        let ralph_script = r#"#!/bin/sh
set -eu
case "${1:-}" in
  auto|run)
    git config user.email "mock@test"
    git config user.name "Mock"
    echo "work" >> e2e-draft.txt
    git add e2e-draft.txt
    git commit -m "mock impl" >/dev/null 2>&1 || true
    # Sleep long enough for the draft-PR watcher (poll_secs=1) to detect the
    # commit, push, and create the draft PR before this child exits.
    sleep 10
    exit 0
    ;;
  *)
    echo "mock ralph: unhandled command: $1" >&2
    exit 1
    ;;
esac
"#;
        let ralph_path = dh
            .write_mock_script("mock-daemon-ralph", ralph_script)
            .expect("write mock daemon ralph");
        let ralph_path_str = ralph_path.to_string_lossy().into_owned();

        let issues = r#"[{"number":901,"title":"Draft lifecycle","labels":[{"name":"ralph:ready"}],"body":"run flow"}]"#;
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
                    ("RALPH_DAEMON_BIN", &ralph_path_str),
                    ("MOCK_GH_ISSUES", issues),
                    // Short poll interval so the draft-PR watcher detects the
                    // child commit and creates the draft PR before the child
                    // exits, making the lifecycle ordering deterministic.
                    ("RALPH_DRAFT_PR_WATCH_POLL_SECS", "1"),
                ],
            )
            .expect("daemon start should execute");
        crate::validate::assertions::assert_exit_code(&output, 0);

        let log = fs::read_to_string(&gh_log).expect("read gh e2e log");
        let create_pos = log.find("create");
        let ready_pos = log.find("ready");
        assert!(
            create_pos.is_some(),
            "expected draft PR creation in log, got: {log}"
        );
        assert!(
            ready_pos.is_some(),
            "expected draft PR ready transition in log, got: {log}"
        );
        assert!(
            create_pos.expect("create") < ready_pos.expect("ready"),
            "expected create before ready, got: {log}"
        );
    })
}

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
        write_executable(&gh_path, &script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let _path_restore = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_dir.display(), original_path);
        unsafe { std::env::set_var("PATH", &composed) };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let url = rt
            .block_on(crate::daemon::github::create_pr(
                "acme",
                "widgets",
                "ralph/issue-93",
                "Draft PR title",
                "Body",
                true,
            ))
            .expect("create_pr draft=true");
        assert_eq!(url, "https://github.com/acme/widgets/pull/99");

        let logged = fs::read_to_string(&args_log).expect("read gh args log");
        assert!(
            logged.lines().any(|line| line == "--draft"),
            "expected --draft arg in gh invocation, got: {logged}"
        );

        let _ = rt
            .block_on(crate::daemon::github::create_pr(
                "acme",
                "widgets",
                "ralph/issue-93",
                "Ready PR",
                "Body",
                false,
            ))
            .expect("create_pr draft=false");
        let logged_after =
            fs::read_to_string(&args_log).expect("read gh args log after second call");
        let draft_count = logged_after
            .lines()
            .filter(|line| *line == "--draft")
            .count();
        assert_eq!(
            draft_count, 1,
            "--draft should only appear for draft=true call"
        );
    })
}

/// Verify that draft PR creation succeeds even when the configured base branch
/// (e.g. "master") does not exist as a remote ref, as long as a default branch
/// (e.g. "main") is resolvable.
fn draft_watcher_fallback_base_when_configured_missing(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = env_lock().lock().expect("env lock");
        let repo = &h.repo_root;

        // Rename the remote default branch to "main" and delete "master" so
        // that "master" is no longer resolvable.
        git(repo, &["branch", "-m", "master", "main"]);
        git(repo, &["push", "origin", "main"]);
        // Delete the old remote master ref (best-effort, may not exist).
        let _ = Command::new("git")
            .args(["push", "origin", "--delete", "master"])
            .current_dir(repo)
            .output();
        git(repo, &["remote", "set-head", "origin", "main"]);

        git(repo, &["checkout", "-b", "ralph/test-fallback-base"]);
        fs::write(repo.join("fallback-base.txt"), "content\n").expect("write file");
        git(repo, &["add", "fallback-base.txt"]);
        git(repo, &["commit", "-m", "ahead commit for fallback test"]);

        let temp = tempfile::tempdir().expect("tempdir");
        let mock_bin = temp.path().join("bin");
        fs::create_dir_all(&mock_bin).expect("mkdir mock bin");
        let gh_log = temp.path().join("gh-fallback.log");

        let gh_script = format!(
            "#!/bin/sh\nset -eu\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then\n  printf 'create %s\\n' \"$*\" >> '{}'\n  echo 'https://github.com/acme/widgets/pull/555'\n  exit 0\nfi\nif [ \"$1\" = \"pr\" ] && [ \"$2\" = \"list\" ]; then\n  printf ''\n  exit 0\nfi\nprintf 'unexpected gh call: %s\\n' \"$*\" >&2\nexit 1\n",
            gh_log.display()
        );
        write_executable(&mock_bin.join("gh"), &gh_script);

        let original_path = std::env::var("PATH").unwrap_or_default();
        let path_guard = PathEnvGuard::new(original_path.clone());
        let composed = format!("{}:{}", mock_bin.display(), original_path);
        unsafe { std::env::set_var("PATH", composed) };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        // Pass "master" as the configured base — it no longer exists; the
        // watcher should resolve the fallback ("main") and still create the PR.
        rt.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(5),
                crate::daemon::runtime::draft_pr_watcher(
                    "acme".to_owned(),
                    "widgets".to_owned(),
                    "master".to_owned(),
                    repo.to_path_buf(),
                    "ralph/test-fallback-base".to_owned(),
                    "acme/widgets#555".to_owned(),
                    555,
                    tokio_util::sync::CancellationToken::new(),
                    repo.join(".ralph"),
                ),
            )
            .await
            .expect("watcher should complete after creating draft PR with fallback base");
        });

        let log = fs::read_to_string(&gh_log).expect("read gh fallback log");
        assert!(
            log.contains("pr create"),
            "expected gh pr create invocation, got: {log}"
        );
        assert!(log.contains("--draft"), "expected --draft flag, got: {log}");

        drop(path_guard);
    })
}

fn resolve_bin(bin: &str) -> String {
    let out = Command::new("bash")
        .args(["-lc", &format!("command -v {bin}")])
        .output()
        .expect("resolve bin path");
    assert!(
        out.status.success(),
        "failed to resolve {bin}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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

fn write_mock_gh_path(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("gh", body)?;
    let base = script
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    Ok(format!("{base}:{existing}"))
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

struct EnvVarRestoreGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvVarRestoreGuard {
    fn new(key: &'static str, original: Option<String>) -> Self {
        Self { key, original }
    }
}

impl Drop for EnvVarRestoreGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.original {
            unsafe { std::env::set_var(self.key, value) };
        } else {
            unsafe { std::env::remove_var(self.key) };
        }
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
