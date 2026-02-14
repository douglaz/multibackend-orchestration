use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Instant};
use tracing::{debug, warn};

use crate::error::RalphError;
use crate::Result;

#[async_trait]
pub trait TmuxCommandRunner: Send + Sync {
    async fn run(&self, args: &[&str]) -> Result<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RealTmuxRunner;

#[async_trait]
impl TmuxCommandRunner for RealTmuxRunner {
    async fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("tmux")
            .args(args)
            .output()
            .await
            .map_err(|err| match err.kind() {
                ErrorKind::NotFound => RalphError::TmuxUnavailable,
                _ => RalphError::BackendCommandFailed {
                    backend: "tmux".to_owned(),
                    details: format!("failed to run tmux {:?}: {err}", args),
                },
            })?;

        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("tmux exited with status {}", output.status)
        };

        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: format!("{details} (args: {:?})", args),
        })
    }
}

pub fn check_tmux_available() -> Result<()> {
    which::which("tmux")
        .map(|_| ())
        .map_err(|_| RalphError::TmuxUnavailable)
}

pub async fn ensure_session<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
) -> Result<()> {
    match runner.run(&["has-session", "-t", session_name]).await {
        Ok(_) => Ok(()),
        Err(RalphError::BackendCommandFailed { details, .. })
            if looks_like_missing_session(&details) =>
        {
            debug!(session = session_name, "tmux session missing, creating");
            runner
                .run(&["new-session", "-d", "-s", session_name])
                .await
                .map(|_| ())
        }
        Err(err) => Err(err),
    }
}

pub async fn create_window<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
    label: &str,
    shell_command: &str,
) -> Result<String> {
    let window_id = runner
        .run(&[
            "new-window",
            "-P",
            "-F",
            "#{window_index}",
            "-t",
            session_name,
            "-n",
            label,
            shell_command,
        ])
        .await?;

    let window_id = window_id.trim().to_owned();
    if window_id.is_empty() {
        return Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "tmux new-window returned an empty window identifier".to_owned(),
        });
    }

    Ok(window_id)
}

pub async fn wait_for_exit(
    exit_file_path: &Path,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<i32> {
    let started = Instant::now();

    loop {
        if exit_file_path.exists() {
            let raw_exit = fs::read_to_string(exit_file_path).await?;
            let trimmed = raw_exit.trim();
            let exit_code = trimmed
                .parse::<i32>()
                .map_err(|err| RalphError::CorruptedState {
                    path: exit_file_path.to_path_buf(),
                    reason: format!("invalid tmux exit code '{trimmed}': {err}"),
                })?;
            return Ok(exit_code);
        }

        if started.elapsed() >= timeout {
            return Err(RalphError::BackendTimeout {
                backend: "tmux".to_owned(),
            });
        }

        sleep(poll_interval).await;
    }
}

pub async fn kill_window<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
    window_id: &str,
) -> Result<()> {
    let target = format!("{session_name}:{window_id}");
    runner.run(&["kill-window", "-t", &target]).await?;
    Ok(())
}

fn looks_like_missing_session(details: &str) -> bool {
    let normalized = details.to_ascii_lowercase();
    normalized.contains("can't find session") || normalized.contains("no server running")
}

fn looks_like_missing_window(details: &str) -> bool {
    let normalized = details.to_ascii_lowercase();
    normalized.contains("can't find window") || normalized.contains("no such window")
}

/// Check whether a tmux session exists.
pub async fn has_session<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
) -> Result<bool> {
    match runner.run(&["has-session", "-t", session_name]).await {
        Ok(_) => Ok(true),
        Err(RalphError::BackendCommandFailed { details, .. })
            if looks_like_missing_session(&details) =>
        {
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

/// Create a tmux window with one retry: if the session disappears between our
/// check and the actual `new-window`, we re-ensure the session and retry once.
pub async fn create_window_with_retry<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
    label: &str,
    shell_command: &str,
) -> Result<String> {
    match create_window(runner, session_name, label, shell_command).await {
        Ok(id) => Ok(id),
        Err(RalphError::BackendCommandFailed { ref details, .. })
            if looks_like_missing_session(details) =>
        {
            debug!(
                session = session_name,
                "session vanished before window creation, re-ensuring and retrying"
            );
            ensure_session(runner, session_name).await?;
            create_window(runner, session_name, label, shell_command).await
        }
        Err(err) => Err(err),
    }
}

/// Enable `remain-on-exit` on a tmux window so the pane stays visible after
/// the command process exits. This is needed for the retention-delay feature:
/// without it, completed command windows close immediately on process exit
/// before the keep-seconds sleep even starts.
pub async fn set_remain_on_exit<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
    window_id: &str,
) -> Result<()> {
    let target = format!("{session_name}:{window_id}");
    runner
        .run(&["set-option", "-t", &target, "remain-on-exit", "on"])
        .await?;
    Ok(())
}

/// Check whether a specific tmux window exists within a session.
pub async fn has_window<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
    window_id: &str,
) -> Result<bool> {
    let target = format!("{session_name}:{window_id}");
    match runner
        .run(&["list-windows", "-t", &target, "-F", "#{window_index}"])
        .await
    {
        Ok(_) => Ok(true),
        Err(RalphError::BackendCommandFailed { details, .. })
            if looks_like_missing_session(&details) || looks_like_missing_window(&details) =>
        {
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

/// Best-effort window cleanup. Logs a warning on failure instead of returning
/// an error, so that session/window disappearance during cleanup never causes
/// a hard failure.
pub async fn kill_window_best_effort<R: TmuxCommandRunner + ?Sized>(
    runner: &R,
    session_name: &str,
    window_id: &str,
) {
    if let Err(err) = kill_window(runner, session_name, window_id).await {
        warn!(
            session = session_name,
            window = window_id,
            error = %err,
            "best-effort window cleanup failed (window may have been closed externally)"
        );
    }
}

/// Maximum length for a tmux window label. Tmux itself allows long names, but
/// we truncate for readability and terminal width concerns.
const MAX_LABEL_LEN: usize = 32;

/// Generate a deterministic, tmux-safe window label from execution context.
///
/// Format: `L{loop}-{role}-{backend}` (e.g. `L3-impl-codex`).
/// Invalid characters (anything not alphanumeric, dash, or underscore) are
/// replaced with dashes, and the result is truncated to [`MAX_LABEL_LEN`].
pub fn format_window_label(loop_number: u32, role: &str, backend: &str) -> String {
    let raw = format!("L{loop_number}-{role}-{backend}");
    sanitize_tmux_label(&raw)
}

/// Sanitize a string for use as a tmux window name.
///
/// Replaces any character that is not alphanumeric, dash, or underscore with
/// a dash, collapses consecutive dashes, strips leading/trailing dashes, and
/// truncates to [`MAX_LABEL_LEN`].
fn sanitize_tmux_label(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            prev_dash = ch == '-';
            result.push(ch);
        } else if !prev_dash && !result.is_empty() {
            result.push('-');
            prev_dash = true;
        }
    }
    // Trim trailing dash
    while result.ends_with('-') {
        result.pop();
    }
    // Truncate to max length (break at char boundary, which is always safe for ASCII)
    if result.len() > MAX_LABEL_LEN {
        result.truncate(MAX_LABEL_LEN);
        // Remove any trailing dash after truncation
        while result.ends_with('-') {
            result.pop();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use tempfile::tempdir;
    use tokio::fs;
    use tokio::sync::Mutex;
    use tokio::time::sleep;

    use super::{
        create_window, create_window_with_retry, ensure_session, format_window_label, has_session,
        has_window, kill_window, kill_window_best_effort, sanitize_tmux_label, set_remain_on_exit,
        wait_for_exit, TmuxCommandRunner,
    };
    use crate::error::RalphError;
    use crate::Result;

    #[derive(Clone, Default)]
    struct MockTmuxRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        responses: Arc<Mutex<VecDeque<Result<String>>>>,
    }

    impl MockTmuxRunner {
        fn with_responses(responses: Vec<Result<String>>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            }
        }

        async fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().await.clone()
        }
    }

    #[async_trait]
    impl TmuxCommandRunner for MockTmuxRunner {
        async fn run(&self, args: &[&str]) -> Result<String> {
            self.calls
                .lock()
                .await
                .push(args.iter().map(|arg| (*arg).to_owned()).collect());
            let mut responses = self.responses.lock().await;
            match responses.pop_front() {
                Some(result) => result,
                None => Ok(String::new()),
            }
        }
    }

    #[tokio::test]
    async fn ensure_session_skips_creation_when_session_exists() {
        let runner = MockTmuxRunner::with_responses(vec![Ok(String::new())]);
        ensure_session(&runner, "ralph").await.unwrap();

        let calls = runner.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], vec!["has-session", "-t", "ralph"]);
    }

    #[tokio::test]
    async fn ensure_session_creates_when_missing() {
        let runner = MockTmuxRunner::with_responses(vec![
            Err(RalphError::BackendCommandFailed {
                backend: "tmux".to_owned(),
                details: "can't find session: ralph".to_owned(),
            }),
            Ok(String::new()),
        ]);

        ensure_session(&runner, "ralph").await.unwrap();
        let calls = runner.calls().await;

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], vec!["has-session", "-t", "ralph"]);
        assert_eq!(calls[1], vec!["new-session", "-d", "-s", "ralph"]);
    }

    #[tokio::test]
    async fn create_window_returns_identifier() {
        let runner = MockTmuxRunner::with_responses(vec![Ok("7\n".to_owned())]);
        let window_id = create_window(&runner, "ralph", "L1-impl-codex", "echo hi")
            .await
            .unwrap();

        assert_eq!(window_id, "7");

        let calls = runner.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec![
                "new-window",
                "-P",
                "-F",
                "#{window_index}",
                "-t",
                "ralph",
                "-n",
                "L1-impl-codex",
                "echo hi"
            ]
        );
    }

    #[tokio::test]
    async fn wait_for_exit_returns_zero_exit_code() {
        let dir = tempdir().unwrap();
        let exit_file = dir.path().join("exit.txt");
        let writer_path: PathBuf = exit_file.clone();

        tokio::spawn(async move {
            sleep(Duration::from_millis(20)).await;
            fs::write(writer_path, "0\n").await.unwrap();
        });

        let exit_code = wait_for_exit(
            &exit_file,
            Duration::from_secs(1),
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn wait_for_exit_times_out() {
        let dir = tempdir().unwrap();
        let exit_file = dir.path().join("missing-exit.txt");

        let result = wait_for_exit(
            &exit_file,
            Duration::from_millis(40),
            Duration::from_millis(10),
        )
        .await;
        assert!(matches!(
            result,
            Err(RalphError::BackendTimeout { backend }) if backend == "tmux"
        ));
    }

    #[tokio::test]
    async fn wait_for_exit_returns_non_zero_exit_code() {
        let dir = tempdir().unwrap();
        let exit_file = dir.path().join("exit.txt");
        fs::write(&exit_file, "17\n").await.unwrap();

        let exit_code = wait_for_exit(
            &exit_file,
            Duration::from_secs(1),
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        assert_eq!(exit_code, 17);
    }

    #[tokio::test]
    async fn kill_window_invokes_kill_window() {
        let runner = MockTmuxRunner::with_responses(vec![Ok(String::new())]);
        kill_window(&runner, "ralph", "3").await.unwrap();

        let calls = runner.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], vec!["kill-window", "-t", "ralph:3"]);
    }

    // --- has_session tests ---

    #[tokio::test]
    async fn has_session_returns_true_when_session_exists() {
        let runner = MockTmuxRunner::with_responses(vec![Ok(String::new())]);
        assert!(has_session(&runner, "ralph").await.unwrap());
    }

    #[tokio::test]
    async fn has_session_returns_false_when_session_missing() {
        let runner = MockTmuxRunner::with_responses(vec![Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find session: ralph".to_owned(),
        })]);
        assert!(!has_session(&runner, "ralph").await.unwrap());
    }

    // --- create_window_with_retry tests ---

    #[tokio::test]
    async fn create_window_with_retry_succeeds_on_first_try() {
        let runner = MockTmuxRunner::with_responses(vec![Ok("5\n".to_owned())]);
        let id = create_window_with_retry(&runner, "ralph", "L1-impl-codex", "echo hi")
            .await
            .unwrap();
        assert_eq!(id, "5");
        assert_eq!(runner.calls().await.len(), 1);
    }

    #[tokio::test]
    async fn create_window_with_retry_retries_on_missing_session() {
        let runner = MockTmuxRunner::with_responses(vec![
            // First create_window fails - session gone
            Err(RalphError::BackendCommandFailed {
                backend: "tmux".to_owned(),
                details: "can't find session: ralph".to_owned(),
            }),
            // ensure_session: has-session fails
            Err(RalphError::BackendCommandFailed {
                backend: "tmux".to_owned(),
                details: "can't find session: ralph".to_owned(),
            }),
            // ensure_session: new-session succeeds
            Ok(String::new()),
            // Retry create_window succeeds
            Ok("2\n".to_owned()),
        ]);

        let id = create_window_with_retry(&runner, "ralph", "L1-impl-codex", "echo hi")
            .await
            .unwrap();
        assert_eq!(id, "2");
        let calls = runner.calls().await;
        assert_eq!(calls.len(), 4);
    }

    // --- kill_window_best_effort tests ---

    #[tokio::test]
    async fn kill_window_best_effort_succeeds_silently() {
        let runner = MockTmuxRunner::with_responses(vec![Ok(String::new())]);
        kill_window_best_effort(&runner, "ralph", "3").await;
        assert_eq!(runner.calls().await.len(), 1);
    }

    #[tokio::test]
    async fn kill_window_best_effort_does_not_panic_on_error() {
        let runner = MockTmuxRunner::with_responses(vec![Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find window: 3".to_owned(),
        })]);
        kill_window_best_effort(&runner, "ralph", "3").await;
        // Should not panic
    }

    // --- label formatting tests ---

    #[test]
    fn format_window_label_basic() {
        assert_eq!(format_window_label(3, "impl", "codex"), "L3-impl-codex");
    }

    #[test]
    fn format_window_label_long_values_truncated() {
        let long_role = "a".repeat(50);
        let label = format_window_label(1, &long_role, "backend");
        assert!(label.len() <= 32, "label too long: {label}");
        assert!(label.starts_with("L1-"));
    }

    #[test]
    fn sanitize_tmux_label_strips_invalid_chars() {
        assert_eq!(sanitize_tmux_label("L3-impl codex!"), "L3-impl-codex");
    }

    #[test]
    fn sanitize_tmux_label_collapses_replacement_dashes() {
        // The sanitizer replaces runs of invalid characters with a single dash,
        // but does NOT collapse pre-existing consecutive dashes in the input.
        assert_eq!(sanitize_tmux_label("a--b---c"), "a--b---c");
        // Multiple consecutive invalid chars produce a single replacement dash:
        assert_eq!(sanitize_tmux_label("a!!b"), "a-b");
        assert_eq!(sanitize_tmux_label("a!! !!b"), "a-b");
    }

    #[test]
    fn sanitize_tmux_label_strips_trailing_dash() {
        assert_eq!(sanitize_tmux_label("hello-"), "hello");
    }

    #[test]
    fn format_window_label_planner_role() {
        assert_eq!(
            format_window_label(1, "planner", "claude"),
            "L1-planner-claude"
        );
    }

    #[test]
    fn format_window_label_reviewer_role() {
        assert_eq!(
            format_window_label(2, "reviewer", "codex"),
            "L2-reviewer-codex"
        );
    }

    // --- set_remain_on_exit tests ---

    #[tokio::test]
    async fn set_remain_on_exit_sends_correct_command() {
        let runner = MockTmuxRunner::with_responses(vec![Ok(String::new())]);
        set_remain_on_exit(&runner, "ralph", "5").await.unwrap();

        let calls = runner.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec!["set-option", "-t", "ralph:5", "remain-on-exit", "on"]
        );
    }

    // --- has_window tests ---

    #[tokio::test]
    async fn has_window_returns_true_when_window_exists() {
        let runner = MockTmuxRunner::with_responses(vec![Ok("3\n".to_owned())]);
        assert!(has_window(&runner, "ralph", "3").await.unwrap());

        let calls = runner.calls().await;
        assert_eq!(calls[0][0], "list-windows");
        assert!(calls[0].contains(&"ralph:3".to_owned()));
    }

    #[tokio::test]
    async fn has_window_returns_false_when_window_missing() {
        let runner = MockTmuxRunner::with_responses(vec![Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find window: 99".to_owned(),
        })]);
        assert!(!has_window(&runner, "ralph", "99").await.unwrap());
    }

    #[tokio::test]
    async fn has_window_returns_false_when_session_missing() {
        let runner = MockTmuxRunner::with_responses(vec![Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find session: ralph".to_owned(),
        })]);
        assert!(!has_window(&runner, "ralph", "3").await.unwrap());
    }
}
