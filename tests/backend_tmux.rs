//! Integration-style tests for TmuxBackend wrapper behavior using mocks (no tmux binary required).

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tokio::sync::Mutex;

use ralph::backend::tmux::TmuxCommandRunner;
use ralph::backend::tmux_backend::{TmuxBackend, TmuxExecutionContext};
use ralph::backend::{Backend, CliBackend, SharedTmuxContext};
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

fn make_backend(
    cli: CliBackend,
    session: &str,
    runner: MockTmuxRunner,
) -> TmuxBackend<MockTmuxRunner> {
    TmuxBackend::new(
        cli,
        session.to_owned(),
        runner,
        0,
        SharedTmuxContext::default(),
    )
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
                        fs::write(&exit_path, format!("{exit_code}\n"))
                            .await
                            .unwrap();

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
        Ok(String::new()),    // has-session
        Ok("4\n".to_owned()), // create_window
        Ok(String::new()),    // kill_window
    ]);

    let cli = CliBackend::new(
        "seq-test",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "seq-session", runner.clone());

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
    let backend = make_backend(cli, "new-sess", runner.clone());

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
    let backend = make_backend(cli, "fail-session", runner);

    let watcher = spawn_file_watcher("fail-session", "some output", 42);
    let result = backend.execute("prompt").await;
    watcher.await.unwrap();

    match result {
        Err(RalphError::BackendCommandFailed { backend, details }) => {
            assert_eq!(backend, "fail-backend");
            assert!(
                details.contains("42"),
                "should include exit code: {details}"
            );
            assert!(
                details.contains("mycommand"),
                "should include command: {details}"
            );
        }
        other => panic!("expected BackendCommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_backend_genuine_timeout_returns_backend_timeout() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok("1\n".to_owned()), // has_window (list-windows) — classified BEFORE cleanup
        Ok(String::new()),    // kill_window (best-effort, after classification)
    ]);

    let cli = CliBackend::new(
        "timeout-backend",
        "sleep".to_owned(),
        vec![],
        Duration::from_millis(100),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "timeout-session", runner);

    // No file watcher — genuine timeout, session still alive → BackendTimeout
    let result = backend.execute("prompt").await;

    match result {
        Err(RalphError::BackendTimeout { backend }) => {
            assert_eq!(backend, "timeout-backend");
        }
        other => panic!("expected BackendTimeout, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_backend_timeout_with_missing_session_returns_actionable_error() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        // has_window (list-windows) check BEFORE cleanup — session gone
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find session: timeout-session".to_owned(),
        }),
        Ok(String::new()), // kill_window (best-effort, after classification)
    ]);

    let cli = CliBackend::new(
        "timeout-backend",
        "sleep".to_owned(),
        vec![],
        Duration::from_millis(100),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "timeout-session", runner);

    // No file watcher — timeout + session disappeared → BackendCommandFailed
    let result = backend.execute("prompt").await;

    match result {
        Err(RalphError::BackendCommandFailed { backend, details }) => {
            assert_eq!(backend, "timeout-backend");
            assert!(
                details.contains("disappeared or timed out"),
                "expected actionable diagnostics: {details}"
            );
        }
        other => panic!("expected BackendCommandFailed with diagnostics, got: {other:?}"),
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
    let backend = make_backend(cli, "cleanup-ok-session", runner);

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
    let backend = make_backend(cli, "cleanup-fail-session", runner);

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
    let backend = make_backend(cli, "ralph", runner);
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
    let backend = make_backend(cli, "env-session", runner.clone());

    let watcher = spawn_file_watcher("env-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    // Verify the create_window call includes the shell command with env and args
    let calls = runner.calls().await;
    let create_call = &calls[1]; // new-window call
    let shell_cmd = create_call.last().unwrap();

    assert!(
        shell_cmd.contains("API_KEY"),
        "should contain env var: {shell_cmd}"
    );
    assert!(
        shell_cmd.contains("secret123"),
        "should contain env value: {shell_cmd}"
    );
    assert!(
        shell_cmd.contains("--verbose"),
        "should contain --verbose arg: {shell_cmd}"
    );
    assert!(
        shell_cmd.contains("--mode=fast"),
        "should contain --mode=fast arg: {shell_cmd}"
    );
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
    let backend = make_backend(cli, "stderr-session", runner.clone());

    let watcher = spawn_file_watcher("stderr-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    let calls = runner.calls().await;
    let shell_cmd = calls[1].last().unwrap();
    assert!(
        !shell_cmd.contains("2>&1"),
        "stderr must not be redirected: {shell_cmd}"
    );
}

// --- Contextual label tests ---

#[tokio::test]
async fn tmux_backend_uses_contextual_label_from_shared_context() {
    let shared_ctx = SharedTmuxContext::default();
    shared_ctx
        .set(TmuxExecutionContext {
            loop_number: Some(3),
            role: Some("impl".to_owned()),
        })
        .await;

    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok(String::new()),    // kill_window
    ]);

    let cli = CliBackend::new(
        "codex",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = TmuxBackend::new(cli, "ctx-session".to_owned(), runner.clone(), 0, shared_ctx);

    let watcher = spawn_file_watcher("ctx-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    let calls = runner.calls().await;
    // The create_window call (calls[1]) should contain the label "L3-impl-codex"
    let create_call = &calls[1];
    assert!(
        create_call.contains(&"L3-impl-codex".to_owned()),
        "expected contextual label L3-impl-codex in: {create_call:?}"
    );
}

#[tokio::test]
async fn tmux_backend_falls_back_to_generic_label_without_context() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok(String::new()),    // kill_window
    ]);

    let cli = CliBackend::new(
        "codex",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "noctx-session", runner.clone());

    let watcher = spawn_file_watcher("noctx-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();

    let calls = runner.calls().await;
    let create_call = &calls[1];
    assert!(
        create_call.contains(&"ralph-codex".to_owned()),
        "expected fallback label ralph-codex in: {create_call:?}"
    );
}

// --- Session retry test ---

#[tokio::test]
async fn tmux_backend_retries_window_creation_on_session_loss() {
    let runner = MockTmuxRunner::with_responses(vec![
        // ensure_session: has-session succeeds (session exists initially)
        Ok(String::new()),
        // create_window: fails because session was removed between check and create
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find session: retry-session".to_owned(),
        }),
        // create_window_with_retry: re-ensure: has-session fails
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find session: retry-session".to_owned(),
        }),
        // create_window_with_retry: re-ensure: new-session succeeds
        Ok(String::new()),
        // Retry create_window succeeds
        Ok("2\n".to_owned()),
        // kill_window
        Ok(String::new()),
    ]);

    let cli = CliBackend::new(
        "retry-backend",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "retry-session", runner.clone());

    let watcher = spawn_file_watcher("retry-session", "output", 0);
    let result = backend.execute("prompt").await;
    watcher.await.unwrap();

    assert!(result.is_ok(), "should succeed after retry: {result:?}");
    assert_eq!(result.unwrap(), "output");
}

// --- Best-effort cleanup test ---

#[tokio::test]
async fn tmux_backend_does_not_fail_on_window_cleanup_error() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        // kill_window fails (window already removed)
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find window: 1".to_owned(),
        }),
    ]);

    let cli = CliBackend::new(
        "cleanup-err-backend",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "cleanup-err-session", runner);

    let watcher = spawn_file_watcher("cleanup-err-session", "done", 0);
    let result = backend.execute("prompt").await;
    watcher.await.unwrap();

    // Should succeed even though cleanup failed
    assert!(
        result.is_ok(),
        "should succeed despite cleanup error: {result:?}"
    );
    assert_eq!(result.unwrap(), "done");
}

// --- Retention timing tests ---

#[tokio::test]
async fn tmux_backend_keep_seconds_zero_cleans_up_immediately() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok(String::new()),    // kill_window
    ]);

    let cli = CliBackend::new(
        "keep0-backend",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    // window_keep_seconds = 0 means immediate cleanup
    let backend = TmuxBackend::new(
        cli,
        "keep0-session".to_owned(),
        runner.clone(),
        0,
        SharedTmuxContext::default(),
    );

    let start = std::time::Instant::now();
    let watcher = spawn_file_watcher("keep0-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();
    let elapsed = start.elapsed();

    // With keep_seconds=0, cleanup should not add any noticeable delay.
    // Allow some slack for file I/O but it should be well under 1 second.
    assert!(
        elapsed < Duration::from_secs(2),
        "keep_seconds=0 should not delay cleanup: elapsed {:?}",
        elapsed
    );

    // Verify kill_window was called
    let calls = runner.calls().await;
    assert_eq!(calls.last().unwrap()[0], "kill-window");
}

#[tokio::test]
async fn tmux_backend_keep_seconds_nonzero_delays_cleanup() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok(String::new()),    // set-option (remain-on-exit)
        Ok(String::new()),    // kill_window
    ]);

    let cli = CliBackend::new(
        "keep2-backend",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    // window_keep_seconds = 1 means 1-second retention
    let backend = TmuxBackend::new(
        cli,
        "keep2-session".to_owned(),
        runner.clone(),
        1,
        SharedTmuxContext::default(),
    );

    let start = std::time::Instant::now();
    let watcher = spawn_file_watcher("keep2-session", "ok", 0);
    let _ = backend.execute("prompt").await.unwrap();
    watcher.await.unwrap();
    let elapsed = start.elapsed();

    // With keep_seconds=1, the cleanup should be delayed by at least ~1 second.
    assert!(
        elapsed >= Duration::from_millis(900),
        "keep_seconds=1 should delay cleanup by ~1s: elapsed {:?}",
        elapsed
    );

    // Verify kill_window was still called
    let calls = runner.calls().await;
    assert_eq!(calls.last().unwrap()[0], "kill-window");
}

#[tokio::test]
async fn tmux_backend_keep_seconds_skipped_on_failure() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok(String::new()),    // set-option (remain-on-exit, since keep_seconds=5)
        Ok("1\n".to_owned()), // has_window (list-windows) — classified BEFORE cleanup
        Ok(String::new()),    // kill_window (best-effort, after classification)
    ]);

    let cli = CliBackend::new(
        "keepfail-backend",
        "echo".to_owned(),
        vec![],
        Duration::from_millis(100), // Short timeout
        BTreeMap::new(),
    );
    // window_keep_seconds = 5 but should NOT wait on failure
    let backend = TmuxBackend::new(
        cli,
        "keepfail-session".to_owned(),
        runner.clone(),
        5,
        SharedTmuxContext::default(),
    );

    let start = std::time::Instant::now();
    // Don't write exit file — will timeout
    let _ = backend.execute("prompt").await;
    let elapsed = start.elapsed();

    // Even though keep_seconds=5, on failure the keep delay is skipped.
    // Total time should be roughly the timeout (100ms) + cleanup, not 5+ seconds.
    assert!(
        elapsed < Duration::from_secs(3),
        "keep_seconds should be skipped on failure: elapsed {:?}",
        elapsed
    );
}

// --- Context preserved across retries ---

#[tokio::test]
async fn tmux_backend_context_preserved_across_multiple_executions() {
    // Verify that context is NOT consumed by take() but rather read by get(),
    // so retries see the same label.
    let shared_ctx = SharedTmuxContext::default();
    shared_ctx
        .set(TmuxExecutionContext {
            loop_number: Some(5),
            role: Some("reviewer".to_owned()),
        })
        .await;

    // First execution
    let runner1 = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("1\n".to_owned()), // create_window
        Ok(String::new()),    // kill_window
    ]);

    let cli1 = CliBackend::new(
        "codex",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend1 = TmuxBackend::new(
        cli1,
        "retry-ctx-session".to_owned(),
        runner1.clone(),
        0,
        shared_ctx.clone(),
    );

    let watcher1 = spawn_file_watcher("retry-ctx-session", "ok1", 0);
    let _ = backend1.execute("prompt1").await.unwrap();
    watcher1.await.unwrap();

    let calls1 = runner1.calls().await;
    assert!(
        calls1[1].contains(&"L5-reviewer-codex".to_owned()),
        "first execution should use contextual label: {:?}",
        calls1[1]
    );

    // Second execution with same shared context (simulating retry)
    let runner2 = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session
        Ok("2\n".to_owned()), // create_window
        Ok(String::new()),    // kill_window
    ]);
    let cli2 = CliBackend::new(
        "codex",
        "echo".to_owned(),
        vec![],
        Duration::from_secs(5),
        BTreeMap::new(),
    );
    let backend2 = TmuxBackend::new(
        cli2,
        "retry-ctx2-session".to_owned(),
        runner2.clone(),
        0,
        shared_ctx.clone(),
    );

    let watcher2 = spawn_file_watcher("retry-ctx2-session", "ok2", 0);
    let _ = backend2.execute("prompt2").await.unwrap();
    watcher2.await.unwrap();

    let calls2 = runner2.calls().await;
    assert!(
        calls2[1].contains(&"L5-reviewer-codex".to_owned()),
        "second execution (retry) should still use contextual label: {:?}",
        calls2[1]
    );
}

// --- Window disappearance during execution ---

/// Focused test: session is alive but the specific window was closed externally
/// before the exit file was written. This must return BackendCommandFailed with
/// actionable diagnostics (not BackendTimeout).
#[tokio::test]
async fn tmux_backend_session_alive_but_window_gone_returns_command_failed() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session (ensure_session)
        Ok("7\n".to_owned()), // create_window
        // has_window (list-windows) check BEFORE cleanup — window gone
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "no such window: 7".to_owned(),
        }),
        Ok(String::new()), // kill_window (best-effort, after classification)
    ]);

    let cli = CliBackend::new(
        "window-gone-backend",
        "mycommand".to_owned(),
        vec![],
        Duration::from_millis(100),
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "alive-session", runner);

    // Don't write exit file — simulates window killed while session lives
    let result = backend.execute("prompt").await;

    match result {
        Err(RalphError::BackendCommandFailed { backend, details }) => {
            assert_eq!(backend, "window-gone-backend");
            assert!(
                details.contains("disappeared or timed out"),
                "should contain actionable message: {details}"
            );
            assert!(
                details.contains("alive-session"),
                "should mention session: {details}"
            );
            assert!(details.contains("7"), "should mention window id: {details}");
        }
        other => panic!("expected BackendCommandFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_backend_returns_actionable_error_on_window_disappearance() {
    let runner = MockTmuxRunner::with_responses(vec![
        Ok(String::new()),    // has-session (ensure_session)
        Ok("1\n".to_owned()), // create_window
        // has_window (list-windows) check BEFORE cleanup — window is gone
        Err(RalphError::BackendCommandFailed {
            backend: "tmux".to_owned(),
            details: "can't find window: 1".to_owned(),
        }),
        Ok(String::new()), // kill_window (best-effort, after classification)
    ]);

    let cli = CliBackend::new(
        "disappear-backend",
        "mycommand".to_owned(),
        vec![],
        Duration::from_millis(100), // Very short timeout
        BTreeMap::new(),
    );
    let backend = make_backend(cli, "disappear-session", runner);

    // Don't write exit file — simulates window disappearance
    let result = backend.execute("prompt").await;

    match result {
        Err(RalphError::BackendCommandFailed { backend, details }) => {
            assert_eq!(backend, "disappear-backend");
            assert!(
                details.contains("disappeared or timed out"),
                "should contain actionable message: {details}"
            );
            assert!(
                details.contains("disappear-session"),
                "should mention session: {details}"
            );
        }
        other => panic!("expected BackendCommandFailed, got: {other:?}"),
    }
}
