pub mod claude;
pub mod codex;
pub mod mock;
pub mod tmux;
pub mod tmux_backend;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::project::state::{CompletionLoopBackends, FeatureLoopBackends};
use crate::Result;

use self::tmux::RealTmuxRunner;
use self::tmux_backend::{TmuxBackend, TmuxExecutionContext};

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
}

#[async_trait]
impl Backend for CliBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, prompt: &str) -> Result<String> {
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

        let output = timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| RalphError::BackendTimeout {
                backend: self.name.clone(),
            })?
            .map_err(|err| RalphError::BackendCommandFailed {
                backend: self.name.clone(),
                details: err.to_string(),
            })?;

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

        Ok(FeatureLoopBackends {
            planner: self.resolve_backend_for_role(&planner, "planner"),
            implementer: self.resolve_backend_for_role(&implementer, "implementer"),
            reviewer: self.resolve_backend_for_role(&reviewer, "reviewer"),
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
    use super::{parse_backend_spec, BackendSpec};

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
}
