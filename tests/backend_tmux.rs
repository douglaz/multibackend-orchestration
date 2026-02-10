//! Integration-style tests for TmuxBackend wrapper behavior using mocks (no tmux binary required).

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tokio::sync::Mutex;

use ralph::backend::tmux::TmuxCommandRunner;
use ralph::backend::tmux_backend::TmuxBackend;
use ralph::backend::{Backend, CliBackend};
use ralph::error::RalphError;
use ralph::Result;

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
            .push(args.iter().map(|a| (*a).to_owned()).collect());
        let mut responses = self.responses.lock().await;
        match responses.pop_front() {
            Some(result) => result,
            None => Ok(String::new()),
        }
    }
}

/// Helper to spawn a watcher task that detects a prompt file being written
/// and then writes the corresponding output and exit files.
fn spawn_file_watcher(
    session_prefix: &str,
    output_content: &str,
    exit_code: i32,
) -> tokio::task::JoinHandle<Vec<PathBuf>> {
    let prefix = session_prefix.to_owned();
    let output = output_content.to_owned();
    tokio::spawn(async move {
        let tmp_dir = std::env::temp_dir();
        loop {
            if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&format!("ralph-{prefix}-"))
                        && name.ends_with("-prompt.txt")
                    {
                        let file_prefix = name.trim_end_matches("-prompt.txt");
                        let prompt_path = tmp_dir.join(&name);
                        let output_path = tmp_dir.join(format!("{file_prefix}-output.txt"));
                        let exit_path = tmp_dir.join(format!("{file_prefix}-exit.txt"));

                        fs::write(&output_path, &output).await.unwrap();
                        fs::write(&exit_path, format!("{exit_code}\n")).await.unwrap();

                        return vec![prompt_path, output_path, exit_path];
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
}

#[tokio::test]
async fn tmux_backend_calls_sequence_ensure_create_wait_kill() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),       // has-session
        Ok("4\n".to_owned()),    // create_window
        Ok(String::new()),       // kill_window
    ]);

    let cli = CliBackend::new(
        "seq-test",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "seq-session".to_owned(), runner.clone());

    let watcher = spawn_file_watcher("seq-session", "output", 0);
    let _ = backend.execute("hello").await.unwrap();
    watcher.await.unwrap();

    let calls = runner.calls().await;
    // Exact sequence: ensure_session(has-session) -> create_window(new-window) -> kill_window
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0][0], "has-session");
    assert_eq!(calls[1][0], "new-window");
    assert_eq!(calls[2][0], "kill-window");
    assert!(calls[2].contains(&"seq-session:4".to_owned()));
}

#[tokio::test]
async fn tmux_backend_session_created_when_missing() {
    let runner = MockTmuxRunner::with_responses(vec![
        // has-session fails (no session)
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find session: new-sess".to_owned(),
        }),
        // new-session succeeds
        Ok(String::new()),
        // create_window
        Ok("0\n".to_owned()),
        // kill_window
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "sess-test",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "new-sess".to_owned(), runner.clone());

    let watcher = spawn_file_watcher("new-sess", "result", 0);
    let output = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    assert_eq!(output, "result");

    let calls = runner.calls().await;
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0][0], "has-session");
    assert_eq!(calls[1][0], "new-session"); // session was created
    assert_eq!(calls[2][0], "new-window");
    assert_eq!(calls[3][0], "kill-window");
}

#[tokio::test]
async fn tmux_backend_nonzero_exit_returns_backend_command_failed() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),
        Ok("1\n".to_owned()),
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "fail-backend",
        "mycommand".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "fail-session".to_owned(), runner);

    let watcher = spawn_file_watcher("fail-session", "some output", 42);
    let result = backend.execute("prompt").await;
    watcher.await.unwrap();

    match result {
        Err(RalphError::BackendCommandFailed { backend, details }) => {
            assert_eq!(backend, "fail-backend");
            assert!(details.contains("42"), "should include exit code: {details}");
            assert!(details.contains("mycommand"), "should include command: {details}");
        }
        other => panic!("expected BackendCommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_backend_timeout_returns_backend_timeout() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),
        Ok("1\n".to_owned()),
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "timeout-backend",
        "sleep".to_owned(),
        vec![],
        Duration::from_millis(100),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "timeout-session".to_owned(), runner);

    // No file watcher — exit file never appears
    let result = backend.execute("prompt").await;

    match result {
        Err(RalphError::BackendTimeout { backend }) => {
            assert_eq!(backend, "timeout-backend");
        }
        other => panic!("expected BackendTimeout, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_backend_temp_files_cleaned_after_success() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),
        Ok("1\n".to_owned()),
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "cleanup-ok",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "cleanup-ok-session".to_owned(), runner);

    let watcher = spawn_file_watcher("cleanup-ok-session", "done", 0);
    let _ = backend.execute("prompt").await.unwrap();
    let files = watcher.await.unwrap();

    for file in &files {
        assert!(
            !file.exists(),
            "temp file should be cleaned up: {}",
            file.display()
        );
    }
}

#[tokio::test]
async fn tmux_backend_temp_files_cleaned_after_failure() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),
        Ok("1\n".to_owned()),
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "cleanup-fail",
        "false".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "cleanup-fail-session".to_owned(), runner);

    let watcher = spawn_file_watcher("cleanup-fail-session", "err", 1);
    let _ = backend.execute("prompt").await;
    let files = watcher.await.unwrap();

    for file in &files {
        assert!(
            !file.exists(),
            "temp file should be cleaned up on failure: {}",
            file.display()
        );
    }
}

#[tokio::test]
async fn tmux_backend_preserves_backend_name() {
    let runner = MockTmuxRunner::default();
    let cli = CliBackend::new(
        "my-special-backend",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(10),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "ralph".to_owned(), runner);
    assert_eq!(backend.name(), "my-special-backend");
}

#[tokio::test]
async fn tmux_backend_command_preserves_env_and_args() {
    let mut env = BTreeMap::new();
    env.insert("API_KEY".to_owned(), "secret123".to_owned());
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),
        Ok("0\n".to_owned()),
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "env-test",
        "myapp".to_owned(),
        vec!["--verbose".to_owned(), "--mode=fast".to_owned()],
        Duration::from_secs(5),
        env,
    );
    let backend = TmuxBackend::new(cli, "env-session".to_owned(), runner.clone());

    let watcher = spawn_file_watcher("env-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    // Verify the create_window call includes the shell command with env and args
    let calls = runner.calls().await;
    let create_call = &calls[1]; // new-window call
    let shell_cmd = create_call.last().unwrap();

    assert!(shell_cmd.contains("API_KEY"), "should contain env var: {shell_cmd}");
    assert!(shell_cmd.contains("secret123"), "should contain env value: {shell_cmd}");
    assert!(shell_cmd.contains("--verbose"), "should contain --verbose arg: {shell_cmd}");
    assert!(shell_cmd.contains("--mode=fast"), "should contain --mode=fast arg: {shell_cmd}");
}

#[tokio::test]
async fn tmux_backend_no_stderr_redirect() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),
        Ok("0\n".to_owned()),
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "stderr-test",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "stderr-session".to_owned(), runner.clone());

    let watcher = spawn_file_watcher("stderr-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    let calls = runner.calls().await;
    let shell_cmd = calls[1].last().unwrap();
    assert!(!shell_cmd.contains("2>&1"), "stderr must not be redirected: {shell_cmd}");
}
