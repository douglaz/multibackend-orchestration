pub mod claude;
pub mod codex;
pub mod mock;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::config::GlobalConfig;
use crate::error::RalphError;
use crate::project::state::{CompletionLoopBackends, FeatureLoopBackends};
use crate::Result;

#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, prompt: &str) -> Result<String>;
    async fn health_check(&self) -> Result<()>;
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

    fn resolved_command_path(&self) -> PathBuf {
        which::which(&self.command).unwrap_or_else(|_| PathBuf::from(&self.command))
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

pub struct BackendRegistry {
    backends: HashMap<String, Arc<dyn Backend>>,
    default_backend: String,
}

impl BackendRegistry {
    pub fn new(config: &GlobalConfig) -> Self {
        let mut backends: HashMap<String, Arc<dyn Backend>> = HashMap::new();

        backends.insert(
            "claude".to_owned(),
            Arc::new(claude::backend_from_config(config)),
        );
        backends.insert(
            "codex".to_owned(),
            Arc::new(codex::backend_from_config(config)),
        );

        Self {
            backends,
            default_backend: config.workspace.default_backend.clone(),
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Backend>> {
        self.backends.get(name).cloned()
    }

    pub fn default_backend(&self) -> &str {
        &self.default_backend
    }

    pub fn opposite(&self, backend: &str) -> Result<&str> {
        match backend {
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

    pub fn assign_feature_backends(
        &self,
        loop_number: u32,
        starting_backend: &str,
    ) -> Result<FeatureLoopBackends> {
        let planner = self.planner_for_loop(loop_number, starting_backend)?;
        let implementer = self.opposite(&planner)?.to_owned();
        Ok(FeatureLoopBackends {
            planner: planner.clone(),
            implementer,
            reviewer: planner,
        })
    }

    pub fn assign_completion_backends(
        &self,
        loop_number: u32,
        starting_backend: &str,
    ) -> Result<CompletionLoopBackends> {
        let planner = self.planner_for_loop(loop_number, starting_backend)?;
        let completer = self.opposite(&planner)?.to_owned();
        Ok(CompletionLoopBackends { planner, completer })
    }

    pub async fn health_check_all(&self) -> Result<()> {
        for backend in self.backends.values() {
            backend.health_check().await?;
        }
        Ok(())
    }
}
