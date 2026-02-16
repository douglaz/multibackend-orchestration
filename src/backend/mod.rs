pub mod claude;
pub mod codex;
pub mod mock;
pub mod tmux;
pub mod tmux_backend;

pub use mock::MockBackend;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::config::GlobalConfig;
use crate::error::RalphError;
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

#[derive(Debug, Clone)]
pub struct CliBackend {
    name: String,
    command: String,
    args: Vec<String>,
    timeout: Duration,
    env: BTreeMap<String, String>,
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

    async fn kill_and_reap_child(&self, child: &mut tokio::process::Child) {
        if let Err(err) = child.kill().await {
            if err.kind() != ErrorKind::InvalidInput {
                warn!(
                    backend = %self.name,
                    error = %err,
                    "failed to kill child process during cleanup"
                );
            }
        }
        if let Err(err) = child.wait().await {
            warn!(
                backend = %self.name,
                error = %err,
                "failed waiting for child process during cleanup"
            );
        }
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
        let resolved_command = self.resolved_command_path();
        let mut cmd = Command::new(&resolved_command);
        cmd.args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .envs(self.env.clone());

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

        let stderr_backend = self.name.clone();
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

        let execution_result = timeout(self.timeout, async {
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
                if let Some(writer) = log_writer.as_deref_mut() {
                    writer.write_bytes(bytes);
                }
            }

            let status = child
                .wait()
                .await
                .map_err(|err| RalphError::BackendCommandFailed {
                    backend: self.name.clone(),
                    details: format!("failed waiting for child process: {err}"),
                })?;

            Ok::<(std::process::ExitStatus, Vec<u8>), RalphError>((status, captured_stdout))
        })
        .await;

        match execution_result {
            Ok(Ok((status, captured_stdout))) => {
                let stderr_bytes = self.collect_stderr(stderr_handle).await?;

                if !status.success() {
                    return Err(RalphError::BackendCommandFailed {
                        backend: self.name.clone(),
                        details: String::from_utf8_lossy(&stderr_bytes).trim().to_owned(),
                    });
                }

                Ok(String::from_utf8_lossy(&captured_stdout).to_string())
            }
            Ok(Err(err)) => {
                self.kill_and_reap_child(&mut child).await;
                let _ = self.collect_stderr(stderr_handle).await;
                Err(err)
            }
            Err(_) => {
                self.kill_and_reap_child(&mut child).await;
                let _ = self.collect_stderr(stderr_handle).await;
                if let Some(writer) = log_writer.as_deref_mut() {
                    writer.write_timeout_footer(&chrono::Utc::now().to_rfc3339());
                }
                Err(RalphError::BackendTimeout {
                    backend: self.name.clone(),
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

        let claude_backend = claude::backend_from_config(config, None);
        backends.insert(
            "claude".to_owned(),
            backend_with_optional_tmux(claude_backend, &tmux, shared_ctx.clone()),
        );
        let codex_backend = codex::backend_from_config(config, None);
        backends.insert(
            "codex".to_owned(),
            backend_with_optional_tmux(codex_backend, &tmux, shared_ctx.clone()),
        );

        Self {
            backends,
            default_backend: config.workspace.default_backend.clone(),
            tmux_context: shared_ctx,
            config: config.clone(),
            tmux,
        }
    }

    /// Set the tmux execution context (loop number, role) for the next backend
    /// invocation. This is a no-op when tmux mode is disabled.
    pub async fn set_tmux_context(&self, ctx: TmuxExecutionContext) {
        self.tmux_context.set(ctx).await;
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Backend>> {
        self.backends.get(name).cloned()
    }

    pub fn get_or_create_for_spec(&mut self, spec: &str) -> Result<Arc<dyn Backend>> {
        let parsed = parse_backend_spec(spec)?;
        let cache_key = backend_spec_key(&parsed);

        if let Some(backend) = self.backends.get(&cache_key) {
            return Ok(backend.clone());
        }

        let backend = backend_with_optional_tmux(
            self.create_cli_backend_for_spec(&parsed)?,
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

    fn create_cli_backend_for_spec(&self, spec: &BackendSpec) -> Result<CliBackend> {
        let model = spec.model.as_deref();
        match spec.name.as_str() {
            "claude" => Ok(claude::backend_from_config(&self.config, model)),
            "codex" => Ok(codex::backend_from_config(&self.config, model)),
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
    use crate::error::RalphError;
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
        let result = Backend::execute_with_log(&backend, "ignored", Some(&mut writer)).await;
        match result {
            Err(RalphError::BackendTimeout { backend }) => assert_eq!(backend, "timeout-test"),
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
}
