pub mod claude;
pub mod codex;
pub mod mock;
pub mod output_normalizer;
pub mod tmux;
pub mod tmux_backend;

pub use mock::MockBackend;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::config::GlobalConfig;
use crate::error::{RalphError, TimeoutKind};
use crate::output_log::LogWriter;
use crate::project::state::{CompletionLoopBackends, FeatureLoopBackends};
use crate::util::time::now_timestamp_yyyymmddhhmmss;
use crate::Result;

use self::tmux::RealTmuxRunner;
use self::tmux_backend::{TmuxBackend, TmuxExecutionContext};

pub(crate) static CLI_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, prompt: &str) -> Result<String>;
    async fn execute_with_log(
        &self,
        prompt: &str,
        mut log_writer: Option<&mut LogWriter>,
    ) -> Result<String> {
        let _ = log_writer.as_deref_mut();
        self.execute(prompt).await
    }
    async fn health_check(&self) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSpec {
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
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(RalphError::Validation(
            "backend spec must not be empty".to_owned(),
        ));
    }

    let open_count = spec.matches('(').count();
    let close_count = spec.matches(')').count();

    if open_count == 0 && close_count == 0 {
        return Ok(BackendSpec {
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
        name: name.to_owned(),
        model: Some(model.to_owned()),
    })
}

fn sanitize_role_for_filename(role: &str) -> String {
    let sanitized = role
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

fn build_cli_output_filename(role: &str) -> String {
    let timestamp = now_timestamp_yyyymmddhhmmss();
    let counter = CLI_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp}-agent-output-{role}-{counter}.log")
}

pub(crate) async fn persist_cli_output(
    loop_dir: Option<&Path>,
    backend_name: &str,
    role: Option<&str>,
    exit_status: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) {
    let Some(loop_dir) = loop_dir else {
        debug!(
            backend = backend_name,
            role = ?role,
            "skipping backend output artifact: loop_dir is not set"
        );
        return;
    };

    let role_value = role
        .map(str::trim)
        .filter(|role| !role.is_empty())
        .unwrap_or("unknown");
    let file_role = sanitize_role_for_filename(role_value);
    let filename = build_cli_output_filename(&file_role);
    let artifact_path = loop_dir.join(filename);
    let exit_status_text = exit_status
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_owned());

    let content = format!(
        "=== Backend Output Log ===\nbackend: {backend_name}\nrole: {role_value}\nexit_status: {exit_status_text}\n\n=== STDOUT ===\n{}\n\n=== STDERR ===\n{}\n",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );

    if let Err(err) = tokio::fs::create_dir_all(loop_dir).await {
        warn!(
            backend = backend_name,
            role = role_value,
            path = %loop_dir.display(),
            error = %err,
            "failed to prepare loop directory for backend output artifact"
        );
        return;
    }

    match tokio::fs::write(&artifact_path, content).await {
        Ok(()) => {
            info!(
                path = %artifact_path.display(),
                backend = backend_name,
                role = role_value,
                "wrote backend output artifact"
            );
        }
        Err(err) => {
            warn!(
                backend = backend_name,
                role = role_value,
                path = %artifact_path.display(),
                error = %err,
                "failed to write backend output artifact"
            );
        }
    }
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
            invocation_ctx: SharedInvocationContext::default(),
        }
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
                n if n.starts_with("claude") || n == "claude" => {
                    self.effective_args_claude(id)
                }
                n if n.starts_with("codex") || n == "codex" => {
                    self.effective_args_codex(id)
                }
                _ => Ok(self.args.clone()),
            },
            None if ctx.json_output_required => {
                self.ensure_json_output_args()
            }
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
                if !args.iter().any(|a| a == "--output-format" || a.starts_with("--output-format=")) {
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
                        if i < args.len() && args[i] != "-" && args[i] != "--json" && !args[i].starts_with("--") {
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
    ) -> Result<String> {
        // Compute effective args: if an invocation context is set, use
        // effective_args() for session-aware arg rewriting and/or JSON output
        // flags. On failure, fall back to base args.
        let effective_args = {
            let ctx_opt = self.invocation_ctx.get().await;
            match ctx_opt {
                Some(ref ctx) => {
                    match self.effective_args(ctx) {
                        Ok(args) => args,
                        Err(e) => {
                            debug!(
                                backend = self.name,
                                error = %e,
                                "effective_args rewrite failed in CliBackend, using base args"
                            );
                            self.args.clone()
                        }
                    }
                }
                None => self.args.clone(),
            }
        };

        let resolved_command = self.resolved_command_path();
        let mut cmd = Command::new(&resolved_command);
        cmd.args(&effective_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .envs(self.env.clone());

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

        // Shared idle-tracking state: last_activity is updated by both stdout
        // and stderr readers; the watchdog checks it periodically.
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let timed_out = Arc::new(AtomicBool::new(false));

        let stderr_backend = self.name.clone();
        let stderr_log_file: Option<std::fs::File> = log_writer.as_ref().and_then(|w| {
            std::fs::OpenOptions::new().append(true).open(w.path()).ok()
        });
        let stderr_last_activity = last_activity.clone();
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

        // Inactivity watchdog: checks every ~1s whether idle duration >= timeout.
        // Cancels via `timed_out` flag + killing the child process group.
        let watchdog_timeout = self.timeout;
        let watchdog_last_activity = last_activity.clone();
        let watchdog_timed_out = timed_out.clone();
        let watchdog_child_id = child.id();
        let watchdog_handle = tokio::spawn(async move {
            let poll_interval = Duration::from_secs(1).min(watchdog_timeout / 2).max(Duration::from_millis(10));
            loop {
                tokio::time::sleep(poll_interval).await;
                let idle_elapsed = watchdog_last_activity.lock().await.elapsed();
                if idle_elapsed >= watchdog_timeout {
                    watchdog_timed_out.store(true, Ordering::SeqCst);
                    // Kill the process group to unblock stdout/stderr readers
                    if let Some(pid) = watchdog_child_id {
                        unsafe {
                            libc::kill(-(pid as i32), libc::SIGKILL);
                        }
                    }
                    return idle_elapsed;
                }
            }
        });

        // Read stdout with idle tracking
        let stdout_last_activity = last_activity.clone();
        let stdout_result: std::result::Result<
            std::result::Result<(std::process::ExitStatus, Vec<u8>), RalphError>,
            tokio::task::JoinError,
        > = {
            let backend_name = self.name.clone();
            let handle = tokio::spawn(async move {
                let mut captured_stdout = Vec::new();
                let mut chunk = BytesMut::with_capacity(8192);
                loop {
                    chunk.clear();
                    match stdout.read_buf(&mut chunk).await {
                        Ok(0) => break,
                        Ok(read) => {
                            let bytes = &chunk[..read];
                            captured_stdout.extend_from_slice(bytes);
                            *stdout_last_activity.lock().await = Instant::now();
                        }
                        Err(err) => {
                            return Err(RalphError::BackendCommandFailed {
                                backend: backend_name,
                                details: format!("failed to read stdout: {err}"),
                            });
                        }
                    }
                }

                let status = child.wait().await.map_err(|err| {
                    RalphError::BackendCommandFailed {
                        backend: backend_name,
                        details: format!("failed waiting for child process: {err}"),
                    }
                })?;

                Ok((status, captured_stdout))
            });
            handle.await
        };

        // Cancel the watchdog — normal completion arrived first
        watchdog_handle.abort();

        // Write stdout to log (we collected in the spawned task, now replay to log_writer)
        let was_timeout = timed_out.load(Ordering::SeqCst);

        match stdout_result {
            Ok(Ok((status, captured_stdout))) if !was_timeout => {
                // Write captured stdout to log
                if let Some(writer) = log_writer.as_deref_mut() {
                    writer.write_bytes(&captured_stdout);
                }
                let stderr_bytes = self.collect_stderr(stderr_handle).await?;

                if !status.success() {
                    return Err(RalphError::BackendCommandFailed {
                        backend: self.name.clone(),
                        details: String::from_utf8_lossy(&stderr_bytes).trim().to_owned(),
                    });
                }

                Ok(String::from_utf8_lossy(&captured_stdout).to_string())
            }
            Ok(Err(err)) if !was_timeout => {
                let _ = self.collect_stderr(stderr_handle).await;
                Err(err)
            }
            _ => {
                // Timeout path (or stdout read error during timeout kill)
                // Write any partial output to log before the footer
                if let Ok(Ok((_, ref partial))) = stdout_result {
                    if let Some(writer) = log_writer.as_deref_mut() {
                        writer.write_bytes(partial);
                    }
                }
                let _ = self.collect_stderr(stderr_handle).await;
                if let Some(writer) = log_writer.as_deref_mut() {
                    writer.write_timeout_footer(&chrono::Utc::now().to_rfc3339());
                }
                let idle_secs = last_activity.lock().await.elapsed().as_secs();
                Err(RalphError::BackendTimeout {
                    backend: self.name.clone(),
                    idle_seconds: idle_secs,
                    timeout_kind: TimeoutKind::Idle,
                })
            }
        }
    }
}

#[async_trait]
impl Backend for CliBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, prompt: &str) -> Result<String> {
        self.execute_streaming(prompt, None).await
    }

    async fn execute_with_log(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
    ) -> Result<String> {
        self.execute_streaming(prompt, log_writer).await
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

        let mut claude_backend = claude::backend_from_config(config, None, None);
        claude_backend.invocation_ctx = shared_invocation.clone();
        backends.insert(
            "claude".to_owned(),
            backend_with_optional_tmux(claude_backend, &tmux, shared_ctx.clone()),
        );
        let mut codex_backend = codex::backend_from_config(config, None, None);
        codex_backend.invocation_ctx = shared_invocation.clone();
        backends.insert(
            "codex".to_owned(),
            backend_with_optional_tmux(codex_backend, &tmux, shared_ctx.clone()),
        );

        Self {
            backends,
            default_backend: config.workspace.default_backend.clone(),
            tmux_context: shared_ctx,
            invocation_context: shared_invocation,
            config: config.clone(),
            tmux,
        }
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

    fn get_or_create_inner(&mut self, spec: &str, role: Option<&str>) -> Result<Arc<dyn Backend>> {
        let parsed = parse_backend_spec(spec)?;
        let cache_key = match role {
            Some(r) => format!("{}:{r}", backend_spec_key(&parsed)),
            None => backend_spec_key(&parsed),
        };

        if let Some(backend) = self.backends.get(&cache_key) {
            return Ok(backend.clone());
        }

        let mut cli_backend = self.create_cli_backend_for_spec(&parsed, role)?;
        cli_backend.invocation_ctx = self.invocation_context.clone();
        let backend = backend_with_optional_tmux(
            cli_backend,
            &self.tmux,
            self.tmux_context.clone(),
        );
        self.backends.insert(cache_key, backend.clone());
        Ok(backend)
    }

    pub fn default_backend(&self) -> &str {
        &self.default_backend
    }

    pub fn opposite(&self, backend: &str) -> Result<&str> {
        let parsed = parse_backend_spec(backend)?;
        match parsed.name.as_str() {
            "claude" => Ok("codex"),
            "codex" => Ok("claude"),
            _ => Err(RalphError::Validation(format!(
                "unknown backend for opposite lookup: {backend}"
            ))),
        }
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

        Ok(CompletionLoopBackends {
            planner: self.resolve_backend_for_role(&planner, "planner"),
            completer: self.resolve_backend_for_role(&completer, "completer"),
        })
    }

    /// Collect model-injected backend specs configured for all roles across
    /// all known backends.
    pub fn backend_role_model_specs(&self) -> Vec<String> {
        let mut specs = BTreeSet::new();
        let roles = [
            "planner",
            "implementer",
            "reviewer",
            "qa",
            "completer",
            "reformatter",
        ];

        for (backend_name, models) in [
            ("claude", &self.config.backends.claude.models),
            ("codex", &self.config.backends.codex.models),
        ] {
            for role in roles {
                if let Some(model) = models.for_role(role) {
                    specs.insert(format!("{backend_name}({model})"));
                }
            }
        }

        specs.into_iter().collect()
    }

    pub async fn health_check_all(&self) -> Result<()> {
        for backend in self.backends.values() {
            backend.health_check().await?;
        }
        Ok(())
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
        let model = spec.model.as_deref();
        match spec.name.as_str() {
            "claude" => Ok(claude::backend_from_config(&self.config, model, role)),
            "codex" => Ok(codex::backend_from_config(&self.config, model, role)),
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
    use std::time::Duration;

    use tempfile::tempdir;

    use super::{
        parse_backend_spec, Backend, BackendRegistry, BackendRegistryTmuxConfig, BackendSpec,
        CliBackend,
    };
    use crate::config::GlobalConfig;
    use crate::error::{RalphError, TimeoutKind};
    use crate::output_log::LogWriter;

    #[test]
    fn parse_backend_spec_accepts_bare_name() {
        let parsed = parse_backend_spec("claude").expect("bare backend should parse");
        assert_eq!(
            parsed,
            BackendSpec {
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
                name: "claude".to_owned(),
                model: Some("opus".to_owned()),
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

        let mut writer = LogWriter::open(temp.path(), Some(1), None, "planner");
        let output = Backend::execute_with_log(&backend, "ignored", Some(&mut writer))
            .await
            .expect("backend should succeed");

        assert_eq!(output, "progress 10%\rprogress 20%\rpartial-line");
        let logged = fs::read(writer.path()).expect("read log bytes");
        assert_eq!(logged, b"progress 10%\rprogress 20%\rpartial-line");
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
sleep 30
"#,
                pid_file = pid_file.display()
            ),
        );

        let backend = CliBackend::new(
            "timeout-test",
            script_path.to_string_lossy().to_string(),
            vec![],
            Duration::from_millis(150),
            BTreeMap::new(),
        );

        let mut writer = LogWriter::open(temp.path(), Some(2), None, "implementer");
        let start = std::time::Instant::now();
        let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
        let elapsed = start.elapsed();

        // Timeout must fire near the configured idle threshold (150ms), not
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
        assert!(log_content.contains("--- timeout ts="));

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
        assert!(err_msg.contains("exec"), "error should mention 'exec': {err_msg}");
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
        assert!(!args.contains(&"old-session".to_owned()), "old session id must be replaced");
        assert!(!args.contains(&"text".to_owned()), "old output-format value must be replaced");
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
        assert!(!args.contains(&"old-thread".to_owned()), "old session must be replaced");
        assert_eq!(args[2], "new-thread", "new session id in correct position");
        assert_eq!(
            args.iter().filter(|a| *a == "--json").count(),
            1,
            "exactly one --json"
        );
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

        let mut writer = LogWriter::open(temp.path(), Some(3), None, "planner");
        let start = std::time::Instant::now();
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
}
