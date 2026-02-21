use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::*;

use crate::daemon::rebase_agent::{
    build_agent_prompt, classify_rebase_failure, is_rebase_in_progress,
    parse_rebase_agent_backend, resolve_rebase_conflicts, RebaseAgentBackend,
    RebaseFailureKind,
};
use crate::validate::harness::RalphHarness;

/// Mutex to serialize tests that mutate the process PATH.
static PATH_MUTEX: Mutex<()> = Mutex::new(());

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "daemon_rebase::config_default_value",
            func: config_default_value,
        },
        ConformanceTest {
            name: "daemon_rebase::config_backward_compat_missing_key",
            func: config_backward_compat_missing_key,
        },
        ConformanceTest {
            name: "daemon_rebase::agent_disabled_parses_none",
            func: agent_disabled_parses_none,
        },
        ConformanceTest {
            name: "daemon_rebase::agent_enabled_recovery_prompt_contract",
            func: agent_enabled_recovery_prompt_contract,
        },
        ConformanceTest {
            name: "daemon_rebase::agent_enabled_recovery_resolves_conflict",
            func: agent_enabled_recovery_resolves_conflict,
        },
        ConformanceTest {
            name: "daemon_rebase::agent_disabled_fallback_aborts_conflict",
            func: agent_disabled_fallback_aborts_conflict,
        },
        ConformanceTest {
            name: "daemon_rebase::agent_failure_aborts_rebase",
            func: agent_failure_aborts_rebase,
        },
    ]
}

fn config_default_value(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");

        let value = h
            .ralph_ok(["config", "get", "workspace.daemon_rebase_agent_backend"])
            .expect("config get daemon_rebase_agent_backend");

        assert_eq!(value.trim(), "claude(opus)");
    })
}

fn config_backward_compat_missing_key(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");

        let global_config = h.repo_root.join(".ralph").join("ralph.toml");
        let raw = fs::read_to_string(&global_config).expect("read global config");
        let filtered = raw
            .lines()
            .filter(|line| !line.contains("daemon_rebase_agent_backend"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&global_config, format!("{filtered}\n")).expect("write config without new key");

        let value = h
            .ralph_ok(["config", "get", "workspace.daemon_rebase_agent_backend"])
            .expect("config get should still resolve default");

        assert_eq!(value.trim(), "claude(opus)");
    })
}

fn agent_disabled_parses_none(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("daemon-rebase", "Daemon Rebase", "Prompt")
            .expect("create project");

        h.ralph_ok(["config", "set", "daemon.rebase_agent_backend", "none"])
            .expect("set project daemon.rebase_agent_backend to none");

        let resolved = h
            .ralph_ok(["config", "get", "daemon.rebase_agent_backend"])
            .expect("get resolved daemon.rebase_agent_backend");
        let parsed = parse_rebase_agent_backend(resolved.trim())
            .expect("parse resolved daemon.rebase_agent_backend");

        assert_eq!(parsed, RebaseAgentBackend::None);
    })
}

/// Verify that the prompt contract includes required elements when agent is enabled.
fn agent_enabled_recovery_prompt_contract(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let files = vec!["src/main.rs".to_owned(), "Cargo.toml".to_owned()];
        let prompt = build_agent_prompt("origin/master", &files);

        assert!(
            prompt.contains("origin/master"),
            "prompt must include rebase target"
        );
        assert!(
            prompt.contains("- src/main.rs"),
            "prompt must list conflicting files"
        );
        assert!(
            prompt.contains("- Cargo.toml"),
            "prompt must list conflicting files"
        );
        assert!(
            prompt.contains("git add"),
            "prompt must require staging with git add"
        );
        assert!(
            prompt.contains("Do NOT run `git rebase --continue`"),
            "prompt must forbid rebase --continue"
        );
        assert!(
            prompt.contains("`git rebase --abort`"),
            "prompt must forbid rebase --abort"
        );
        assert!(
            prompt.contains("Do NOT modify any files that are not listed"),
            "prompt must forbid unrelated file edits"
        );
    })
}

/// Agent-enabled recovery: create a real conflict, use mock claude to resolve it,
/// verify the rebase completes successfully and no longer in progress.
fn agent_enabled_recovery_resolves_conflict(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let repo = tmp.path();

        // Set up repo with conflict
        run_git_in(repo, &["init"]);
        run_git_in(repo, &["config", "user.email", "test@example.com"]);
        run_git_in(repo, &["config", "user.name", "Test User"]);

        fs::write(repo.join("conflict.txt"), "base\n").expect("write base");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "base"]);

        fs::write(repo.join("conflict.txt"), "master\n").expect("write master");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "master diverges"]);

        run_git_in(repo, &["checkout", "-b", "feature", "HEAD~1"]);
        fs::write(repo.join("conflict.txt"), "feature\n").expect("write feature");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "feature diverges"]);

        // Start rebase (will conflict)
        let output = std::process::Command::new("git")
            .args(["rebase", "master"])
            .current_dir(repo)
            .output()
            .expect("run git rebase");
        assert!(!output.status.success(), "expected rebase conflict");
        assert!(is_rebase_in_progress(repo), "rebase should be in progress");

        // Create mock claude that actually resolves the conflict
        let bin_dir = tmp.path().join("mock-bin");
        fs::create_dir_all(&bin_dir).expect("create mock-bin");
        let claude_path = bin_dir.join("claude");
        let script = format!(
            "#!/bin/sh\necho 'resolved' > {}/conflict.txt\ngit -C {} add conflict.txt\n",
            repo.display(),
            repo.display(),
        );
        fs::write(&claude_path, &script).expect("write mock claude");
        let mut perms = fs::metadata(&claude_path).expect("meta").permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&claude_path, perms).expect("set perms");

        let _guard = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), old_path));

        let backend = RebaseAgentBackend::Claude {
            model: "opus".to_owned(),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let result = resolve_rebase_conflicts(repo, "master", &backend, deadline);

        std::env::set_var("PATH", &old_path);

        assert!(
            result.is_ok(),
            "agent-enabled recovery should succeed: {:?}",
            result.err()
        );
        assert!(
            !is_rebase_in_progress(repo),
            "rebase should be complete after agent-enabled recovery"
        );
    })
}

/// Agent-disabled (backend=none) fallback: create a real conflict, verify that
/// classify_rebase_failure correctly identifies the conflict, and confirm that
/// the None backend means the caller should abort without invoking an agent.
fn agent_disabled_fallback_aborts_conflict(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let repo = tmp.path();

        // Set up repo with conflict
        run_git_in(repo, &["init"]);
        run_git_in(repo, &["config", "user.email", "test@example.com"]);
        run_git_in(repo, &["config", "user.name", "Test User"]);

        fs::write(repo.join("conflict.txt"), "base\n").expect("write base");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "base"]);

        fs::write(repo.join("conflict.txt"), "master\n").expect("write master");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "master diverges"]);

        run_git_in(repo, &["checkout", "-b", "feature", "HEAD~1"]);
        fs::write(repo.join("conflict.txt"), "feature\n").expect("write feature");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "feature diverges"]);

        // Start rebase (will conflict)
        let output = std::process::Command::new("git")
            .args(["rebase", "master"])
            .current_dir(repo)
            .output()
            .expect("run git rebase");
        assert!(!output.status.success(), "expected rebase conflict");

        // Verify conflict classification
        let exit_code = output.status.code().unwrap_or(-1);
        let kind = classify_rebase_failure(exit_code, &output.stderr, repo);
        assert_eq!(
            kind,
            RebaseFailureKind::Conflict,
            "should classify as conflict"
        );

        // With None backend, runtime should NOT invoke agent — just abort
        let backend = parse_rebase_agent_backend("none").expect("parse none");
        assert_eq!(backend, RebaseAgentBackend::None);

        // Simulate what runtime.rs does for None backend: abort and fail
        let _ = std::process::Command::new("git")
            .args(["rebase", "--abort"])
            .current_dir(repo)
            .output();

        assert!(
            !is_rebase_in_progress(repo),
            "rebase should be aborted in none-backend fallback path"
        );
    })
}

/// Verify that agent failures abort rebase-in-progress and produce an error.
fn agent_failure_aborts_rebase(_h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a repo with an active rebase conflict
        let tmp = tempfile::TempDir::new().expect("create tempdir");
        let repo = tmp.path();

        // Init repo
        run_git_in(repo, &["init"]);
        run_git_in(repo, &["config", "user.email", "test@example.com"]);
        run_git_in(repo, &["config", "user.name", "Test User"]);

        // Base commit
        fs::write(repo.join("conflict.txt"), "base\n").expect("write base");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "base"]);

        // Master diverges
        fs::write(repo.join("conflict.txt"), "master\n").expect("write master");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "master diverges"]);

        // Feature branch
        run_git_in(repo, &["checkout", "-b", "feature", "HEAD~1"]);
        fs::write(repo.join("conflict.txt"), "feature\n").expect("write feature");
        run_git_in(repo, &["add", "conflict.txt"]);
        run_git_in(repo, &["commit", "-m", "feature diverges"]);

        // Start rebase (will conflict)
        let output = std::process::Command::new("git")
            .args(["rebase", "master"])
            .current_dir(repo)
            .output()
            .expect("run git rebase");
        assert!(!output.status.success(), "expected rebase conflict");
        assert!(is_rebase_in_progress(repo), "rebase should be in progress");

        // Create mock claude that exits non-zero (simulating agent failure)
        let bin_dir = tmp.path().join("mock-bin");
        fs::create_dir_all(&bin_dir).expect("create mock-bin");
        let claude_path = bin_dir.join("claude");
        fs::write(&claude_path, "#!/bin/sh\nexit 1\n").expect("write mock claude");
        let mut perms = fs::metadata(&claude_path).expect("meta").permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        fs::set_permissions(&claude_path, perms).expect("set perms");

        let _guard = PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", bin_dir.display(), old_path));

        let backend = RebaseAgentBackend::Claude {
            model: "opus".to_owned(),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let result = resolve_rebase_conflicts(repo, "master", &backend, deadline);

        std::env::set_var("PATH", &old_path);

        assert!(result.is_err(), "agent failure should return error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("non-zero"),
            "error should mention non-zero exit: {err}"
        );
        // After failure, rebase should be aborted
        assert!(
            !is_rebase_in_progress(repo),
            "rebase should be aborted after agent failure"
        );
    })
}

fn run_git_in(repo: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git command");
    assert!(
        output.status.success(),
        "git {:?} failed.\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
