pub mod claude;
pub mod codex;
pub mod mock;
pub mod tmux;
pub mod tmux_backend;

pub use mock::MockBackend;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::config::GlobalConfig;
use crate::error::RalphError;
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

    if let Err(err) = fs::create_dir_all(loop_dir).await {
        warn!(
            backend = backend_name,
            role = role_value,
            path = %loop_dir.display(),
            error = %err,
            "failed to prepare loop directory for backend output artifact"
        );
        return;
    }

    match fs::write(&artifact_path, content).await {
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
    shared_context: Option<SharedTmuxContext>,
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
            shared_context: None,
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

    pub(crate) fn with_shared_context(mut self, shared_context: SharedTmuxContext) -> Self {
        self.shared_context = Some(shared_context);
        self
    }
}

#[async_trait]
impl Backend for CliBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, prompt: &str) -> Result<String> {
        let execution_ctx = match &self.shared_context {
            Some(shared_context) => Some(shared_context.get().await),
            None => None,
        };
        let role = execution_ctx.as_ref().and_then(|ctx| ctx.role.as_deref());

        let resolved_command = self.resolved_command_path();
        let mut cmd = Command::new(&resolved_command);
        cmd.args(&self.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .envs(self.env.clone());

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                debug!(
                    backend = self.name(),
                    role = ?role,
                    "skipping backend output artifact: process spawn failed"
                );
                return Err(RalphError::BackendCommandFailed {
                    backend: self.name.clone(),
                    details: format!(
                        "{err} (command='{}', resolved='{}')",
                        self.command,
                        resolved_command.display()
                    ),
                });
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.map_err(|err| {
                RalphError::BackendCommandFailed {
                    backend: self.name.clone(),
                    details: format!("failed to write prompt to stdin: {err}"),
                }
            })?;
        }

        let output = match timeout(self.timeout, child.wait_with_output()).await {
            Ok(wait_result) => wait_result.map_err(|err| RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: err.to_string(),
            })?,
            Err(_) => {
                debug!(
                    backend = self.name(),
                    role = ?role,
                    "skipping backend output artifact: command timed out before output capture"
                );
                return Err(RalphError::BackendTimeout {
                    backend: self.name.clone(),
                });
            }
        };

        persist_cli_output(
            execution_ctx
                .as_ref()
                .and_then(|ctx| ctx.loop_dir.as_deref()),
            self.name(),
            role,
            output.status.code(),
            &output.stdout,
            &output.stderr,
        )
        .await;

        if !output.status.success() {
            return Err(RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn health_check(&self) -> Result<()> {
        which::which(&self.command).map_err(|_| RalphError::BackendUnavailable {
            backend: self.name.clone(),
        })?;
        Ok(())
    }
}

/// Shared execution context that the orchestrator updates before each backend
/// invocation, allowing backend instances to read loop/role info without
/// a change to the Backend trait.
#[derive(Debug, Clone, Default)]
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

    /// Set the execution context (loop number, role, loop_dir) for the next
    /// backend invocation.
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
        Arc::new(backend.with_shared_context(shared_ctx))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use tempfile::tempdir;

    use super::tmux_backend::TmuxExecutionContext;
    use super::{parse_backend_spec, Backend, BackendSpec, CliBackend, SharedTmuxContext};
    use crate::error::RalphError;

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

    async fn cli_backend_with_context(
        command: &str,
        args: Vec<String>,
        timeout: Duration,
        loop_dir: Option<PathBuf>,
        role: Option<&str>,
    ) -> CliBackend {
        let shared = SharedTmuxContext::default();
        shared
            .set(TmuxExecutionContext {
                loop_number: Some(1),
                role: role.map(ToOwned::to_owned),
                loop_dir,
            })
            .await;

        CliBackend::new(
            "test-backend",
            command.to_owned(),
            args,
            timeout,
            BTreeMap::new(),
        )
        .with_shared_context(shared)
    }

    fn list_agent_output_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let entries = std::fs::read_dir(dir).expect("read_dir should succeed");
        for entry in entries {
            let entry = entry.expect("directory entry should load");
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.path().is_file() && name.contains("-agent-output-") && name.ends_with(".log") {
                files.push(entry.path());
            }
        }
        files.sort();
        files
    }

    #[tokio::test]
    async fn cli_backend_writes_output_artifact_on_success() {
        let loop_dir = tempdir().expect("tempdir should succeed");
        let backend = cli_backend_with_context(
            "cat",
            vec![],
            Duration::from_secs(2),
            Some(loop_dir.path().to_path_buf()),
            Some("implementer"),
        )
        .await;

        let output = Backend::execute(&backend, "hello artifact\n")
            .await
            .expect("backend should succeed");
        assert_eq!(output, "hello artifact\n");

        let files = list_agent_output_files(loop_dir.path());
        assert_eq!(files.len(), 1, "expected exactly one output artifact");

        let content = std::fs::read_to_string(&files[0]).expect("artifact should be readable");
        assert!(content.contains("backend: test-backend"));
        assert!(content.contains("role: implementer"));
        assert!(content.contains("exit_status: 0"));
        assert!(content.contains("=== STDOUT ==="));
        assert!(content.contains("hello artifact"));
        assert!(content.contains("=== STDERR ==="));
    }

    #[tokio::test]
    async fn cli_backend_writes_output_artifact_on_nonzero_exit() {
        let loop_dir = tempdir().expect("tempdir should succeed");
        let backend = cli_backend_with_context(
            "sh",
            vec![
                "-c".to_owned(),
                "echo stdout-line; echo stderr-line >&2; exit 7".to_owned(),
            ],
            Duration::from_secs(2),
            Some(loop_dir.path().to_path_buf()),
            Some("implementer"),
        )
        .await;

        let result = Backend::execute(&backend, "ignored").await;
        match result {
            Err(RalphError::BackendCommandFailed { .. }) => {}
            other => panic!("expected BackendCommandFailed, got: {other:?}"),
        }

        let files = list_agent_output_files(loop_dir.path());
        assert_eq!(files.len(), 1, "expected exactly one output artifact");

        let content = std::fs::read_to_string(&files[0]).expect("artifact should be readable");
        assert!(content.contains("exit_status: 7"));
        assert!(content.contains("stdout-line"));
        assert!(content.contains("stderr-line"));
    }

    #[tokio::test]
    async fn cli_backend_does_not_write_artifact_when_loop_dir_is_none() {
        let output_dir = tempdir().expect("tempdir should succeed");
        let backend =
            cli_backend_with_context("cat", vec![], Duration::from_secs(2), None, Some("qa")).await;

        let output = Backend::execute(&backend, "no artifact expected\n")
            .await
            .expect("backend should succeed");
        assert_eq!(output, "no artifact expected\n");

        let files = list_agent_output_files(output_dir.path());
        assert!(
            files.is_empty(),
            "expected no output artifacts when loop_dir is not set"
        );
    }

    #[tokio::test]
    async fn cli_backend_artifact_filename_has_timestamp_prefix() {
        let loop_dir = tempdir().expect("tempdir should succeed");
        let backend = cli_backend_with_context(
            "cat",
            vec![],
            Duration::from_secs(2),
            Some(loop_dir.path().to_path_buf()),
            Some("implementer"),
        )
        .await;

        Backend::execute(&backend, "filename test")
            .await
            .expect("backend should succeed");

        let files = list_agent_output_files(loop_dir.path());
        assert_eq!(files.len(), 1, "expected one artifact");
        let name = files[0]
            .file_name()
            .and_then(|n| n.to_str())
            .expect("filename should be UTF-8");

        assert!(
            name.len() > 14,
            "filename should include a 14-digit timestamp prefix: {name}"
        );
        assert!(
            name.as_bytes()[..14]
                .iter()
                .copied()
                .all(|ch| ch.is_ascii_digit()),
            "filename should start with YYYYMMDDHHMMSS timestamp: {name}"
        );
        assert!(
            name.contains("-agent-output-implementer-"),
            "filename missing role section: {name}"
        );
        assert!(
            name.ends_with(".log"),
            "filename should end with .log: {name}"
        );
    }

    #[tokio::test]
    async fn cli_backend_counter_makes_filenames_unique_on_rapid_invocations() {
        let loop_dir = tempdir().expect("tempdir should succeed");
        let backend = cli_backend_with_context(
            "cat",
            vec![],
            Duration::from_secs(2),
            Some(loop_dir.path().to_path_buf()),
            Some("reviewer"),
        )
        .await;

        Backend::execute(&backend, "first")
            .await
            .expect("first invocation should succeed");
        Backend::execute(&backend, "second")
            .await
            .expect("second invocation should succeed");

        let files = list_agent_output_files(loop_dir.path());
        assert_eq!(files.len(), 2, "expected two artifacts");
        let unique_names = files
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .expect("filename should be utf-8")
                    .to_owned()
            })
            .collect::<HashSet<_>>();
        assert_eq!(unique_names.len(), 2, "filenames must be unique");
    }

    #[tokio::test]
    async fn cli_backend_does_not_write_artifact_on_timeout() {
        let loop_dir = tempdir().expect("tempdir should succeed");
        let backend = cli_backend_with_context(
            "sh",
            vec!["-c".to_owned(), "sleep 1".to_owned()],
            Duration::from_millis(25),
            Some(loop_dir.path().to_path_buf()),
            Some("qa"),
        )
        .await;

        let result = Backend::execute(&backend, "timeout").await;
        match result {
            Err(RalphError::BackendTimeout { .. }) => {}
            other => panic!("expected BackendTimeout, got: {other:?}"),
        }

        let files = list_agent_output_files(loop_dir.path());
        assert!(files.is_empty(), "expected no artifact on timeout");
    }

    #[tokio::test]
    async fn cli_backend_does_not_write_artifact_on_spawn_failure() {
        let loop_dir = tempdir().expect("tempdir should succeed");
        let backend = cli_backend_with_context(
            "__ralph_command_should_not_exist__",
            vec![],
            Duration::from_secs(1),
            Some(loop_dir.path().to_path_buf()),
            Some("implementer"),
        )
        .await;

        let result = Backend::execute(&backend, "spawn fail").await;
        match result {
            Err(RalphError::BackendCommandFailed { .. }) => {}
            other => panic!("expected BackendCommandFailed, got: {other:?}"),
        }

        let files = list_agent_output_files(loop_dir.path());
        assert!(files.is_empty(), "expected no artifact on spawn failure");
    }
}
