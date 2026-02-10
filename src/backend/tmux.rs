use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Instant};
use tracing::debug;

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

    use super::{create_window, ensure_session, kill_window, wait_for_exit, TmuxCommandRunner};
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
}
