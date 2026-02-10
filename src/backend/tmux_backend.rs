use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tracing::debug;

use super::tmux::{self, TmuxCommandRunner};
use super::{Backend, CliBackend};
use crate::error::RalphError;
use crate::Result;

static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A `Backend` implementation that runs commands inside tmux windows
/// while still capturing stdout for orchestration parsing.
pub struct TmuxBackend<R: TmuxCommandRunner = tmux::RealTmuxRunner> {
    inner: CliBackend,
    session_name: String,
    runner: R,
}

impl<R: TmuxCommandRunner> TmuxBackend<R> {
    pub fn new(inner: CliBackend, session_name: String, runner: R) -> Self {
        Self {
            inner,
            session_name,
            runner,
        }
    }

    /// Build a unique prefix for temp files for this invocation.
    fn temp_file_prefix(&self) -> String {
        let id = INVOCATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        format!("ralph-{}-{}-{}", self.session_name, pid, id)
    }

    /// Build the shell command that will run inside the tmux window.
    ///
    /// The command:
    ///   1. Pipes the prompt file into the backend command via stdin
    ///   2. Redirects only stdout to the output file (stderr stays on the terminal)
    ///   3. Writes the exit code to the exit file
    fn build_shell_command(
        &self,
        prompt_file: &Path,
        output_file: &Path,
        exit_file: &Path,
    ) -> String {
        let resolved = self.inner.resolved_command_path().display().to_string();

        let mut parts: Vec<String> = Vec::new();

        // Prepend env var exports
        for (key, val) in self.inner.env() {
            parts.push(format!(
                "export {}={};",
                shell_escape(key),
                shell_escape(val)
            ));
        }

        // cat prompt | command args > output; echo $? > exit
        parts.push(format!(
            "cat {} | {} {} > {}; echo $? > {}",
            shell_escape(&prompt_file.display().to_string()),
            shell_escape(&resolved),
            self.inner
                .args()
                .iter()
                .map(|a| shell_escape(a))
                .collect::<Vec<_>>()
                .join(" "),
            shell_escape(&output_file.display().to_string()),
            shell_escape(&exit_file.display().to_string()),
        ));

        parts.join(" ")
    }
}

/// RAII guard that removes a list of temp files on drop.
struct TempFileGuard {
    paths: Vec<PathBuf>,
}

impl TempFileGuard {
    fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Shell-escape a string by wrapping in single quotes and escaping embedded
/// single quotes (the standard `'\''` trick).
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[async_trait]
impl<R: TmuxCommandRunner> Backend for TmuxBackend<R> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn execute(&self, prompt: &str) -> Result<String> {
        let prefix = self.temp_file_prefix();
        let tmp_dir = std::env::temp_dir();
        let prompt_file = tmp_dir.join(format!("{prefix}-prompt.txt"));
        let output_file = tmp_dir.join(format!("{prefix}-output.txt"));
        let exit_file = tmp_dir.join(format!("{prefix}-exit.txt"));

        // RAII cleanup for all temp files, even on early return / panic.
        let _guard = TempFileGuard::new(vec![
            prompt_file.clone(),
            output_file.clone(),
            exit_file.clone(),
        ]);

        // 1. Write prompt to temp file
        fs::write(&prompt_file, prompt)
            .await
            .map_err(|err| RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!("failed to write prompt temp file: {err}"),
            })?;

        // 2. Ensure tmux session exists
        tmux::ensure_session(&self.runner, &self.session_name).await?;

        // 3. Create tmux window with the shell command
        let shell_cmd = self.build_shell_command(&prompt_file, &output_file, &exit_file);
        let label = format!("ralph-{}", self.inner.name());

        debug!(
            backend = self.inner.name(),
            session = %self.session_name,
            label = %label,
            "creating tmux window for backend execution"
        );

        let window_id =
            tmux::create_window(&self.runner, &self.session_name, &label, &shell_cmd).await?;

        // 4. Wait for exit file (respecting backend timeout)
        let wait_result =
            tmux::wait_for_exit(&exit_file, self.inner.timeout(), POLL_INTERVAL).await;

        // 5. Best-effort window cleanup regardless of outcome
        let _ = tmux::kill_window(&self.runner, &self.session_name, &window_id).await;

        // 6. Process the result
        let exit_code = wait_result.map_err(|err| match err {
            RalphError::BackendTimeout { .. } => RalphError::BackendTimeout {
                backend: self.inner.name().to_owned(),
            },
            other => other,
        })?;

        if exit_code != 0 {
            return Err(RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!(
                    "tmux command exited with code {exit_code} (command='{}')",
                    self.inner.command()
                ),
            });
        }

        // 7. Read the captured stdout
        let output = fs::read_to_string(&output_file).await.map_err(|err| {
            RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!("failed to read output file: {err}"),
            }
        })?;

        Ok(output)
    }

    async fn health_check(&self) -> Result<()> {
        // Validate tmux is available
        tmux::check_tmux_available()?;
        // Validate the wrapped backend is available
        self.inner.health_check().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::fs;
    use tokio::sync::Mutex;

    use super::*;
    use crate::backend::tmux::TmuxCommandRunner;
    use crate::error::RalphError;

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

    fn test_cli_backend() -> CliBackend {
        CliBackend::new(
            "test-backend",
            "echo".to_owned(),
            vec!["-n".to_owned()],
            Duration::from_secs(30),
            BTreeMap::new(),
        )
    }

    fn test_cli_backend_with_env() -> CliBackend {
        let mut env = BTreeMap::new();
        env.insert("MY_VAR".to_owned(), "hello world".to_owned());
        CliBackend::new(
            "test-backend",
            "mycommand".to_owned(),
            vec!["--flag".to_owned(), "value".to_owned()],
            Duration::from_secs(60),
            env,
        )
    }

    // --- Command construction tests ---

    #[test]
    fn build_shell_command_basic() {
        let runner = MockTmuxRunner::default();
        let backend = TmuxBackend::new(test_cli_backend(), "ralph".to_owned(), runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/output.txt"),
            Path::new("/tmp/exit.txt"),
        );

        // Should pipe prompt -> command -> output, and capture exit code
        assert!(
            cmd.contains("cat '/tmp/prompt.txt'"),
            "missing cat prompt: {cmd}"
        );
        assert!(
            cmd.contains("> '/tmp/output.txt'"),
            "missing stdout redirect: {cmd}"
        );
        assert!(
            cmd.contains("echo $? > '/tmp/exit.txt'"),
            "missing exit capture: {cmd}"
        );
        // Should NOT contain 2>&1
        assert!(
            !cmd.contains("2>&1"),
            "stderr must not be redirected: {cmd}"
        );
    }

    #[test]
    fn build_shell_command_preserves_args() {
        let runner = MockTmuxRunner::default();
        let backend = TmuxBackend::new(test_cli_backend(), "ralph".to_owned(), runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/p.txt"),
            Path::new("/tmp/o.txt"),
            Path::new("/tmp/e.txt"),
        );

        assert!(cmd.contains("'-n'"), "missing -n arg: {cmd}");
    }

    #[test]
    fn build_shell_command_includes_env() {
        let runner = MockTmuxRunner::default();
        let backend = TmuxBackend::new(test_cli_backend_with_env(), "ralph".to_owned(), runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/p.txt"),
            Path::new("/tmp/o.txt"),
            Path::new("/tmp/e.txt"),
        );

        assert!(
            cmd.contains("export 'MY_VAR'='hello world'"),
            "missing env export: {cmd}"
        );
    }

    #[test]
    fn build_shell_command_escapes_single_quotes() {
        let mut env = BTreeMap::new();
        env.insert("VAR".to_owned(), "it's here".to_owned());
        let cli = CliBackend::new(
            "test",
            "cmd".to_owned(),
            vec!["--msg=it's".to_owned()],
            Duration::from_secs(10),
            env,
        );
        let runner = MockTmuxRunner::default();
        let backend = TmuxBackend::new(cli, "ralph".to_owned(), runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/p.txt"),
            Path::new("/tmp/o.txt"),
            Path::new("/tmp/e.txt"),
        );

        // Single quotes inside should be escaped as '\''
        assert!(
            cmd.contains("'it'\\''s here'"),
            "env value not escaped: {cmd}"
        );
        assert!(cmd.contains("'--msg=it'\\''s'"), "arg not escaped: {cmd}");
    }

    // --- Execute success path ---

    #[tokio::test]
    async fn execute_success_returns_output() {
        // We mock the runner so tmux calls succeed, then manually write exit + output files.

        let runner = MockTmuxRunner::with_responses(vec![
            // ensure_session: has-session succeeds
            Ok(String::new()),
            // create_window: returns window id
            Ok("1\n".to_owned()),
            // kill_window: succeeds
            Ok(String::new()),
        ]);

        let cli = CliBackend::new(
            "test-backend",
            "echo".to_owned(),
            vec![],
            Duration::from_secs(5),
            BTreeMap::new(),
        );
        let backend = TmuxBackend::new(cli, "test-success".to_owned(), runner.clone());

        // We need to intercept the execute flow. Since execute writes prompt then calls
        // create_window, and then wait_for_exit polls for the exit file, we need to
        // write the exit and output files that execute will look for.
        //
        // The file names are deterministic based on the prefix. We'll spawn a task that
        // watches for the prompt file to appear (indicating execute has started) then
        // writes the output and exit files.
        let watcher = tokio::spawn(async move {
            // Poll for any file matching ralph-test-success-*-prompt.txt in tmp dir
            let tmp_dir = std::env::temp_dir();
            loop {
                if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("ralph-test-success-") && name.ends_with("-prompt.txt")
                        {
                            let prefix = name.trim_end_matches("-prompt.txt");
                            let output_path = tmp_dir.join(format!("{prefix}-output.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));
                            fs::write(&output_path, "hello from tmux").await.unwrap();
                            fs::write(&exit_path, "0\n").await.unwrap();
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let result = backend.execute("test prompt").await;
        watcher.await.unwrap();

        let output = result.unwrap();
        assert_eq!(output, "hello from tmux");

        // Verify tmux call sequence
        let calls = runner.calls().await;
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0][0], "has-session"); // ensure_session
        assert_eq!(calls[1][0], "new-window"); // create_window
        assert_eq!(calls[2][0], "kill-window"); // cleanup
    }

    // --- Execute non-zero exit ---

    #[tokio::test]
    async fn execute_nonzero_exit_returns_error() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),    // has-session
            Ok("2\n".to_owned()), // create_window
            Ok(String::new()),    // kill_window
        ]);

        let cli = CliBackend::new(
            "failing-backend",
            "false".to_owned(),
            vec![],
            Duration::from_secs(5),
            BTreeMap::new(),
        );
        let backend = TmuxBackend::new(cli, "test-nonzero".to_owned(), runner);

        // Watcher that writes non-zero exit
        let watcher = tokio::spawn(async move {
            let tmp_dir = std::env::temp_dir();
            loop {
                if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("ralph-test-nonzero-") && name.ends_with("-prompt.txt")
                        {
                            let prefix = name.trim_end_matches("-prompt.txt");
                            let output_path = tmp_dir.join(format!("{prefix}-output.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));
                            fs::write(&output_path, "error output").await.unwrap();
                            fs::write(&exit_path, "1\n").await.unwrap();
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let result = backend.execute("test prompt").await;
        watcher.await.unwrap();

        match result {
            Err(RalphError::BackendCommandFailed { backend, details }) => {
                assert_eq!(backend, "failing-backend");
                assert!(details.contains("exited with code 1"), "details: {details}");
            }
            other => panic!("expected BackendCommandFailed, got: {other:?}"),
        }
    }

    // --- Timeout test ---

    #[tokio::test]
    async fn execute_timeout_returns_backend_timeout() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),    // has-session
            Ok("3\n".to_owned()), // create_window
            Ok(String::new()),    // kill_window
        ]);

        let cli = CliBackend::new(
            "slow-backend",
            "sleep".to_owned(),
            vec![],
            Duration::from_millis(100), // Very short timeout
            BTreeMap::new(),
        );
        let backend = TmuxBackend::new(cli, "test-timeout".to_owned(), runner);

        // Don't write any exit file — should timeout
        let result = backend.execute("test prompt").await;

        match result {
            Err(RalphError::BackendTimeout { backend }) => {
                assert_eq!(backend, "slow-backend");
            }
            other => panic!("expected BackendTimeout, got: {other:?}"),
        }
    }

    // --- Cleanup test ---

    #[tokio::test]
    async fn temp_files_cleaned_up_on_success() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),
            Ok("1\n".to_owned()),
            Ok(String::new()),
        ]);

        let cli = CliBackend::new(
            "clean-backend",
            "echo".to_owned(),
            vec![],
            Duration::from_secs(5),
            BTreeMap::new(),
        );
        let backend = TmuxBackend::new(cli, "clean-session".to_owned(), runner);

        let created_files: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let created_files_clone = created_files.clone();

        let watcher = tokio::spawn(async move {
            let tmp_dir = std::env::temp_dir();
            loop {
                if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("ralph-clean-session-") && name.ends_with("-prompt.txt")
                        {
                            let prefix = name.trim_end_matches("-prompt.txt");
                            let prompt_path = tmp_dir.join(&name);
                            let output_path = tmp_dir.join(format!("{prefix}-output.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));

                            fs::write(&output_path, "ok").await.unwrap();
                            fs::write(&exit_path, "0\n").await.unwrap();

                            let mut files = created_files_clone.lock().await;
                            files.push(prompt_path);
                            files.push(output_path);
                            files.push(exit_path);
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let _ = backend.execute("test").await.unwrap();
        watcher.await.unwrap();

        // After execute returns, temp files should be cleaned up by the guard
        let files = created_files.lock().await;
        for file in files.iter() {
            assert!(
                !file.exists(),
                "temp file should be cleaned up: {}",
                file.display()
            );
        }
    }

    #[tokio::test]
    async fn temp_files_cleaned_up_on_failure() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),
            Ok("1\n".to_owned()),
            Ok(String::new()),
        ]);

        let cli = CliBackend::new(
            "fail-clean",
            "false".to_owned(),
            vec![],
            Duration::from_secs(5),
            BTreeMap::new(),
        );
        let backend = TmuxBackend::new(cli, "fail-clean-session".to_owned(), runner);

        let created_files: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
        let created_files_clone = created_files.clone();

        let watcher = tokio::spawn(async move {
            let tmp_dir = std::env::temp_dir();
            loop {
                if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("ralph-fail-clean-session-")
                            && name.ends_with("-prompt.txt")
                        {
                            let prefix = name.trim_end_matches("-prompt.txt");
                            let prompt_path = tmp_dir.join(&name);
                            let output_path = tmp_dir.join(format!("{prefix}-output.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));

                            fs::write(&output_path, "error output").await.unwrap();
                            fs::write(&exit_path, "42\n").await.unwrap();

                            let mut files = created_files_clone.lock().await;
                            files.push(prompt_path);
                            files.push(output_path);
                            files.push(exit_path);
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let _ = backend.execute("test").await; // Should fail
        watcher.await.unwrap();

        let files = created_files.lock().await;
        for file in files.iter() {
            assert!(
                !file.exists(),
                "temp file should be cleaned up on failure: {}",
                file.display()
            );
        }
    }

    // --- Shell escape tests ---

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_spaces() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn shell_escape_with_single_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    // --- Name delegation ---

    #[test]
    fn name_delegates_to_inner() {
        let runner = MockTmuxRunner::default();
        let cli = test_cli_backend();
        let backend = TmuxBackend::new(cli, "ralph".to_owned(), runner);
        assert_eq!(backend.name(), "test-backend");
    }

    // --- Kill window called even on non-zero exit ---

    #[tokio::test]
    async fn kill_window_called_on_nonzero_exit() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),    // has-session
            Ok("5\n".to_owned()), // create_window
            Ok(String::new()),    // kill_window
        ]);

        let cli = CliBackend::new(
            "kill-test",
            "false".to_owned(),
            vec![],
            Duration::from_secs(5),
            BTreeMap::new(),
        );
        let backend = TmuxBackend::new(cli, "test-killwin".to_owned(), runner.clone());

        let watcher = tokio::spawn(async move {
            let tmp_dir = std::env::temp_dir();
            loop {
                if let Ok(mut entries) = tokio::fs::read_dir(&tmp_dir).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("ralph-test-killwin-") && name.ends_with("-prompt.txt")
                        {
                            let prefix = name.trim_end_matches("-prompt.txt");
                            let output_path = tmp_dir.join(format!("{prefix}-output.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));
                            fs::write(&output_path, "").await.unwrap();
                            fs::write(&exit_path, "1\n").await.unwrap();
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let _ = backend.execute("test").await;
        watcher.await.unwrap();

        let calls = runner.calls().await;
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2][0], "kill-window");
        assert!(calls[2].contains(&"test-killwin:5".to_owned()));
    }
}
