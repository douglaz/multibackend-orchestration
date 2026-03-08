use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tracing::debug;

use super::tmux::{self, TmuxCommandRunner};
use super::{Backend, CliBackend, SharedTmuxContext};
use crate::error::{RalphError, TimeoutKind};
use crate::output_log::LogWriter;
use crate::Result;

static INVOCATION_COUNTER: AtomicU64 = AtomicU64::new(0);

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Per-invocation context that the orchestrator sets before calling `execute()`.
/// This allows the orchestrator to pass loop/role information without changing
/// the `Backend` trait signature.
#[derive(Debug, Clone, Default)]
pub struct TmuxExecutionContext {
    pub loop_number: Option<u32>,
    pub role: Option<String>,
    pub loop_dir: Option<PathBuf>,
    /// When set, `build_shell_command` will delegate to `effective_args()` for
    /// session-aware argument rewriting. If rewriting fails, it falls back to
    /// the default args.
    pub session_id: Option<String>,
}

/// A `Backend` implementation that runs commands inside tmux windows
/// while still capturing stdout for orchestration parsing.
pub struct TmuxBackend<R: TmuxCommandRunner = tmux::RealTmuxRunner> {
    inner: CliBackend,
    session_name: String,
    runner: R,
    window_keep_seconds: u64,
    shared_context: SharedTmuxContext,
}

impl<R: TmuxCommandRunner> TmuxBackend<R> {
    pub fn new(
        inner: CliBackend,
        session_name: String,
        runner: R,
        window_keep_seconds: u64,
        shared_context: SharedTmuxContext,
    ) -> Self {
        Self {
            inner,
            session_name,
            runner,
            window_keep_seconds,
            shared_context,
        }
    }

    /// Build a contextual window label from the current execution context.
    fn build_label(&self, ctx: &TmuxExecutionContext) -> String {
        match (&ctx.loop_number, &ctx.role) {
            (Some(loop_num), Some(role)) => {
                tmux::format_window_label(*loop_num, role, self.inner.name())
            }
            _ => format!("ralph-{}", self.inner.name()),
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
    ///   2. Pipes stdout through `tee` so it's visible in the tmux pane AND captured to the output file
    ///   3. Captures stderr to a dedicated file
    ///   4. Writes the exit code to the exit file
    ///
    /// When the execution context contains a `session_id`, this method
    /// delegates to `CliBackend::effective_args()` for session-aware arg
    /// rewriting. If rewriting fails, it falls back to the default args.
    fn build_shell_command(
        &self,
        prompt_file: &Path,
        output_file: &Path,
        stderr_file: &Path,
        exit_file: &Path,
        ctx: &TmuxExecutionContext,
    ) -> String {
        let resolved = self.inner.resolved_command_path().display().to_string();

        // Try session-aware arg rewriting when a session_id is present.
        let args = if let Some(ref session_id) = ctx.session_id {
            let invocation_ctx = super::BackendInvocationContext {
                loop_dir: ctx.loop_dir.clone().unwrap_or_default(),
                role: ctx.role.clone().unwrap_or_default(),
                session_id: Some(session_id.clone()),
                json_output_required: true,
            };
            match self.inner.effective_args(&invocation_ctx) {
                Ok(rewritten) => rewritten,
                Err(e) => {
                    debug!(
                        backend = self.inner.name(),
                        error = %e,
                        "effective_args rewrite failed, falling back to default args"
                    );
                    self.inner.args().to_vec()
                }
            }
        } else {
            self.inner.args().to_vec()
        };

        let mut parts: Vec<String> = Vec::new();

        // Strip daemon-only env vars so tmux backend subprocesses
        // don't inherit them (mirrors CliBackend::execute_streaming).
        for var in super::SANITIZED_ENV_VARS {
            parts.push(format!("unset {};", shell_escape(var)));
        }

        // Prepend env var exports
        for (key, val) in self.inner.env() {
            parts.push(format!(
                "export {}={};",
                shell_escape(key),
                shell_escape(val)
            ));
        }

        // cat prompt | command args 2>stderr | tee output; echo ${PIPESTATUS[1]} > exit
        parts.push(format!(
            "cat {} | {} {} 2>{} | tee {}; echo ${{PIPESTATUS[1]}} > {}",
            shell_escape(&prompt_file.display().to_string()),
            shell_escape(&resolved),
            args.iter()
                .map(|a| shell_escape(a))
                .collect::<Vec<_>>()
                .join(" "),
            shell_escape(&stderr_file.display().to_string()),
            shell_escape(&output_file.display().to_string()),
            shell_escape(&exit_file.display().to_string()),
        ));

        parts.join(" ")
    }

    /// Execute the tmux-backed command and return raw stdout, stderr, and exit
    /// code without interpreting the exit code. Both `execute()` and
    /// `execute_with_log()` delegate here so that log writes can happen
    /// regardless of exit status.
    async fn execute_raw(&self, prompt: &str) -> Result<TmuxRawOutput> {
        let prefix = self.temp_file_prefix();
        let tmp_dir = std::env::temp_dir();
        let prompt_file = tmp_dir.join(format!("{prefix}-prompt.txt"));
        let output_file = tmp_dir.join(format!("{prefix}-output.txt"));
        let stderr_file = tmp_dir.join(format!("{prefix}-stderr.txt"));
        let exit_file = tmp_dir.join(format!("{prefix}-exit.txt"));

        // RAII cleanup for all temp files, even on early return / panic.
        let _guard = TempFileGuard::new(vec![
            prompt_file.clone(),
            output_file.clone(),
            stderr_file.clone(),
            exit_file.clone(),
        ]);

        // Snapshot the execution context for this invocation. We use get()
        // (not take()) so that the context remains available for retry
        // invocations triggered by parse/timeout retries in the orchestrator.
        let ctx = self.shared_context.get().await;

        // 1. Write prompt to temp file
        fs::write(&prompt_file, prompt)
            .await
            .map_err(|err| RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!("failed to write prompt temp file: {err}"),
            })?;

        // 2. Ensure tmux session exists
        tmux::ensure_session(&self.runner, &self.session_name).await?;

        // 2b. Create stderr capture file so the redirect target exists
        fs::write(&stderr_file, "")
            .await
            .map_err(|err| RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!("failed to create stderr capture file: {err}"),
            })?;

        // 3. Create tmux window with the shell command (with retry on session loss)
        let shell_cmd =
            self.build_shell_command(&prompt_file, &output_file, &stderr_file, &exit_file, &ctx);
        let label = self.build_label(&ctx);

        debug!(
            backend = self.inner.name(),
            session = %self.session_name,
            label = %label,
            "creating tmux window for backend execution"
        );

        let window_id =
            tmux::create_window_with_retry(&self.runner, &self.session_name, &label, &shell_cmd)
                .await?;

        // 3b. Enable remain-on-exit so the window stays visible after the command
        // process exits. Without this, the window closes immediately on exit and
        // users cannot inspect it during the retention period.
        if self.window_keep_seconds > 0 {
            if let Err(err) =
                tmux::set_remain_on_exit(&self.runner, &self.session_name, &window_id).await
            {
                debug!(
                    error = %err,
                    "failed to set remain-on-exit (non-fatal, window may close on completion)"
                );
            }
        }

        // 4. Wait for exit file with inactivity tracking via capture-file growth
        let wait_result = tmux::wait_for_exit_with_activity(
            &exit_file,
            &[output_file.as_path(), stderr_file.as_path()],
            self.inner.timeout(),
            POLL_INTERVAL,
        )
        .await;

        // 5. Classify timeout cause BEFORE cleanup.
        //
        // We must check whether the window/session still exists before
        // kill_window_best_effort, because a successful kill-window would
        // make has_window report false, misclassifying genuine timeouts as
        // external disappearance.
        let exit_code = match wait_result {
            Ok(code) => code,
            Err(RalphError::BackendTimeout {
                idle_seconds: measured_idle,
                ..
            }) => {
                // Check the specific window, not just the session. A disappeared
                // window in a still-alive session should be reported as a command
                // failure with actionable diagnostics, not as a timeout.
                let window_alive = tmux::has_window(&self.runner, &self.session_name, &window_id)
                    .await
                    .unwrap_or(false);

                // Best-effort cleanup after classification
                tmux::kill_window_best_effort(&self.runner, &self.session_name, &window_id).await;

                if window_alive {
                    // Window still alive — this is a genuine idle timeout.
                    // Propagate as BackendTimeout so orchestrator can retry.
                    debug!(
                        backend = self.inner.name(),
                        role = ?ctx.role,
                        "skipping backend output artifact: tmux command timed out before output capture"
                    );
                    return Err(RalphError::BackendTimeout {
                        backend: self.inner.name().to_owned(),
                        idle_seconds: measured_idle,
                        timeout_kind: TimeoutKind::Idle,
                    });
                }
                debug!(
                    backend = self.inner.name(),
                    role = ?ctx.role,
                    "skipping backend output artifact: tmux window disappeared before output capture"
                );
                // Window (or session) is gone — external interruption.
                return Err(RalphError::BackendCommandFailed {
                    backend: self.inner.name().to_owned(),
                    details: format!(
                        "tmux window '{}' (id={}) for backend '{}' disappeared or timed out \
                         before the exit file was written. The tmux window may have been \
                         closed externally. Re-run the command or check your tmux session '{}'.",
                        label,
                        window_id,
                        self.inner.command(),
                        self.session_name
                    ),
                });
            }
            Err(other) => return Err(other),
        };

        // 6. Retention delay before cleanup (only on success)
        let keep_seconds = self.window_keep_seconds;
        if keep_seconds > 0 {
            debug!(
                keep_seconds = keep_seconds,
                window = %window_id,
                "keeping completed tmux window for inspection"
            );
            tokio::time::sleep(Duration::from_secs(keep_seconds)).await;
        }

        // 7. Best-effort window cleanup
        tmux::kill_window_best_effort(&self.runner, &self.session_name, &window_id).await;

        // 8. Read captured stdout before interpreting exit code so we can
        // persist artifacts even for non-zero exits.
        let output_bytes = match fs::read(&output_file).await {
            Ok(bytes) => bytes,
            Err(err) => {
                if exit_code == 0 {
                    return Err(RalphError::BackendCommandFailed {
                        backend: self.inner.name().to_owned(),
                        details: format!("failed to read output file: {err}"),
                    });
                }
                debug!(
                    backend = self.inner.name(),
                    role = ?ctx.role,
                    path = %output_file.display(),
                    error = %err,
                    "non-zero tmux command exit without readable output capture"
                );
                Vec::new()
            }
        };

        let stderr_bytes = match fs::read(&stderr_file).await {
            Ok(bytes) => bytes,
            Err(err) => {
                debug!(
                    backend = self.inner.name(),
                    path = %stderr_file.display(),
                    error = %err,
                    "could not read stderr capture file (non-fatal)"
                );
                Vec::new()
            }
        };

        debug!(
            backend = self.inner.name(),
            role = ?ctx.role,
            exit_code = exit_code,
            stdout_len = output_bytes.len(),
            stderr_len = stderr_bytes.len(),
            "tmux backend output captured (not persisted to loop dir)"
        );

        Ok(TmuxRawOutput {
            exit_code,
            stdout: output_bytes,
            stderr: stderr_bytes,
        })
    }
}

/// Raw output captured from a tmux execution, before interpreting the exit code.
struct TmuxRawOutput {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
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
        let raw = self.execute_raw(prompt).await?;

        if raw.exit_code != 0 {
            return Err(RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!(
                    "tmux command exited with code {} (command='{}')",
                    raw.exit_code,
                    self.inner.command()
                ),
            });
        }

        Ok(String::from_utf8_lossy(&raw.stdout).to_string())
    }

    async fn execute_with_log(
        &self,
        prompt: &str,
        mut log_writer: Option<&mut LogWriter>,
    ) -> Result<String> {
        let raw = self.execute_raw(prompt).await?;

        // Persist stdout and stderr to LogWriter (routed to `.ralph/tmp/logs`)
        // for both success and failure paths. This ensures tmux diagnostics are
        // always captured in tmp logs while keeping loop dirs artifact-free.
        if let Some(writer) = log_writer.as_mut() {
            if !raw.stdout.is_empty() {
                writer.write_bytes(&raw.stdout);
            }
            if !raw.stderr.is_empty() {
                writer.write_str("\n=== STDERR ===\n");
                writer.write_bytes(&raw.stderr);
            }
        }

        if raw.exit_code != 0 {
            return Err(RalphError::BackendCommandFailed {
                backend: self.inner.name().to_owned(),
                details: format!(
                    "tmux command exited with code {} (command='{}')",
                    raw.exit_code,
                    self.inner.command()
                ),
            });
        }

        Ok(String::from_utf8_lossy(&raw.stdout).to_string())
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

    fn test_shared_ctx() -> SharedTmuxContext {
        SharedTmuxContext::default()
    }

    fn make_backend(
        cli: CliBackend,
        session: &str,
        runner: MockTmuxRunner,
    ) -> TmuxBackend<MockTmuxRunner> {
        TmuxBackend::new(cli, session.to_owned(), runner, 0, test_shared_ctx())
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
        let backend = make_backend(test_cli_backend(), "ralph", runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/prompt.txt"),
            Path::new("/tmp/output.txt"),
            Path::new("/tmp/stderr.txt"),
            Path::new("/tmp/exit.txt"),
            &TmuxExecutionContext::default(),
        );

        // Should pipe prompt -> command -> output, and capture exit code
        assert!(
            cmd.contains("cat '/tmp/prompt.txt'"),
            "missing cat prompt: {cmd}"
        );
        assert!(
            cmd.contains("2>'/tmp/stderr.txt'"),
            "missing stderr redirect: {cmd}"
        );
        assert!(
            cmd.contains("| tee '/tmp/output.txt'"),
            "missing tee stdout: {cmd}"
        );
        assert!(
            cmd.contains("echo ${PIPESTATUS[1]} > '/tmp/exit.txt'"),
            "missing exit capture: {cmd}"
        );
        // Should NOT contain 2>&1
        assert!(!cmd.contains("2>&1"), "stderr must not be merged: {cmd}");
    }

    #[test]
    fn build_shell_command_preserves_args() {
        let runner = MockTmuxRunner::default();
        let backend = make_backend(test_cli_backend(), "ralph", runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/p.txt"),
            Path::new("/tmp/o.txt"),
            Path::new("/tmp/se.txt"),
            Path::new("/tmp/e.txt"),
            &TmuxExecutionContext::default(),
        );

        assert!(cmd.contains("'-n'"), "missing -n arg: {cmd}");
    }

    #[test]
    fn build_shell_command_includes_env() {
        let runner = MockTmuxRunner::default();
        let backend = make_backend(test_cli_backend_with_env(), "ralph", runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/p.txt"),
            Path::new("/tmp/o.txt"),
            Path::new("/tmp/se.txt"),
            Path::new("/tmp/e.txt"),
            &TmuxExecutionContext::default(),
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
        let backend = make_backend(cli, "ralph", runner);

        let cmd = backend.build_shell_command(
            Path::new("/tmp/p.txt"),
            Path::new("/tmp/o.txt"),
            Path::new("/tmp/se.txt"),
            Path::new("/tmp/e.txt"),
            &TmuxExecutionContext::default(),
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
        let backend = TmuxBackend::new(
            cli,
            "test-success".to_owned(),
            runner.clone(),
            0,
            test_shared_ctx(),
        );

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
        let backend =
            TmuxBackend::new(cli, "test-nonzero".to_owned(), runner, 0, test_shared_ctx());

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

    // --- Timeout tests ---

    #[tokio::test]
    async fn execute_genuine_timeout_returns_backend_timeout() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),    // has-session (ensure_session)
            Ok("3\n".to_owned()), // create_window
            Ok("3\n".to_owned()), // has_window (list-windows) — window still exists (classified BEFORE cleanup)
            Ok(String::new()),    // kill_window (best-effort, after classification)
        ]);

        let cli = CliBackend::new(
            "slow-backend",
            "sleep".to_owned(),
            vec![],
            Duration::from_millis(100), // Very short timeout
            BTreeMap::new(),
        );
        let backend =
            TmuxBackend::new(cli, "test-timeout".to_owned(), runner, 0, test_shared_ctx());

        // Don't write any exit file — genuine timeout with session still alive.
        let result = backend.execute("test prompt").await;

        match result {
            Err(RalphError::BackendTimeout {
                backend,
                timeout_kind: TimeoutKind::Idle,
                ..
            }) => {
                assert_eq!(backend, "slow-backend");
            }
            other => panic!("expected BackendTimeout, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_window_disappeared_returns_actionable_error() {
        let runner = MockTmuxRunner::with_responses(vec![
            Ok(String::new()),    // has-session (ensure_session)
            Ok("3\n".to_owned()), // create_window
            // has_window (list-windows) check BEFORE cleanup — window gone
            Err(RalphError::BackendCommandFailed {
                backend: "tmux".to_owned(),
                details: "can't find window: 3".to_owned(),
            }),
            Ok(String::new()), // kill_window (best-effort, after classification)
        ]);

        let cli = CliBackend::new(
            "slow-backend",
            "sleep".to_owned(),
            vec![],
            Duration::from_millis(100), // Very short timeout
            BTreeMap::new(),
        );
        let backend =
            TmuxBackend::new(cli, "test-timeout".to_owned(), runner, 0, test_shared_ctx());

        // Don't write any exit file — timeout + session disappeared externally.
        let result = backend.execute("test prompt").await;

        match result {
            Err(RalphError::BackendCommandFailed { backend, details }) => {
                assert_eq!(backend, "slow-backend");
                assert!(
                    details.contains("disappeared or timed out"),
                    "expected actionable diagnostics: {details}"
                );
            }
            other => panic!("expected BackendCommandFailed with diagnostics, got: {other:?}"),
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
        let backend = TmuxBackend::new(
            cli,
            "clean-session".to_owned(),
            runner,
            0,
            test_shared_ctx(),
        );

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
                            let stderr_path = tmp_dir.join(format!("{prefix}-stderr.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));

                            fs::write(&output_path, "ok").await.unwrap();
                            fs::write(&exit_path, "0\n").await.unwrap();

                            let mut files = created_files_clone.lock().await;
                            files.push(prompt_path);
                            files.push(output_path);
                            files.push(stderr_path);
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
        let backend = TmuxBackend::new(
            cli,
            "fail-clean-session".to_owned(),
            runner,
            0,
            test_shared_ctx(),
        );

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
                            let stderr_path = tmp_dir.join(format!("{prefix}-stderr.txt"));
                            let exit_path = tmp_dir.join(format!("{prefix}-exit.txt"));

                            fs::write(&output_path, "error output").await.unwrap();
                            fs::write(&exit_path, "42\n").await.unwrap();

                            let mut files = created_files_clone.lock().await;
                            files.push(prompt_path);
                            files.push(output_path);
                            files.push(stderr_path);
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
        let backend = make_backend(cli, "ralph", runner);
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
        let backend = TmuxBackend::new(
            cli,
            "test-killwin".to_owned(),
            runner.clone(),
            0,
            test_shared_ctx(),
        );

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
