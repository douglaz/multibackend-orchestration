pub mod claude;
pub mod codex;
pub mod mock;
pub mod openrouter;
pub mod output_normalizer;
pub mod tmux;
pub mod tmux_backend;

pub use mock::MockBackend;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex, Notify};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::config::global::BackendEnabled;
use crate::config::GlobalConfig;
use crate::error::{RalphError, TimeoutKind};
use crate::output_log::LogWriter;
use crate::project::state::{CompletionLoopBackends, FeatureLoopBackends};
use crate::Result;

use self::tmux::RealTmuxRunner;
use self::tmux_backend::{TmuxBackend, TmuxExecutionContext};

/// Environment variables that must be stripped from backend subprocess
/// environments. Prevents in-process daemon tasks from leaking daemon-only
/// env vars (e.g. `CLAUDECODE`) to backend child processes.
pub const SANITIZED_ENV_VARS: &[&str] = &["CLAUDECODE"];

/// Emergency guard that SIGKILL-s a child process group on drop.
///
/// This is a last-resort fallback for unexpected future drops (e.g. task
/// abort). Normal cancellation and timeout paths call `kill_and_reap_child`
/// directly for cooperative SIGTERM → SIGKILL shutdown and **disarm** this
/// guard before returning.
///
/// The drop is non-blocking: it sends SIGKILL immediately and does a
/// non-blocking waitpid to reap the leader zombie.
struct KillOnDrop(Option<u32>);

impl KillOnDrop {
    /// Disarm the guard so Drop becomes a no-op.
    fn disarm(&mut self) {
        self.0 = None;
    }
}

/// Grace period before escalating from SIGTERM to SIGKILL.
const KILL_GRACE_SECONDS: u64 = 5;

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(pgid) = self.0 {
            if let Ok(raw) = i32::try_from(pgid) {
                if raw == 0 {
                    return; // Never kill our own process group
                }
                // Emergency path: SIGKILL immediately. The normal cancellation
                // path already performed cooperative SIGTERM → wait → SIGKILL
                // via `kill_and_reap_child` before disarming this guard.
                // If we reach here, the future was dropped without cleanup
                // (e.g. task abort), so hard-kill is appropriate.
                // SAFETY: Sending signals to a process group is safe.
                unsafe {
                    libc::kill(-(raw), libc::SIGKILL);
                    libc::waitpid(raw, std::ptr::null_mut(), libc::WNOHANG);
                }
            }
        }
    }
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, prompt: &str) -> Result<String>;
    async fn execute_with_log(
        &self,
        prompt: &str,
        _log_writer: Option<&mut LogWriter>,
    ) -> Result<String> {
        self.execute(prompt).await
    }
    /// Execute with cancellation support. On cancellation, backend
    /// subprocesses are killed and reaped before returning `Cancelled`.
    ///
    /// The default implementation races `execute_with_log` against the
    /// cancellation token. Backends that spawn subprocesses (e.g.
    /// `CliBackend`) override this to perform synchronous cleanup so that
    /// backend processes are guaranteed dead before this method returns.
    ///
    /// **WARNING**: When the cancel branch wins the `select!`, the
    /// `execute_with_log` future is dropped. If that future has spawned
    /// a child process, dropping the future does NOT kill the child.
    /// Backends that spawn subprocesses MUST override this method to
    /// perform explicit child process cleanup on cancellation (see
    /// `CliBackend::execute_streaming` for the reference implementation
    /// with `KillOnDrop` guard).
    async fn execute_with_cancel(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
        cancel: &CancellationToken,
    ) -> Result<String> {
        tokio::select! {
            result = self.execute_with_log(prompt, log_writer) => result,
            _ = cancel.cancelled() => Err(RalphError::Cancelled),
        }
    }
    async fn health_check(&self) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSpec {
    pub optional: bool,
    pub name: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleOverrides {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub qa: Option<String>,
    pub completer: Option<String>,
}

pub fn parse_backend_spec(spec: &str) -> Result<BackendSpec> {
    let mut spec = spec.trim();
    if spec.is_empty() {
        return Err(RalphError::Validation(
            "backend spec must not be empty".to_owned(),
        ));
    }

    let optional = if let Some(stripped) = spec.strip_prefix('?') {
        spec = stripped.trim();
        true
    } else {
        false
    };

    if spec.is_empty() {
        return Err(RalphError::Validation(
            "backend name must not be empty in spec".to_owned(),
        ));
    }
    if spec.contains('?') {
        return Err(RalphError::Validation(format!(
            "invalid backend spec format: {spec}"
        )));
    }

    let open_count = spec.matches('(').count();
    let close_count = spec.matches(')').count();

    if open_count == 0 && close_count == 0 {
        return Ok(BackendSpec {
            optional,
            name: spec.to_owned(),
            model: None,
        });
    }

    if open_count != 1 || close_count != 1 || !spec.ends_with(')') {
        return Err(RalphError::Validation(format!(
            "invalid backend spec format: {spec}"
        )));
    }

    let open_idx = spec
        .find('(')
        .ok_or_else(|| RalphError::Validation(format!("invalid backend spec format: {spec}")))?;
    let name = &spec[..open_idx];
    let model = &spec[open_idx + 1..spec.len() - 1];

    if name.is_empty() {
        return Err(RalphError::Validation(format!(
            "backend name must not be empty in spec: {spec}"
        )));
    }
    if model.is_empty() {
        return Err(RalphError::Validation(format!(
            "backend model must not be empty in spec: {spec}"
        )));
    }

    Ok(BackendSpec {
        optional,
        name: name.to_owned(),
        model: Some(model.to_owned()),
    })
}

/// Context provided by the orchestrator for each backend invocation,
/// enabling session-aware argument rewriting.
#[derive(Debug, Clone)]
pub struct BackendInvocationContext {
    pub loop_dir: PathBuf,
    pub role: String,
    pub session_id: Option<String>,
    pub json_output_required: bool,
}

/// Shared invocation context for session-aware arg rewriting.
/// The orchestrator sets this before each backend call so both tmux and
/// non-tmux CliBackend instances can pick up the session_id.
#[derive(Clone, Default)]
pub struct SharedInvocationContext(Arc<Mutex<Option<BackendInvocationContext>>>);

impl SharedInvocationContext {
    pub async fn set(&self, ctx: Option<BackendInvocationContext>) {
        *self.0.lock().await = ctx;
    }

    pub async fn get(&self) -> Option<BackendInvocationContext> {
        self.0.lock().await.clone()
    }
}

impl std::fmt::Debug for SharedInvocationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedInvocationContext(..)")
    }
}

#[derive(Debug, Clone)]
pub struct CliBackend {
    name: String,
    command: String,
    args: Vec<String>,
    timeout: Duration,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
    /// Shared invocation context for session-aware arg rewriting in non-tmux mode.
    /// When set, `execute_streaming` uses `effective_args()` instead of raw `self.args`.
    invocation_ctx: SharedInvocationContext,
}

impl CliBackend {
    pub fn new(
        name: &str,
        command: String,
        args: Vec<String>,
        timeout: Duration,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            command,
            args,
            timeout,
            env,
            cwd: None,
            invocation_ctx: SharedInvocationContext::default(),
        }
    }

    pub fn with_cwd(mut self, cwd: Option<PathBuf>) -> Self {
        self.cwd = cwd;
        self
    }

    pub fn resolved_command_path(&self) -> PathBuf {
        which::which(&self.command).unwrap_or_else(|_| PathBuf::from(&self.command))
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Rewrite CLI args for session resume and/or JSON output.
    /// When a session_id is provided, adds resume flags and --output-format json.
    /// When json_output_required is true but no session_id, adds only JSON output flags
    /// so the normalizer can extract a session_id from the first invocation.
    pub fn effective_args(&self, ctx: &BackendInvocationContext) -> Result<Vec<String>> {
        match &ctx.session_id {
            Some(id) => match self.name.as_str() {
                n if n.starts_with("claude") || n == "claude" => self.effective_args_claude(id),
                n if n.starts_with("openrouter") || n == "openrouter" => {
                    self.effective_args_goose(id)
                }
                n if n.starts_with("codex") || n == "codex" => self.effective_args_codex(id),
                _ => Ok(self.args.clone()),
            },
            None if ctx.json_output_required => self.ensure_json_output_args(),
            None => Ok(self.args.clone()),
        }
    }

    /// Add JSON output flags without session resume args.
    /// For Claude: adds --output-format json (keeps -p).
    /// For Codex: adds --json.
    fn ensure_json_output_args(&self) -> Result<Vec<String>> {
        match self.name.as_str() {
            n if n.starts_with("claude") || n == "claude" => {
                let mut args = self.args.clone();
                // Only add if not already present
                if !args
                    .iter()
                    .any(|a| a == "--output-format" || a.starts_with("--output-format="))
                {
                    args.push("--output-format".to_owned());
                    args.push("json".to_owned());
                }
                Ok(args)
            }
            n if n.starts_with("codex") || n == "codex" => {
                let mut args = self.args.clone();
                if !args.contains(&"--json".to_owned()) {
                    // Insert --json before trailing "-" if present
                    if args.last().map(|s| s.as_str()) == Some("-") {
                        let pos = args.len() - 1;
                        args.insert(pos, "--json".to_owned());
                    } else {
                        args.push("--json".to_owned());
                    }
                }
                Ok(args)
            }
            n if n.starts_with("openrouter") || n == "openrouter" => {
                let mut args = Vec::with_capacity(self.args.len() + 2);
                let mut skip_next = false;
                for arg in &self.args {
                    if skip_next {
                        skip_next = false;
                        continue;
                    }
                    if arg == "--output-format" {
                        skip_next = true;
                        continue;
                    }
                    if arg.starts_with("--output-format=") {
                        continue;
                    }
                    args.push(arg.clone());
                }
                args.push("--output-format".to_owned());
                args.push("stream-json".to_owned());
                Ok(args)
            }
            _ => Ok(self.args.clone()),
        }
    }

    fn effective_args_claude(&self, session_id: &str) -> Result<Vec<String>> {
        // Claude resume rules:
        // 1. Require -p in base args; if missing => Err
        // 2. Remove -p
        // 3. Ensure exactly one --resume <id>
        // 4. Ensure exactly one --output-format json
        if !self.args.contains(&"-p".to_owned()) {
            return Err(RalphError::Validation(
                "claude backend args must include -p for session resume".to_owned(),
            ));
        }

        let mut result = Vec::new();
        let mut skip_next = false;

        for arg in self.args.iter() {
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg == "-p" {
                // Remove -p
                continue;
            }
            if arg == "--resume" {
                // Skip existing --resume and its value
                skip_next = true;
                continue;
            }
            if arg == "--output-format" {
                // Skip existing --output-format and its value
                skip_next = true;
                continue;
            }
            // Handle --resume=value form
            if arg.starts_with("--resume=") {
                continue;
            }
            if arg.starts_with("--output-format=") {
                continue;
            }
            result.push(arg.clone());
        }

        // Add --resume <id>
        result.push("--resume".to_owned());
        result.push(session_id.to_owned());

        // Add --output-format json
        result.push("--output-format".to_owned());
        result.push("json".to_owned());

        Ok(result)
    }

    fn effective_args_codex(&self, session_id: &str) -> Result<Vec<String>> {
        // Codex resume rules:
        // 1. Require valid `exec ... -` base form; otherwise Validation error
        // 2. Deterministically produce: exec resume <id> [flags...] --json -
        // 3. Ensure exactly one --json, one session ID, trailing "-"
        // 4. Idempotent across repeated calls
        if self.args.last().map(|s| s.as_str()) != Some("-") {
            return Err(RalphError::Validation(
                "codex backend args must end with '-' for session resume".to_owned(),
            ));
        }
        // Require `exec` in base args (either plain `exec` or `exec resume ...`)
        if !self.args.contains(&"exec".to_owned()) {
            return Err(RalphError::Validation(
                "codex backend args must contain 'exec' for session resume (expected `exec ... -` form)".to_owned(),
            ));
        }

        // Collect non-structural flags (skip exec, resume, old session ids, --json, -)
        let mut flags: Vec<String> = Vec::new();
        let mut i = 0;
        let args = &self.args;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-" || arg == "--json" || arg == "exec" {
                if arg == "exec" {
                    // Skip "exec" and then skip "resume <id>" if present
                    i += 1;
                    if i < args.len() && args[i] == "resume" {
                        i += 1; // skip "resume"
                        if i < args.len()
                            && args[i] != "-"
                            && args[i] != "--json"
                            && !args[i].starts_with("--")
                        {
                            i += 1; // skip old session id
                        }
                    }
                    continue;
                }
                i += 1;
                continue;
            }
            flags.push(arg.clone());
            i += 1;
        }

        // Build deterministic form: exec resume <id> [flags...] --json -
        let mut result = vec![
            "exec".to_owned(),
            "resume".to_owned(),
            session_id.to_owned(),
        ];
        result.extend(flags);
        result.push("--json".to_owned());
        result.push("-".to_owned());

        Ok(result)
    }

    fn effective_args_goose(&self, session_id: &str) -> Result<Vec<String>> {
        // Goose resume rules:
        // 1. Keep all existing args.
        // 2. Remove existing --name, --session-id, --output-format forms and --resume flag.
        // 3. Add --name <id> --resume --output-format stream-json.
        let mut result = Vec::new();
        let mut skip_next = false;

        for arg in self.args.iter() {
            if skip_next {
                skip_next = false;
                continue;
            }
            if arg == "--name" || arg == "-n" || arg == "--session-id" || arg == "--output-format" {
                skip_next = true;
                continue;
            }
            if arg.starts_with("--name=")
                || arg.starts_with("--session-id=")
                || arg.starts_with("--output-format=")
            {
                continue;
            }
            if arg == "--resume" || arg == "-r" {
                continue;
            }
            result.push(arg.clone());
        }

        result.push("--name".to_owned());
        result.push(session_id.to_owned());
        result.push("--resume".to_owned());
        result.push("--output-format".to_owned());
        result.push("stream-json".to_owned());
        Ok(result)
    }

    async fn collect_stderr(
        &self,
        handle: tokio::task::JoinHandle<Result<Vec<u8>>>,
    ) -> Result<Vec<u8>> {
        match handle.await {
            Ok(result) => result,
            Err(err) => Err(RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: format!("stderr reader task failed: {err}"),
            }),
        }
    }

    async fn execute_streaming(
        &self,
        prompt: &str,
        mut log_writer: Option<&mut LogWriter>,
        cancel: &CancellationToken,
    ) -> Result<String> {
        // Compute effective args: if an invocation context is set, use
        // effective_args() for session-aware arg rewriting and/or JSON output
        // flags. On failure, fall back to base args.
        let effective_args = {
            let ctx_opt = self.invocation_ctx.get().await;
            match ctx_opt {
                Some(ref ctx) => match self.effective_args(ctx) {
                    Ok(args) => args,
                    Err(e) => {
                        debug!(
                            backend = self.name,
                            error = %e,
                            "effective_args rewrite failed in CliBackend, using base args"
                        );
                        self.args.clone()
                    }
                },
                None => self.args.clone(),
            }
        };

        let resolved_command = self.resolved_command_path();
        let mut cmd = Command::new(&resolved_command);
        if let Some(ref cwd) = self.cwd {
            cmd.current_dir(cwd);
        }
        cmd.args(&effective_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .envs(self.env.clone());

        // Strip daemon-only env vars so backend subprocesses don't inherit them.
        for var in SANITIZED_ENV_VARS {
            cmd.env_remove(var);
        }

        // Place the child in its own process group so that
        // `kill(-(pid), SIGKILL)` reliably terminates it and all its
        // descendants on timeout.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = cmd
            .spawn()
            .map_err(|err| RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: format!(
                    "{err} (command='{}', resolved='{}')",
                    self.command,
                    resolved_command.display()
                ),
            })?;

        // Guard: kill the child's process group if this future is dropped
        // (e.g. due to cancellation via tokio::select!). Disarmed on
        // successful completion.
        let spawned_pgid = child.id();
        let mut kill_guard = KillOnDrop(spawned_pgid);

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.map_err(|err| {
                RalphError::BackendCommandFailed {
                    backend: self.name.clone(),
                    details: format!("failed to write prompt to stdin: {err}"),
                }
            })?;
        }

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: "child stdout pipe unavailable".to_owned(),
            })?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: "child stderr pipe unavailable".to_owned(),
            })?;

        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let activity_notify = Arc::new(Notify::new());
        let stderr_backend = self.name.clone();
        let stderr_activity_notify = activity_notify.clone();
        let stderr_last_activity = last_activity.clone();
        let stderr_log_file: Option<std::fs::File> = log_writer
            .as_ref()
            .and_then(|w| std::fs::OpenOptions::new().append(true).open(w.path()).ok());
        let stderr_handle = tokio::spawn(async move {
            let mut log_file = stderr_log_file;
            let mut captured = Vec::new();
            let mut chunk = BytesMut::with_capacity(4096);
            loop {
                chunk.clear();
                match stderr.read_buf(&mut chunk).await {
                    Ok(0) => return Ok(captured),
                    Ok(n) => {
                        let bytes = &chunk[..n];
                        captured.extend_from_slice(bytes);
                        *stderr_last_activity.lock().await = Instant::now();
                        if let Some(ref mut f) = log_file {
                            use std::io::Write;
                            let _ = f.write_all(bytes).and_then(|_| f.flush());
                        }
                        stderr_activity_notify.notify_one();
                    }
                    Err(err) => {
                        return Err(RalphError::BackendCommandFailed {
                            backend: stderr_backend.clone(),
                            details: format!("failed to read stderr: {err}"),
                        });
                    }
                }
            }
        });

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum WatchdogOutcome {
            Cancelled,
            TimedOut,
        }

        let (watchdog_cancel_tx, mut watchdog_cancel_rx) = oneshot::channel::<()>();
        let watchdog_timeout = self.timeout;
        let watchdog_activity_notify = activity_notify.clone();
        let mut watchdog_handle = tokio::spawn(async move {
            let sleep = tokio::time::sleep(watchdog_timeout);
            tokio::pin!(sleep);

            loop {
                tokio::select! {
                    biased;

                    _ = &mut watchdog_cancel_rx => return WatchdogOutcome::Cancelled,
                    _ = watchdog_activity_notify.notified() => {
                        sleep.as_mut().reset(tokio::time::Instant::now() + watchdog_timeout);
                    }
                    _ = &mut sleep => return WatchdogOutcome::TimedOut,
                }
            }
        });

        enum ExecutionOutcome {
            Completed(Result<(std::process::ExitStatus, Vec<u8>)>),
            TimedOut,
            WatchdogFailed(String),
            Cancelled,
        }

        let mut watchdog_cancel_error: Option<String> = None;
        let execution_outcome = {
            let stdout_activity_notify = activity_notify.clone();
            let stdout_last_activity = last_activity.clone();
            let execution = async {
                let mut captured_stdout = Vec::new();
                let mut chunk = BytesMut::with_capacity(8192);
                loop {
                    chunk.clear();
                    let read = stdout.read_buf(&mut chunk).await.map_err(|err| {
                        RalphError::BackendCommandFailed {
                            backend: self.name.clone(),
                            details: format!("failed to read stdout: {err}"),
                        }
                    })?;
                    if read == 0 {
                        break;
                    }
                    let bytes = chunk.as_ref();
                    captured_stdout.extend_from_slice(bytes);
                    *stdout_last_activity.lock().await = Instant::now();
                    if let Some(writer) = log_writer.as_mut() {
                        writer.write_bytes(bytes);
                    }
                    stdout_activity_notify.notify_one();
                }

                let status =
                    child
                        .wait()
                        .await
                        .map_err(|err| RalphError::BackendCommandFailed {
                            backend: self.name.clone(),
                            details: format!("failed waiting for child process: {err}"),
                        })?;

                Ok::<(std::process::ExitStatus, Vec<u8>), RalphError>((status, captured_stdout))
            };
            tokio::pin!(execution);

            let execution_outcome = tokio::select! {
                biased;

                result = &mut execution => ExecutionOutcome::Completed(result),
                watchdog_result = &mut watchdog_handle => {
                    match watchdog_result {
                        Ok(WatchdogOutcome::TimedOut) => ExecutionOutcome::TimedOut,
                        Ok(WatchdogOutcome::Cancelled) => ExecutionOutcome::WatchdogFailed(
                            "watchdog cancelled before backend execution completed".to_owned(),
                        ),
                        Err(err) => {
                            ExecutionOutcome::WatchdogFailed(format!("watchdog task failed: {err}"))
                        }
                    }
                }
                _ = cancel.cancelled() => ExecutionOutcome::Cancelled,
            };

            // Cancel the watchdog unless it already completed on its own.
            if matches!(
                execution_outcome,
                ExecutionOutcome::Completed(_) | ExecutionOutcome::Cancelled
            ) {
                let _ = watchdog_cancel_tx.send(());
                if let Err(err) = watchdog_handle.await {
                    if !matches!(execution_outcome, ExecutionOutcome::Cancelled) {
                        watchdog_cancel_error =
                            Some(format!("watchdog task failed during cancellation: {err}"));
                    }
                }
            }

            execution_outcome
        };

        // NOTE: kill_guard remains armed here. It is disarmed in each
        // outcome branch below only after the child is confirmed dead or
        // explicitly reaped. This ensures that if cancellation drops this
        // future mid-cleanup, the guard's Drop fires SIGKILL as a fallback.

        if let Some(details) = watchdog_cancel_error {
            // Guard stays armed — drop will kill the child.
            return Err(RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details,
            });
        }

        match execution_outcome {
            ExecutionOutcome::Completed(Ok((status, captured_stdout))) => {
                // Guard stays armed while we drain stderr. If cancellation
                // drops this future mid-drain, KillOnDrop fires SIGKILL on
                // the process group, cleaning up any surviving descendants.
                let stderr_bytes = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.collect_stderr(stderr_handle),
                )
                .await
                {
                    Ok(result) => result?,
                    Err(_elapsed) => {
                        // Stderr drain timed out — descendants may be holding
                        // the pipe open. Kill the process group to unblock.
                        warn!(
                            backend = %self.name,
                            "stderr drain timed out after 5s, killing process group"
                        );
                        self.kill_and_reap_child(&mut child, spawned_pgid).await;
                        Vec::new()
                    }
                };
                if !status.success() {
                    // Backend exited non-zero — kill the process group to
                    // clean up any detached descendants before disarming.
                    self.kill_and_reap_child(&mut child, spawned_pgid).await;
                    kill_guard.disarm();

                    let stderr_text = String::from_utf8_lossy(&stderr_bytes).trim().to_owned();
                    // Some backends (e.g. codex) report errors via stdout JSON
                    // rather than stderr. When stderr is empty, include stdout
                    // so quota/error messages are visible to callers.
                    let details = if stderr_text.is_empty() {
                        String::from_utf8_lossy(&captured_stdout).trim().to_owned()
                    } else {
                        stderr_text
                    };
                    return Err(RalphError::BackendCommandFailed {
                        backend: self.name.clone(),
                        details,
                    });
                }

                // Success path — all async cleanup done, safe to disarm
                // with no further .await between here and return.
                kill_guard.disarm();

                let raw = String::from_utf8_lossy(&captured_stdout).to_string();
                // Normalize structured output (stream-json NDJSON, single-object JSON)
                // to plain text so all callers receive clean content regardless of
                // --output-format mode.
                let normalized = output_normalizer::normalize_output(&raw)
                    .map(|n| n.text)
                    .unwrap_or(raw);
                Ok(normalized)
            }
            ExecutionOutcome::Completed(Err(err)) => {
                self.kill_and_reap_child(&mut child, spawned_pgid).await;
                // Child is now dead — safe to disarm.
                kill_guard.disarm();
                let _ = self.collect_stderr(stderr_handle).await;
                Err(err)
            }
            ExecutionOutcome::TimedOut => {
                self.kill_and_reap_child(&mut child, spawned_pgid).await;
                // Child is now dead — safe to disarm.
                kill_guard.disarm();
                let _ = self.collect_stderr(stderr_handle).await;
                if let Some(writer) = log_writer.as_mut() {
                    writer.write_timeout_footer(&chrono::Utc::now().to_rfc3339());
                }
                let idle_secs = last_activity.lock().await.elapsed().as_secs();
                Err(RalphError::BackendTimeout {
                    backend: self.name.clone(),
                    idle_seconds: idle_secs,
                    timeout_kind: TimeoutKind::Idle,
                })
            }
            ExecutionOutcome::WatchdogFailed(details) => {
                self.kill_and_reap_child(&mut child, spawned_pgid).await;
                // Child is now dead — safe to disarm.
                kill_guard.disarm();
                let _ = self.collect_stderr(stderr_handle).await;
                Err(RalphError::BackendCommandFailed {
                    backend: self.name.clone(),
                    details,
                })
            }
            ExecutionOutcome::Cancelled => {
                // Synchronous cancellation cleanup: kill the backend process
                // group and wait for it to die before returning. This
                // guarantees no backend processes outlive the task.
                self.kill_and_reap_child(&mut child, spawned_pgid).await;
                kill_guard.disarm();
                // Best-effort drain of stderr (bounded to avoid stalling).
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    self.collect_stderr(stderr_handle),
                )
                .await;
                Err(RalphError::Cancelled)
            }
        }
    }

    async fn kill_and_reap_child(
        &self,
        child: &mut tokio::process::Child,
        spawned_pgid: Option<u32>,
    ) {
        // Two-stage termination for the entire process group (child used
        // setsid(), so its PID is the group leader). Send SIGTERM first
        // for cooperative shutdown, then escalate to SIGKILL after a 5s
        // grace period if the process group is still alive.
        //
        // Track group liveness via kill(-pgid, 0) combined with
        // child.try_wait() to reap the leader zombie.  Using only
        // child.wait() would miss descendants that ignore SIGTERM but
        // whose leader exits; using only kill(-pgid, 0) without reaping
        // would see zombies as alive.
        //
        // Use the stored `spawned_pgid` (captured at spawn time) rather than
        // `child.id()` because `child.id()` may return `None` after the
        // child has been reaped via `child.wait()`.
        if let Some(pid) = spawned_pgid.or(child.id()) {
            let Ok(raw_pid) = i32::try_from(pid) else {
                warn!(
                    backend = %self.name,
                    pid = pid,
                    "pid overflows i32, cannot send signal to process group"
                );
                // Fall through to best-effort reap below.
                let _ = child.kill().await;
                let _ = child.wait().await;
                return;
            };
            // Stage 1: SIGTERM the entire process group.
            unsafe {
                libc::kill(-raw_pid, libc::SIGTERM);
            }
            // Poll group liveness over the grace period.
            let grace = Duration::from_secs(KILL_GRACE_SECONDS);
            let deadline = tokio::time::Instant::now() + grace;
            let group_dead = loop {
                // Reap leader zombie via tokio so kill(-pgid, 0) is accurate.
                let _ = child.try_wait();
                let group_alive = unsafe { libc::kill(-raw_pid, 0) } == 0;
                if !group_alive {
                    break true;
                }
                if tokio::time::Instant::now() >= deadline {
                    break false;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            if !group_dead {
                // Stage 2: Grace period expired — hard kill the entire group.
                unsafe {
                    libc::kill(-raw_pid, libc::SIGKILL);
                }
            }
        } else if let Err(err) = child.kill().await {
            if err.kind() != std::io::ErrorKind::InvalidInput {
                warn!(
                    backend = %self.name,
                    error = %err,
                    "failed to kill child process during cleanup"
                );
            }
        }
        // Best-effort reap of the leader process.
        if let Err(err) = child.wait().await {
            warn!(
                backend = %self.name,
                error = %err,
                "failed waiting for child process during cleanup"
            );
        }
    }
}

#[async_trait]
impl Backend for CliBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, prompt: &str) -> Result<String> {
        self.execute_streaming(prompt, None, &CancellationToken::new())
            .await
    }

    async fn execute_with_log(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
    ) -> Result<String> {
        self.execute_streaming(prompt, log_writer, &CancellationToken::new())
            .await
    }

    async fn execute_with_cancel(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
        cancel: &CancellationToken,
    ) -> Result<String> {
        self.execute_streaming(prompt, log_writer, cancel).await
    }

    async fn health_check(&self) -> Result<()> {
        which::which(&self.command).map_err(|_| RalphError::BackendUnavailable {
            backend: self.name.clone(),
        })?;
        Ok(())
    }
}

/// Shared execution context that the orchestrator updates before each backend
/// invocation, allowing TmuxBackend instances to read loop/role info without
/// a change to the Backend trait.
#[derive(Clone, Default)]
pub struct SharedTmuxContext(Arc<Mutex<TmuxExecutionContext>>);

impl SharedTmuxContext {
    pub async fn set(&self, ctx: TmuxExecutionContext) {
        *self.0.lock().await = ctx;
    }

    pub async fn get(&self) -> TmuxExecutionContext {
        self.0.lock().await.clone()
    }
}

pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn Backend>>,
    default_backend: String,
    tmux_context: SharedTmuxContext,
    invocation_context: SharedInvocationContext,
    config: GlobalConfig,
    tmux: BackendRegistryTmuxConfig,
    cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BackendRegistryTmuxConfig {
    pub enabled: bool,
    pub session_name: String,
    pub window_keep_seconds: u64,
}

impl BackendRegistry {
    pub fn new(config: &GlobalConfig, tmux: BackendRegistryTmuxConfig) -> Self {
        let mut backends: HashMap<String, Arc<dyn Backend>> = HashMap::new();
        let shared_ctx = SharedTmuxContext::default();
        let shared_invocation = SharedInvocationContext::default();

        let mut claude_backend = claude::backend_from_config(config, None, None, None);
        claude_backend.invocation_ctx = shared_invocation.clone();
        backends.insert(
            "claude".to_owned(),
            backend_with_optional_tmux(claude_backend, &tmux, shared_ctx.clone()),
        );
        let mut codex_backend = codex::backend_from_config(config, None, None, None);
        codex_backend.invocation_ctx = shared_invocation.clone();
        backends.insert(
            "codex".to_owned(),
            backend_with_optional_tmux(codex_backend, &tmux, shared_ctx.clone()),
        );
        let mut openrouter_backend = openrouter::backend_from_config(config, None, None, None);
        openrouter_backend.invocation_ctx = shared_invocation.clone();
        backends.insert(
            "openrouter".to_owned(),
            backend_with_optional_tmux(openrouter_backend, &tmux, shared_ctx.clone()),
        );

        Self {
            backends,
            default_backend: config.workspace.default_backend.clone(),
            tmux_context: shared_ctx,
            invocation_context: shared_invocation,
            config: config.clone(),
            tmux,
            cwd: None,
        }
    }

    pub fn set_cwd(&mut self, cwd: Option<PathBuf>) {
        self.cwd = cwd;
        // Clear cached backends so they are re-created with the new cwd.
        self.backends.clear();
    }

    /// Set the tmux execution context (loop number, role) for the next backend
    /// invocation. Also propagates session_id to the shared invocation context
    /// so non-tmux CliBackend instances can perform session-aware arg rewriting.
    pub async fn set_tmux_context(&self, ctx: TmuxExecutionContext) {
        // Always propagate invocation context so that json_output_required is
        // honoured even on first invocations without a session_id.
        let invocation = Some(BackendInvocationContext {
            loop_dir: ctx.loop_dir.clone().unwrap_or_default(),
            role: ctx.role.clone().unwrap_or_default(),
            session_id: ctx.session_id.clone(),
            json_output_required: true,
        });
        self.invocation_context.set(invocation).await;
        self.tmux_context.set(ctx).await;
    }

    /// Override only the session id in the current execution context.
    /// Used by parse-retry stages to force resume/fresh behavior per attempt.
    pub async fn override_session_id(&self, session_id: Option<String>) {
        let mut ctx = self.tmux_context.get().await;
        ctx.session_id = session_id;
        self.set_tmux_context(ctx).await;
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Backend>> {
        self.backends.get(name).cloned()
    }

    pub fn get_or_create_for_spec(&mut self, spec: &str) -> Result<Arc<dyn Backend>> {
        self.get_or_create_inner(spec, None)
    }

    pub fn get_or_create_for_role(&mut self, spec: &str, role: &str) -> Result<Arc<dyn Backend>> {
        self.get_or_create_inner(spec, Some(role))
    }

    /// Check whether a backend spec refers to a known, enabled backend.
    /// Returns false for empty strings, unknown backends, or disabled backends.
    pub fn is_backend_available(&self, spec: &str) -> bool {
        if spec.is_empty() {
            return false;
        }
        let Ok(parsed) = parse_backend_spec(spec) else {
            return false;
        };
        self.config
            .backend_config(&parsed.name)
            .is_some_and(|cfg| cfg.enabled != BackendEnabled::Disabled)
    }

    fn get_or_create_inner(&mut self, spec: &str, role: Option<&str>) -> Result<Arc<dyn Backend>> {
        let parsed = parse_backend_spec(spec)?;
        if self
            .config
            .backend_config(&parsed.name)
            .is_some_and(|cfg| cfg.enabled == BackendEnabled::Disabled)
        {
            return Err(RalphError::BackendUnavailable {
                backend: backend_spec_key(&parsed),
            });
        }
        let cache_key = match role {
            Some(r) => format!("{}:{r}", backend_spec_key(&parsed)),
            None => backend_spec_key(&parsed),
        };

        if let Some(backend) = self.backends.get(&cache_key) {
            return Ok(backend.clone());
        }

        let mut cli_backend = self.create_cli_backend_for_spec(&parsed, role)?;
        cli_backend.invocation_ctx = self.invocation_context.clone();
        let backend =
            backend_with_optional_tmux(cli_backend, &self.tmux, self.tmux_context.clone());
        self.backends.insert(cache_key, backend.clone());
        Ok(backend)
    }

    pub fn default_backend(&self) -> &str {
        &self.default_backend
    }

    pub fn opposite(&self, backend: &str) -> Result<&str> {
        let parsed = parse_backend_spec(backend)?;
        let primary = match parsed.name.as_str() {
            "claude" => "codex",
            "codex" | "openrouter" => "claude",
            _ => {
                return Err(RalphError::Validation(format!(
                    "unknown backend for opposite lookup: {backend}"
                )));
            }
        };
        // If the primary opposite is unavailable, try openrouter as substitute
        // for codex (since openrouter provides codex-equivalent models).
        if !self.is_backend_available(primary)
            && primary == "codex"
            && self.is_backend_available("openrouter")
        {
            return Ok("openrouter");
        }
        Ok(primary)
    }

    pub fn planner_for_loop(&self, loop_number: u32, starting_backend: &str) -> Result<String> {
        if loop_number % 2 == 1 {
            return Ok(starting_backend.to_owned());
        }
        Ok(self.opposite(starting_backend)?.to_owned())
    }

    /// Resolve the backend spec for a given role by injecting the configured
    /// role-specific model when the base spec is bare (no explicit model).
    ///
    /// Returns `base_backend` unchanged when:
    /// - it already includes an explicit model (e.g. `claude(opus)`)
    /// - the backend name is unknown
    /// - no role model is configured for the given role
    /// - spec parsing fails
    pub fn resolve_backend_for_role(&self, base_backend: &str, role: &str) -> String {
        let parsed = match parse_backend_spec(base_backend) {
            Ok(p) => p,
            Err(_) => return base_backend.to_owned(),
        };

        // Already has an explicit model — don't override
        if parsed.model.is_some() {
            return base_backend.to_owned();
        }

        let model = self
            .config
            .backend_config(&parsed.name)
            .and_then(|bc| bc.models.for_role(role));

        match model {
            Some(m) => format!("{}({m})", parsed.name),
            None => base_backend.to_owned(),
        }
    }

    pub fn timeout_for_role(&self, backend_spec: &str, role: &str) -> Duration {
        parse_backend_spec(backend_spec)
            .ok()
            .and_then(|parsed| self.config.backend_config(&parsed.name))
            .map(|config| config.timeout_for_role(role))
            .unwrap_or_else(|| Duration::from_secs(7200))
    }

    pub fn assign_feature_backends(
        &self,
        loop_number: u32,
        starting_backend: &str,
        role_overrides: &RoleOverrides,
    ) -> Result<FeatureLoopBackends> {
        let alternating_planner = self.planner_for_loop(loop_number, starting_backend)?;
        let alternating_implementer = self.opposite(&alternating_planner)?.to_owned();
        let alternating_reviewer = alternating_planner.clone();

        let planner = role_overrides
            .planner
            .clone()
            .unwrap_or(alternating_planner);
        let implementer = role_overrides
            .implementer
            .clone()
            .unwrap_or(alternating_implementer);
        let reviewer = role_overrides
            .reviewer
            .clone()
            .unwrap_or(alternating_reviewer);
        let planner = self.resolve_backend_for_role(&planner, "planner");
        let implementer = self.resolve_backend_for_role(&implementer, "implementer");
        let reviewer = self.resolve_backend_for_role(&reviewer, "reviewer");
        let qa = if let Some(qa_override) = role_overrides.qa.as_deref() {
            self.resolve_backend_for_role(qa_override, "qa")
        } else {
            implementer.clone()
        };

        Ok(FeatureLoopBackends {
            planner,
            implementer,
            reviewer,
            qa,
        })
    }

    pub fn assign_completion_backends(
        &self,
        loop_number: u32,
        starting_backend: &str,
        role_overrides: &RoleOverrides,
    ) -> Result<CompletionLoopBackends> {
        let alternating_planner = self.planner_for_loop(loop_number, starting_backend)?;
        let alternating_completer = self.opposite(&alternating_planner)?.to_owned();

        let planner = role_overrides
            .planner
            .clone()
            .unwrap_or(alternating_planner);
        let completer = role_overrides
            .completer
            .clone()
            .unwrap_or(alternating_completer);

        Ok(CompletionLoopBackends::new(
            self.resolve_backend_for_role(&planner, "planner"),
            vec![self.resolve_backend_for_role(&completer, "completer")],
        ))
    }

    /// Resolve the effective completers for a completion panel from the configured
    /// `completion_backends` list. Optional backends (`?backend`) that are unavailable
    /// are skipped with a warning. Required backends that are unavailable cause an error.
    /// Returns the list of resolved backend specs for use as completers.
    pub async fn resolve_completion_panel(
        &mut self,
        completion_backends: &[String],
        min_completers: u32,
    ) -> Result<Vec<String>> {
        let mut effective = Vec::new();

        for spec_str in completion_backends {
            let parsed = parse_backend_spec(spec_str)?;
            let resolved = self.resolve_backend_for_role(spec_str, "completer");
            let available = match self
                .backend_available_for_spec(&resolved, Some("completer"))
                .await
            {
                Ok(v) => v,
                Err(_) if parsed.optional => false,
                Err(e) => return Err(e),
            };

            if !available {
                if parsed.optional {
                    warn!(
                        backend = spec_str,
                        "optional completion backend unavailable, skipping"
                    );
                    continue;
                } else {
                    return Err(RalphError::BackendUnavailable {
                        backend: spec_str.to_owned(),
                    });
                }
            }

            effective.push(resolved);
        }

        if (effective.len() as u32) < min_completers {
            return Err(RalphError::Validation(format!(
                "only {} effective completers available but completion_min_completers requires {}",
                effective.len(),
                min_completers
            )));
        }

        Ok(effective)
    }

    /// Collect model-injected backend specs configured for all roles across
    /// all known backends.
    pub fn backend_role_model_specs(&self) -> Vec<String> {
        let mut specs = BTreeSet::new();
        let roles = [
            "planner",
            "implementer",
            "reviewer",
            "final_reviewer",
            "arbiter",
            "qa",
            "completer",
            "acceptance_qa",
            "reformatter",
        ];

        for (backend_name, models, enabled) in [
            (
                "claude",
                &self.config.backends.claude.models,
                &self.config.backends.claude.enabled,
            ),
            (
                "codex",
                &self.config.backends.codex.models,
                &self.config.backends.codex.enabled,
            ),
            (
                "openrouter",
                &self.config.backends.openrouter.models,
                &self.config.backends.openrouter.enabled,
            ),
        ] {
            if *enabled == BackendEnabled::Disabled {
                continue;
            }
            for role in roles {
                if let Some(model) = models.for_role(role) {
                    specs.insert(format!("{backend_name}({model})"));
                }
            }
        }

        specs.into_iter().collect()
    }

    pub async fn health_check_all(&self) -> Result<()> {
        for (name, enabled_mode) in [
            ("claude", self.config.backends.claude.enabled.clone()),
            ("codex", self.config.backends.codex.enabled.clone()),
            (
                "openrouter",
                self.config.backends.openrouter.enabled.clone(),
            ),
        ] {
            if enabled_mode != BackendEnabled::Enabled {
                continue;
            }
            if let Some(backend) = self.backends.get(name) {
                backend.health_check().await?;
            }
        }
        Ok(())
    }

    pub async fn backend_available_for_spec(
        &mut self,
        backend_spec: &str,
        role: Option<&str>,
    ) -> Result<bool> {
        let parsed = parse_backend_spec(backend_spec)?;
        let Some(config) = self.config.backend_config(&parsed.name) else {
            return Err(RalphError::Validation(format!(
                "unknown backend for spec lookup: {}",
                backend_spec
            )));
        };

        if config.enabled == BackendEnabled::Disabled {
            return Ok(false);
        }

        let backend = match role {
            Some(r) => self.get_or_create_for_role(backend_spec, r)?,
            None => self.get_or_create_for_spec(backend_spec)?,
        };
        match backend.health_check().await {
            Ok(()) => Ok(true),
            Err(RalphError::BackendUnavailable { .. }) => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Obtain a `CliBackend` for the given backend spec string (e.g. "claude(opus)").
    /// Used by the orchestrator to pre-validate session arg rewriting.
    pub fn cli_backend_for_spec(&self, spec_str: &str) -> Result<CliBackend> {
        let parsed = parse_backend_spec(spec_str)?;
        self.create_cli_backend_for_spec(&parsed, None)
    }

    fn create_cli_backend_for_spec(
        &self,
        spec: &BackendSpec,
        role: Option<&str>,
    ) -> Result<CliBackend> {
        if self
            .config
            .backend_config(&spec.name)
            .is_some_and(|cfg| cfg.enabled == BackendEnabled::Disabled)
        {
            return Err(RalphError::BackendUnavailable {
                backend: backend_spec_key(spec),
            });
        }

        let model = spec.model.as_deref();
        match spec.name.as_str() {
            "claude" => Ok(claude::backend_from_config(
                &self.config,
                model,
                role,
                self.cwd.clone(),
            )),
            "codex" => Ok(codex::backend_from_config(
                &self.config,
                model,
                role,
                self.cwd.clone(),
            )),
            "openrouter" => Ok(openrouter::backend_from_config(
                &self.config,
                model,
                role,
                self.cwd.clone(),
            )),
            _ => Err(RalphError::Validation(format!(
                "unknown backend for spec lookup: {}",
                backend_spec_key(spec)
            ))),
        }
    }
}

fn backend_spec_key(spec: &BackendSpec) -> String {
    match spec.model.as_deref() {
        Some(model) => format!("{}({model})", spec.name),
        None => spec.name.clone(),
    }
}

fn backend_with_optional_tmux(
    backend: CliBackend,
    tmux: &BackendRegistryTmuxConfig,
    shared_ctx: SharedTmuxContext,
) -> Arc<dyn Backend> {
    if tmux.enabled {
        Arc::new(TmuxBackend::new(
            backend,
            tmux.session_name.clone(),
            RealTmuxRunner,
            tmux.window_keep_seconds,
            shared_ctx,
        ))
    } else {
        Arc::new(backend)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use tempfile::tempdir;

    use super::{
        parse_backend_spec, Backend, BackendRegistry, BackendRegistryTmuxConfig, BackendSpec,
        CliBackend,
    };
    use crate::config::global::BackendEnabled;
    use crate::config::GlobalConfig;
    use crate::error::{RalphError, TimeoutKind};
    use crate::output_log::LogWriter;

    #[test]
    fn parse_backend_spec_accepts_bare_name() {
        let parsed = parse_backend_spec("claude").expect("bare backend should parse");
        assert_eq!(
            parsed,
            BackendSpec {
                optional: false,
                name: "claude".to_owned(),
                model: None,
            }
        );
    }

    #[test]
    fn parse_backend_spec_accepts_name_with_model() {
        let parsed = parse_backend_spec("claude(opus)").expect("model backend should parse");
        assert_eq!(
            parsed,
            BackendSpec {
                optional: false,
                name: "claude".to_owned(),
                model: Some("opus".to_owned()),
            }
        );
    }

    #[test]
    fn parse_backend_spec_accepts_optional_bare_name() {
        let parsed = parse_backend_spec("?openrouter").expect("optional backend should parse");
        assert_eq!(
            parsed,
            BackendSpec {
                optional: true,
                name: "openrouter".to_owned(),
                model: None,
            }
        );
    }

    #[test]
    fn parse_backend_spec_accepts_optional_name_with_model() {
        let parsed = parse_backend_spec("?openrouter(gpt-5.3-codex-xhigh)")
            .expect("optional modeled backend");
        assert_eq!(
            parsed,
            BackendSpec {
                optional: true,
                name: "openrouter".to_owned(),
                model: Some("gpt-5.3-codex-xhigh".to_owned()),
            }
        );
    }

    #[test]
    fn parse_backend_spec_rejects_empty_name() {
        assert!(parse_backend_spec("(opus)").is_err());
    }

    #[test]
    fn parse_backend_spec_rejects_empty_model() {
        assert!(parse_backend_spec("claude()").is_err());
    }

    #[test]
    fn parse_backend_spec_rejects_missing_closing_paren() {
        assert!(parse_backend_spec("claude(opus").is_err());
    }

    #[test]
    fn parse_backend_spec_rejects_missing_opening_paren() {
        assert!(parse_backend_spec("claudeopus)").is_err());
    }

    #[test]
    fn parse_backend_spec_rejects_missing_name_after_optional_prefix() {
        assert!(parse_backend_spec("?").is_err());
        assert!(parse_backend_spec("??").is_err());
    }

    fn tmux_disabled() -> BackendRegistryTmuxConfig {
        BackendRegistryTmuxConfig {
            enabled: false,
            session_name: "ralph".to_owned(),
            window_keep_seconds: 5,
        }
    }

    #[test]
    fn backend_registry_timeout_for_role_uses_backend_role_override_for_bare_and_modeled_specs() {
        let mut config = GlobalConfig::default();
        config.backends.claude.timeout_seconds = 123;
        config.backends.claude.role_timeouts.planner = Some(45);
        let registry = BackendRegistry::new(&config, tmux_disabled());

        assert_eq!(registry.timeout_for_role("claude", "planner").as_secs(), 45);
        assert_eq!(
            registry
                .timeout_for_role("claude(opus)", "planner")
                .as_secs(),
            45
        );
        assert_eq!(registry.timeout_for_role("claude", "qa").as_secs(), 123);
    }

    #[test]
    fn backend_registry_timeout_for_role_falls_back_to_default_for_unknown_or_invalid_spec() {
        let config = GlobalConfig::default();
        let registry = BackendRegistry::new(&config, tmux_disabled());

        assert_eq!(
            registry
                .timeout_for_role("unknown(opus)", "planner")
                .as_secs(),
            7200
        );
        assert_eq!(
            registry.timeout_for_role("claude(", "planner").as_secs(),
            7200
        );
    }

    #[test]
    fn backend_registry_creates_openrouter_backend_for_modeled_spec() {
        let mut config = GlobalConfig::default();
        config.backends.openrouter.enabled = BackendEnabled::Enabled;
        let mut registry = BackendRegistry::new(&config, tmux_disabled());

        let backend = registry
            .get_or_create_for_spec("openrouter(gpt-5.3-codex-xhigh)")
            .expect("openrouter backend should be creatable from registry");
        assert_eq!(backend.name(), "openrouter(gpt-5.3-codex-xhigh)");
    }

    #[test]
    fn backend_registry_rejects_disabled_backend() {
        let mut config = GlobalConfig::default();
        config.backends.openrouter.enabled = BackendEnabled::Disabled;
        let mut registry = BackendRegistry::new(&config, tmux_disabled());
        let result = registry.get_or_create_for_spec("openrouter");
        assert!(matches!(
            result,
            Err(RalphError::BackendUnavailable { backend }) if backend == "openrouter"
        ));
    }

    fn write_executable_script(
        dir: &std::path::Path,
        name: &str,
        body: &str,
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).expect("write script");
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod script");
        path
    }

    #[tokio::test]
    async fn cli_backend_streaming_preserves_exact_bytes_in_log() {
        let temp = tempdir().expect("tempdir");
        let script_path = write_executable_script(
            temp.path(),
            "emit-bytes.sh",
            r#"#!/bin/sh
printf 'progress 10%%\r'
sleep 0.05
printf 'progress 20%%\rpartial-line'
"#,
        );

        let backend = CliBackend::new(
            "streaming-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(2),
            BTreeMap::new(),
        );

        let mut writer = LogWriter::open(temp.path(), "issue-test", Some(1), "planner");
        let output = Backend::execute_with_log(&backend, "ignored", Some(&mut writer))
            .await
            .expect("backend should succeed");

        assert_eq!(output, "progress 10%\rprogress 20%\rpartial-line");
        let logged = fs::read(writer.path()).expect("read log bytes");
        assert_eq!(logged, b"progress 10%\rprogress 20%\rpartial-line");
    }

    #[tokio::test]
    async fn cli_backend_execute_uses_configured_cwd() {
        let temp = tempdir().expect("tempdir");
        let cwd_dir = temp.path().join("repo-clone");
        fs::create_dir_all(&cwd_dir).expect("create cwd dir");
        let script_path = write_executable_script(
            temp.path(),
            "print-cwd.sh",
            r#"#!/bin/sh
cat >/dev/null
pwd
"#,
        );

        let backend = CliBackend::new(
            "cwd-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(2),
            BTreeMap::new(),
        )
        .with_cwd(Some(cwd_dir.clone()));

        let output = Backend::execute(&backend, "ignored")
            .await
            .expect("backend should succeed");
        let observed = PathBuf::from(output.trim())
            .canonicalize()
            .expect("observed cwd should be canonicalizable");
        let expected = cwd_dir.canonicalize().expect("expected cwd canonical");
        assert_eq!(observed, expected);
    }

    #[tokio::test]
    async fn cli_backend_timeout_kills_and_reaps_child_and_writes_footer() {
        let temp = tempdir().expect("tempdir");
        let pid_file = temp.path().join("child.pid");
        let script_path = write_executable_script(
            temp.path(),
            "hang-after-output.sh",
            &format!(
                r#"#!/bin/sh
echo $$ > "{pid_file}"
printf 'partial-timeout-output'
sleep 0.05
printf '%s' '-after-activity'
sleep 30
"#,
                pid_file = pid_file.display()
            ),
        );

        let backend = CliBackend::new(
            "timeout-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_millis(250),
            BTreeMap::new(),
        );

        let mut writer = LogWriter::open(temp.path(), "issue-test", Some(2), "implementer");
        let start = Instant::now();
        let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
        let elapsed = start.elapsed();

        // Timeout must fire near the configured idle threshold (250ms), not
        // after the full 30s sleep. Allow generous buffer for CI scheduling.
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout should fire near idle threshold, not after full sleep; elapsed={elapsed:?}"
        );
        match result {
            Err(RalphError::BackendTimeout {
                backend,
                timeout_kind,
                ..
            }) => {
                assert_eq!(backend, "timeout-test");
                assert_eq!(timeout_kind, TimeoutKind::Idle);
            }
            other => panic!("expected BackendTimeout, got: {other:?}"),
        }

        let log_content = fs::read_to_string(writer.path()).expect("read log");
        assert!(log_content.contains("partial-timeout-output"));
        assert!(log_content.contains("-after-activity"));
        assert!(log_content.contains("--- timeout ts="));
        assert!(
            elapsed >= Duration::from_millis(280),
            "idle timeout should start after latest activity; elapsed={elapsed:?}"
        );

        let pid_raw = fs::read_to_string(&pid_file).expect("read pid file");
        let pid: i32 = pid_raw.trim().parse().expect("pid should be numeric");

        let kill_rc = unsafe { libc::kill(pid, 0) };
        assert_eq!(kill_rc, -1, "child pid should not be alive");
        let os_err = std::io::Error::last_os_error()
            .raw_os_error()
            .expect("raw os error should exist");
        assert_eq!(os_err, libc::ESRCH, "child pid should be fully reaped");
    }

    fn make_invocation_ctx(session_id: Option<&str>) -> super::BackendInvocationContext {
        super::BackendInvocationContext {
            loop_dir: std::path::PathBuf::from("/tmp/loop"),
            role: "implementer".to_owned(),
            session_id: session_id.map(|s| s.to_owned()),
            json_output_required: true,
        }
    }

    #[test]
    fn effective_args_no_session_adds_json_output() {
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec!["-p".to_owned(), "--flag".to_owned()],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(None); // json_output_required = true
        let args = backend.effective_args(&ctx).unwrap();
        assert_eq!(args, vec!["-p", "--flag", "--output-format", "json"]);
    }

    #[test]
    fn effective_args_no_session_no_json_returns_unchanged() {
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec!["-p".to_owned(), "--flag".to_owned()],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = super::BackendInvocationContext {
            loop_dir: std::path::PathBuf::from("/tmp/loop"),
            role: "implementer".to_owned(),
            session_id: None,
            json_output_required: false,
        };
        let args = backend.effective_args(&ctx).unwrap();
        assert_eq!(args, vec!["-p", "--flag"]);
    }

    #[test]
    fn effective_args_no_session_openrouter_rewrites_output_format_to_stream_json() {
        let backend = CliBackend::new(
            "openrouter",
            "openrouter".to_owned(),
            vec![
                "--output-format".to_owned(),
                "json".to_owned(),
                "--other".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(None);
        let args = backend.effective_args(&ctx).unwrap();
        assert_eq!(
            args,
            vec!["--other", "--output-format", "stream-json"],
            "openrouter first call should force output-format=stream-json"
        );
    }

    #[test]
    fn effective_args_claude_rewrites_for_resume() {
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec![
                "-p".to_owned(),
                "--permission-mode".to_owned(),
                "acceptEdits".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("sess-abc"));
        let args = backend.effective_args(&ctx).unwrap();
        assert!(!args.contains(&"-p".to_owned()), "should remove -p");
        assert!(args.contains(&"--resume".to_owned()));
        assert!(args.contains(&"sess-abc".to_owned()));
        assert!(args.contains(&"--output-format".to_owned()));
        assert!(args.contains(&"json".to_owned()));
    }

    #[test]
    fn effective_args_claude_idempotent() {
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec![
                "-p".to_owned(),
                "--permission-mode".to_owned(),
                "acceptEdits".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("sess-abc"));
        let args1 = backend.effective_args(&ctx).unwrap();

        // Create a new backend with the rewritten args plus -p to simulate re-call
        let backend2 = CliBackend::new(
            "claude",
            "claude".to_owned(),
            {
                let mut a = vec!["-p".to_owned()];
                a.extend(args1.clone());
                a
            },
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let args2 = backend2.effective_args(&ctx).unwrap();
        // Should have same structure
        assert!(args2.contains(&"--resume".to_owned()));
        assert!(args2.contains(&"sess-abc".to_owned()));
    }

    #[test]
    fn effective_args_claude_requires_p_flag() {
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec!["--flag".to_owned()],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("sess-abc"));
        assert!(backend.effective_args(&ctx).is_err());
    }

    #[test]
    fn effective_args_codex_rewrites_for_resume() {
        let backend = CliBackend::new(
            "codex",
            "codex".to_owned(),
            vec![
                "exec".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "-".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("thread-xyz"));
        let args = backend.effective_args(&ctx).unwrap();
        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "resume");
        assert_eq!(args[2], "thread-xyz");
        assert_eq!(args.last().unwrap(), "-");
        assert!(args.contains(&"--json".to_owned()));
    }

    #[test]
    fn effective_args_codex_idempotent() {
        let backend = CliBackend::new(
            "codex",
            "codex".to_owned(),
            vec![
                "exec".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "-".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("thread-xyz"));
        let args1 = backend.effective_args(&ctx).unwrap();

        let backend2 = CliBackend::new(
            "codex",
            "codex".to_owned(),
            args1.clone(),
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let args2 = backend2.effective_args(&ctx).unwrap();
        assert_eq!(args2.last().unwrap(), "-");
        // Should not have multiple --json
        assert_eq!(args2.iter().filter(|a| *a == "--json").count(), 1);
    }

    #[test]
    fn effective_args_codex_requires_trailing_dash() {
        let backend = CliBackend::new(
            "codex",
            "codex".to_owned(),
            vec!["exec".to_owned()],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("thread-xyz"));
        assert!(backend.effective_args(&ctx).is_err());
    }

    #[test]
    fn effective_args_codex_requires_exec_in_base_form() {
        // Args with trailing `-` but no `exec` should be rejected
        let backend = CliBackend::new(
            "codex",
            "codex".to_owned(),
            vec!["--flag".to_owned(), "-".to_owned()],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("thread-xyz"));
        let result = backend.effective_args(&ctx);
        assert!(result.is_err(), "codex without 'exec' should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("exec"),
            "error should mention 'exec': {err_msg}"
        );
    }

    // --- Strengthened arg-rewrite tests with full token sequence assertions ---

    #[test]
    fn effective_args_claude_produces_exact_token_sequence() {
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec![
                "-p".to_owned(),
                "--permission-mode".to_owned(),
                "acceptEdits".to_owned(),
                "--model".to_owned(),
                "opus".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("sess-123"));
        let args = backend.effective_args(&ctx).unwrap();
        // -p must be removed; no duplicate --resume or --output-format
        assert!(!args.contains(&"-p".to_owned()), "-p must be removed");
        assert_eq!(
            args.iter().filter(|a| *a == "--resume").count(),
            1,
            "exactly one --resume"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "--output-format").count(),
            1,
            "exactly one --output-format"
        );
        // Verify exact position: --resume must be followed by session id
        let resume_idx = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[resume_idx + 1], "sess-123");
        // --output-format must be followed by "json"
        let fmt_idx = args.iter().position(|a| a == "--output-format").unwrap();
        assert_eq!(args[fmt_idx + 1], "json");
        // Original flags preserved
        assert!(args.contains(&"--permission-mode".to_owned()));
        assert!(args.contains(&"acceptEdits".to_owned()));
    }

    #[test]
    fn effective_args_codex_produces_exact_token_sequence() {
        let backend = CliBackend::new(
            "codex",
            "codex".to_owned(),
            vec![
                "exec".to_owned(),
                "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                "--model".to_owned(),
                "gpt-5.3".to_owned(),
                "-".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("thread-abc"));
        let args = backend.effective_args(&ctx).unwrap();
        // Must be: exec resume <id> [flags...] --json -
        assert_eq!(args[0], "exec", "first token must be 'exec'");
        assert_eq!(args[1], "resume", "second token must be 'resume'");
        assert_eq!(args[2], "thread-abc", "third token must be session id");
        assert_eq!(args.last().unwrap(), "-", "last token must be '-'");
        assert_eq!(
            args.iter().filter(|a| *a == "--json").count(),
            1,
            "exactly one --json"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "exec").count(),
            1,
            "exactly one 'exec'"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "resume").count(),
            1,
            "exactly one 'resume'"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "-").count(),
            1,
            "exactly one '-' (trailing)"
        );
        // Original flags preserved
        assert!(args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_owned()));
        assert!(args.contains(&"--model".to_owned()));
        assert!(args.contains(&"gpt-5.3".to_owned()));
    }

    #[test]
    fn effective_args_claude_with_existing_resume_replaces_cleanly() {
        // If base args already contain --resume, it should be replaced
        let backend = CliBackend::new(
            "claude",
            "claude".to_owned(),
            vec![
                "-p".to_owned(),
                "--resume".to_owned(),
                "old-session".to_owned(),
                "--output-format".to_owned(),
                "text".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("new-session"));
        let args = backend.effective_args(&ctx).unwrap();
        assert!(
            !args.contains(&"old-session".to_owned()),
            "old session id must be replaced"
        );
        assert!(
            !args.contains(&"text".to_owned()),
            "old output-format value must be replaced"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "--resume").count(),
            1,
            "exactly one --resume after replacement"
        );
        let resume_idx = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[resume_idx + 1], "new-session");
    }

    #[test]
    fn effective_args_codex_with_existing_resume_replaces_cleanly() {
        // If base args already contain exec resume <old-id>, replace id
        let backend = CliBackend::new(
            "codex",
            "codex".to_owned(),
            vec![
                "exec".to_owned(),
                "resume".to_owned(),
                "old-thread".to_owned(),
                "--json".to_owned(),
                "-".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("new-thread"));
        let args = backend.effective_args(&ctx).unwrap();
        assert!(
            !args.contains(&"old-thread".to_owned()),
            "old session must be replaced"
        );
        assert_eq!(args[2], "new-thread", "new session id in correct position");
        assert_eq!(
            args.iter().filter(|a| *a == "--json").count(),
            1,
            "exactly one --json"
        );
    }

    #[test]
    fn effective_args_openrouter_rewrites_for_resume_and_preserves_other_flags() {
        let backend = CliBackend::new(
            "openrouter",
            "openrouter".to_owned(),
            vec![
                "run".to_owned(),
                "--resume".to_owned(),
                "--name".to_owned(),
                "old-name".to_owned(),
                "--output-format".to_owned(),
                "json".to_owned(),
                "--other".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("new-session"));
        let args = backend.effective_args(&ctx).unwrap();
        assert!(
            args.contains(&"run".to_owned()),
            "openrouter should keep base args"
        );
        assert!(
            args.contains(&"--other".to_owned()),
            "other args should be kept"
        );
        assert!(
            !args.contains(&"old-name".to_owned()),
            "old --name value must be replaced"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "--resume").count(),
            1,
            "exactly one --resume"
        );
        assert_eq!(
            args.iter().filter(|a| *a == "--name").count(),
            1,
            "exactly one --name"
        );
        let name_idx = args.iter().position(|a| a == "--name").unwrap();
        assert_eq!(args[name_idx + 1], "new-session");
        assert_eq!(
            args.iter().filter(|a| *a == "--output-format").count(),
            1,
            "exactly one --output-format"
        );
        let fmt_idx = args.iter().position(|a| a == "--output-format").unwrap();
        assert_eq!(args[fmt_idx + 1], "stream-json");
    }

    #[test]
    fn effective_args_openrouter_resume_rewrite_is_idempotent() {
        let backend = CliBackend::new(
            "openrouter",
            "openrouter".to_owned(),
            vec![
                "run".to_owned(),
                "--output-format".to_owned(),
                "json".to_owned(),
            ],
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let ctx = make_invocation_ctx(Some("sess-1"));
        let args1 = backend.effective_args(&ctx).unwrap();
        let backend2 = CliBackend::new(
            "openrouter",
            "openrouter".to_owned(),
            args1.clone(),
            Duration::from_secs(10),
            BTreeMap::new(),
        );
        let args2 = backend2.effective_args(&ctx).unwrap();
        assert_eq!(args1, args2);
    }

    /// Active streaming beyond timeout_seconds without timeout: the process emits
    /// output at intervals shorter than timeout_seconds, with total runtime exceeding
    /// timeout_seconds. Inactivity timeout must NOT fire.
    #[tokio::test]
    async fn cli_backend_active_stream_does_not_timeout() {
        let temp = tempdir().expect("tempdir");
        let script_path = write_executable_script(
            temp.path(),
            "active-stream.sh",
            r#"#!/bin/sh
# Emit output every 50ms for 400ms total (total > timeout of 200ms)
for i in 1 2 3 4 5 6 7 8; do
  printf "chunk-%d\n" "$i"
  sleep 0.05
done
"#,
        );

        let backend = CliBackend::new(
            "active-stream-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_millis(200), // total runtime ~400ms > 200ms timeout
            BTreeMap::new(),
        );

        let result = Backend::execute(&backend, "ignored").await;
        assert!(
            result.is_ok(),
            "active streaming should not timeout: {result:?}"
        );
        let output = result.unwrap();
        assert!(
            output.contains("chunk-8"),
            "should see all chunks: {output}"
        );
    }

    /// Hanging after partial output: process emits some output then stalls beyond
    /// timeout_seconds. Must timeout with Idle kind and preserve partial output.
    #[tokio::test]
    async fn cli_backend_stall_after_partial_output_times_out_idle() {
        let temp = tempdir().expect("tempdir");
        let script_path = write_executable_script(
            temp.path(),
            "stall-after-partial.sh",
            r#"#!/bin/sh
printf 'partial-data'
sleep 30
"#,
        );

        let backend = CliBackend::new(
            "stall-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_millis(200),
            BTreeMap::new(),
        );

        let mut writer = LogWriter::open(temp.path(), "issue-test", Some(3), "planner");
        let start = Instant::now();
        let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
        let elapsed = start.elapsed();

        // Timeout must fire near the configured idle threshold (200ms), not
        // after the full 30s sleep. Allow generous buffer for CI scheduling.
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout should fire near idle threshold, not after full sleep; elapsed={elapsed:?}"
        );

        match result {
            Err(RalphError::BackendTimeout {
                backend,
                timeout_kind,
                ..
            }) => {
                assert_eq!(backend, "stall-test");
                assert_eq!(timeout_kind, TimeoutKind::Idle);
            }
            other => panic!("expected BackendTimeout with Idle kind, got: {other:?}"),
        }

        let log_content = fs::read_to_string(writer.path()).expect("read log");
        assert!(
            log_content.contains("partial-data"),
            "partial output should be preserved in log: {log_content}"
        );
        assert!(
            log_content.contains("--- timeout ts="),
            "timeout footer should be written: {log_content}"
        );
    }

    #[tokio::test]
    async fn cli_backend_idle_timeout_resets_on_activity() {
        let temp = tempdir().expect("tempdir");
        let script_path = write_executable_script(
            temp.path(),
            "slow-active.sh",
            r#"#!/bin/sh
i=1
while [ "$i" -le 6 ]; do
  printf 'chunk-%s\n' "$i"
  sleep 0.12
  i=$((i + 1))
done
"#,
        );

        let backend = CliBackend::new(
            "idle-reset-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_millis(250),
            BTreeMap::new(),
        );

        let mut writer = LogWriter::open(temp.path(), "issue-test", Some(3), "planner");
        let start = Instant::now();
        let output = Backend::execute_with_log(&backend, "ignored", Some(&mut writer))
            .await
            .expect("backend should not time out while active");
        let elapsed = start.elapsed();

        assert!(output.contains("chunk-1"));
        assert!(output.contains("chunk-6"));
        assert!(
            elapsed >= Duration::from_millis(600),
            "execution should run past nominal timeout while still active; elapsed={elapsed:?}"
        );

        let logged = fs::read_to_string(writer.path()).expect("read log");
        assert!(logged.contains("chunk-1"));
        assert!(logged.contains("chunk-6"));
        assert!(!logged.contains("--- timeout ts="));
    }

    #[test]
    fn is_backend_available_returns_true_for_enabled_backend() {
        let config = GlobalConfig::default();
        let registry = BackendRegistry::new(&config, tmux_disabled());
        assert!(registry.is_backend_available("claude"));
        assert!(registry.is_backend_available("claude(opus)"));
    }

    #[test]
    fn is_backend_available_returns_false_for_disabled_backend() {
        let mut config = GlobalConfig::default();
        config.backends.openrouter.enabled = BackendEnabled::Disabled;
        let registry = BackendRegistry::new(&config, tmux_disabled());
        assert!(!registry.is_backend_available("openrouter"));
        assert!(!registry.is_backend_available("openrouter(gpt-5.3-codex-xhigh)"));
    }

    #[test]
    fn is_backend_available_returns_false_for_empty_string() {
        let config = GlobalConfig::default();
        let registry = BackendRegistry::new(&config, tmux_disabled());
        assert!(!registry.is_backend_available(""));
    }

    #[test]
    fn is_backend_available_returns_false_for_unknown_backend() {
        let config = GlobalConfig::default();
        let registry = BackendRegistry::new(&config, tmux_disabled());
        assert!(!registry.is_backend_available("unknown"));
        assert!(!registry.is_backend_available("foobar(model)"));
    }

    #[test]
    fn opposite_returns_codex_for_claude_when_codex_enabled() {
        let config = GlobalConfig::default();
        let registry = BackendRegistry::new(&config, tmux_disabled());
        assert_eq!(registry.opposite("claude").unwrap(), "codex");
        assert_eq!(registry.opposite("codex").unwrap(), "claude");
        assert_eq!(registry.opposite("openrouter").unwrap(), "claude");
    }

    #[test]
    fn opposite_falls_back_to_openrouter_when_codex_disabled() {
        let mut config = GlobalConfig::default();
        config.backends.codex.enabled = BackendEnabled::Disabled;
        config.backends.openrouter.enabled = BackendEnabled::Auto;
        let registry = BackendRegistry::new(&config, tmux_disabled());
        // claude's opposite is normally codex, but codex is disabled
        // so it should fall back to openrouter.
        assert_eq!(registry.opposite("claude").unwrap(), "openrouter");
        // codex/openrouter opposite is still claude.
        assert_eq!(registry.opposite("codex").unwrap(), "claude");
        assert_eq!(registry.opposite("openrouter").unwrap(), "claude");
    }

    #[test]
    fn assign_feature_backends_uses_openrouter_when_codex_disabled() {
        let mut config = GlobalConfig::default();
        config.backends.codex.enabled = BackendEnabled::Disabled;
        config.backends.openrouter.enabled = BackendEnabled::Auto;
        let registry = BackendRegistry::new(&config, tmux_disabled());
        let no_overrides = super::RoleOverrides {
            planner: None,
            implementer: None,
            reviewer: None,
            qa: None,
            completer: None,
        };

        // Loop 5 (odd): planner=claude, implementer=opposite(claude)=openrouter (codex disabled)
        let backends = registry
            .assign_feature_backends(5, "claude", &no_overrides)
            .expect("should assign backends");
        assert_eq!(backends.planner, "claude(opus)");
        assert!(
            backends.implementer == "openrouter" || backends.implementer.starts_with("openrouter("),
            "implementer should be openrouter, got: {}",
            backends.implementer
        );

        // Loop 6 (even): planner=opposite(claude)=openrouter, implementer=opposite(openrouter)=claude
        let backends = registry
            .assign_feature_backends(6, "claude", &no_overrides)
            .expect("should assign backends");
        assert!(
            backends.planner == "openrouter" || backends.planner.starts_with("openrouter("),
            "planner should be openrouter, got: {}",
            backends.planner
        );
        assert_eq!(backends.implementer, "claude(opus)");
    }

    #[test]
    fn is_backend_available_with_recalculation_workflow() {
        // Simulate the orchestrator's fallback logic:
        // stored backend is empty (missing frontmatter), recalculate from cycle.
        let config = GlobalConfig::default();
        let registry = BackendRegistry::new(&config, tmux_disabled());
        let no_overrides = super::RoleOverrides {
            planner: None,
            implementer: None,
            reviewer: None,
            qa: None,
            completer: None,
        };

        let stored_backend = ""; // empty from missing frontmatter
        assert!(!registry.is_backend_available(stored_backend));

        // Recalculate for loop 5
        let recalc = registry
            .assign_feature_backends(5, "claude", &no_overrides)
            .expect("should recalculate");
        assert!(registry.is_backend_available(&recalc.implementer));
    }

    // -----------------------------------------------------------------------
    // Environment sanitization — CLAUDECODE stripped from backend subprocess
    // -----------------------------------------------------------------------

    /// Mutex that guards process-global env mutations in tests so parallel
    /// test execution doesn't race on `set_var`/`remove_var`.
    fn env_test_mutex() -> &'static std::sync::Mutex<()> {
        static MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        MUTEX.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// RAII guard that restores an env var to its previous state on drop.
    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn cli_backend_strips_claudecode_from_subprocess_env() {
        let _env_lock = env_test_mutex().lock().expect("env mutex");

        let temp = tempdir().expect("tempdir");
        // Script consumes stdin first to avoid broken-pipe timing issues,
        // then prints whether CLAUDECODE was visible.
        let script_path = write_executable_script(
            temp.path(),
            "env-check.sh",
            r#"#!/bin/sh
cat >/dev/null
if [ -n "$CLAUDECODE" ]; then
    echo "LEAKED:$CLAUDECODE"
else
    echo "SANITIZED"
fi
"#,
        );

        // Set CLAUDECODE in process env; EnvGuard restores on drop.
        let _guard = EnvGuard::set("CLAUDECODE", "should-be-stripped");

        let backend = CliBackend::new(
            "env-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(5),
            std::collections::BTreeMap::new(),
        );

        let output = Backend::execute_with_log(&backend, "ignored", None)
            .await
            .expect("backend should succeed");

        assert!(
            output.contains("SANITIZED"),
            "CLAUDECODE should be stripped from backend subprocess environment, got: {output}"
        );
        assert!(
            !output.contains("LEAKED"),
            "CLAUDECODE was leaked to backend subprocess, got: {output}"
        );
    }

    /// Validates the two-stage termination behavior (SIGTERM → 5s → SIGKILL)
    /// in `kill_and_reap_child`. A stubborn subprocess traps SIGTERM and
    /// ignores it, forcing the code to escalate to SIGKILL after the grace
    /// period.
    #[tokio::test]
    async fn kill_and_reap_child_sends_sigterm_then_sigkill_after_grace() {
        let temp = tempdir().expect("tempdir");
        let sigterm_marker = temp.path().join("got-sigterm");
        // Script that traps SIGTERM (writes a marker file) but refuses to exit,
        // forcing the caller to escalate to SIGKILL.
        let script_path = write_executable_script(
            temp.path(),
            "stubborn.sh",
            &format!(
                r#"#!/bin/sh
trap 'echo yes > "{marker}"' TERM
cat >/dev/null
# Busy-wait forever after stdin closes — only SIGKILL can stop us.
while true; do sleep 0.1; done
"#,
                marker = sigterm_marker.display()
            ),
        );

        let backend = CliBackend::new(
            "sigterm-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(30), // large timeout — we kill manually
            BTreeMap::new(),
        );

        // Spawn the child the same way execute_streaming does.
        let mut cmd = tokio::process::Command::new(&backend.command);
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn stubborn child");
        let pgid = child.id();

        // Close stdin so the script reaches the busy-wait loop.
        drop(child.stdin.take());
        // Brief wait for the trap to be installed.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let start = Instant::now();
        backend.kill_and_reap_child(&mut child, pgid).await;
        let elapsed = start.elapsed();

        // The SIGTERM marker file must exist — proves SIGTERM was sent first.
        assert!(
            sigterm_marker.exists(),
            "SIGTERM marker file should exist — kill_and_reap_child must send SIGTERM before SIGKILL"
        );

        // The total time should be at least ~5s (grace period) because the
        // stubborn process ignores SIGTERM and only dies to SIGKILL.
        assert!(
            elapsed >= Duration::from_secs(4),
            "expected at least ~5s grace period before SIGKILL, got {elapsed:?}"
        );
        // But not excessively long — should be near 5s, not 30s.
        assert!(
            elapsed < Duration::from_secs(10),
            "kill_and_reap_child took too long ({elapsed:?}), should complete near the 5s grace period"
        );
    }

    /// Regression test: parent exits on SIGTERM but a child descendant ignores
    /// it.  `kill_and_reap_child` must send SIGKILL to the process group when
    /// descendants survive the leader, ensuring no orphaned processes.
    #[tokio::test]
    async fn kill_and_reap_child_kills_descendants_that_survive_leader() {
        let temp = tempdir().expect("tempdir");
        let child_pid_file = temp.path().join("child-pid");
        // Parent spawns a background child that ignores SIGTERM, then the
        // parent itself exits immediately on SIGTERM.
        let script_path = write_executable_script(
            temp.path(),
            "parent-exits-child-stays.sh",
            &format!(
                r#"#!/bin/sh
# Child: ignore SIGTERM and busy-wait forever
(
    trap '' TERM
    echo $$ > "{child_pid}"
    while true; do sleep 0.1; done
) &
# Parent: exit gracefully on SIGTERM
trap 'exit 0' TERM
cat >/dev/null
while true; do sleep 0.1; done
"#,
                child_pid = child_pid_file.display()
            ),
        );

        let backend = CliBackend::new(
            "descendant-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(30),
            BTreeMap::new(),
        );

        let mut cmd = tokio::process::Command::new(&backend.command);
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn parent");
        let pgid = child.id();

        // Close stdin so the parent reaches the busy-wait loop.
        drop(child.stdin.take());
        // Wait for the child process to write its PID.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let child_pid_str =
            std::fs::read_to_string(&child_pid_file).expect("child pid file should exist");
        let descendant_pid: i32 = child_pid_str
            .trim()
            .parse()
            .expect("child pid should be a valid number");

        // Verify the descendant is alive before cleanup.
        assert_eq!(
            unsafe { libc::kill(descendant_pid, 0) },
            0,
            "descendant process should be alive before cleanup"
        );

        let start = Instant::now();
        backend.kill_and_reap_child(&mut child, pgid).await;
        let elapsed = start.elapsed();

        // The descendant must be dead after kill_and_reap_child.
        let descendant_alive = unsafe { libc::kill(descendant_pid, 0) } == 0;
        assert!(
            !descendant_alive,
            "descendant process {descendant_pid} should be dead after kill_and_reap_child"
        );

        // Should complete within the grace period + margin.  The leader exits
        // immediately on SIGTERM, but the group stays alive until SIGKILL
        // escalation after ~5s.
        assert!(
            elapsed >= Duration::from_secs(4),
            "expected group-level grace period before SIGKILL, got {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "kill_and_reap_child took too long ({elapsed:?})"
        );
    }

    /// Regression: when a backend exits non-zero after spawning a detached
    /// descendant that closes stdio quickly, the descendant process group must
    /// still be killed.  Previously `kill_guard.disarm()` was called before
    /// checking `status.success()`, so the guard couldn't clean up.
    #[tokio::test]
    async fn nonzero_exit_with_detached_child_kills_process_group() {
        let temp = tempdir().expect("tempdir");
        let child_pid_file = temp.path().join("detached-child-pid");
        // Backend script: spawns a detached child that writes its PID and
        // busy-waits forever, then the parent immediately exits 1.
        let script_path = write_executable_script(
            temp.path(),
            "nonzero-detached.sh",
            &format!(
                r#"#!/bin/sh
# Read and discard stdin to satisfy the protocol
cat >/dev/null &
# Spawn a detached child that outlives the parent
(
    echo $$ > "{child_pid}"
    trap '' TERM
    while true; do sleep 0.1; done
) &
# Brief wait for the child to write its PID
sleep 0.2
exit 1
"#,
                child_pid = child_pid_file.display()
            ),
        );

        let backend = CliBackend::new(
            "nonzero-detached-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(10),
            BTreeMap::new(),
        );

        let result = Backend::execute_with_log(&backend, "ignored", None).await;
        assert!(result.is_err(), "backend should fail with non-zero exit");

        // Give the child a moment to write its PID if it hasn't yet
        tokio::time::sleep(Duration::from_millis(300)).await;

        if child_pid_file.exists() {
            let pid_str = std::fs::read_to_string(&child_pid_file).expect("read child pid file");
            if let Ok(descendant_pid) = pid_str.trim().parse::<i32>() {
                // The descendant must be dead — killed via process group
                // cleanup on the non-zero exit path.
                let alive = unsafe { libc::kill(descendant_pid, 0) } == 0;
                assert!(
                    !alive,
                    "detached descendant {descendant_pid} should be dead after non-zero backend exit"
                );
            }
        }
    }

    /// Verifies that `kill_and_reap_child` uses the `spawned_pgid` parameter
    /// to perform group-level cleanup even when `child.id()` returns `None`
    /// (i.e. after the child has already been reaped via `wait()`).
    ///
    /// Spawns a stubborn process (traps SIGTERM), waits for it to exit
    /// naturally (via stdin EOF), then calls `kill_and_reap_child` with
    /// `child.id()` as `None` but a valid `spawned_pgid`.  The function
    /// must not panic and must attempt group-level signal delivery.
    #[tokio::test]
    async fn kill_and_reap_child_uses_stored_pgid_after_leader_exit() {
        let temp = tempdir().expect("tempdir");
        // Simple script: consume stdin, then exit 0.
        let script_path = write_executable_script(
            temp.path(),
            "exits-on-stdin-close.sh",
            "#!/bin/sh\ncat >/dev/null\nexit 0\n",
        );

        let backend = CliBackend::new(
            "stored-pgid-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_secs(30),
            BTreeMap::new(),
        );

        let mut cmd = tokio::process::Command::new(&backend.command);
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn");
        let stored_pgid = child.id();
        assert!(stored_pgid.is_some(), "child should have a PID at spawn");

        // Close stdin → child exits naturally.
        drop(child.stdin.take());
        let _ = child.wait().await;

        // After wait(), child.id() should be None.
        assert!(
            child.id().is_none(),
            "child.id() should be None after wait()"
        );

        // Call kill_and_reap_child with stored_pgid but child.id() == None.
        // This must not panic and should use the stored PGID for group signal.
        backend.kill_and_reap_child(&mut child, stored_pgid).await;

        // Also verify the fallback path: when spawned_pgid is also None,
        // it falls through to the `else` branch safely.
        backend.kill_and_reap_child(&mut child, None).await;
    }
}
