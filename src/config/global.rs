use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub workspace: WorkspaceConfig,
    pub backends: BackendConfigs,
    pub workflow: WorkflowConfig,
    pub templates: TemplateConfig,
    pub git: GitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub version: String,
    pub default_backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfigs {
    pub claude: BackendConfig,
    pub codex: BackendConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub max_review_iterations: u32,
    pub auto_commit: bool,
    pub commit_message_style: CommitMessageStyle,
    pub commit_tag_format: String,
    pub prompt_change_action: PromptChangeAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommitMessageStyle {
    Conventional,
    Descriptive,
    Minimal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum PromptChangeAction {
    Continue,
    RestartLoop,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    pub planner: String,
    pub implementer: String,
    pub reviewer: String,
    pub completer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    pub auto_branch: bool,
    pub branch_format: String,
    pub sign_commits: bool,
    pub base_branch: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            workspace: WorkspaceConfig {
                version: "1.0".to_owned(),
                default_backend: "claude".to_owned(),
            },
            backends: BackendConfigs {
                claude: BackendConfig {
                    command: "claude".to_owned(),
                    args: vec!["--dangerously-skip-permissions".to_owned()],
                    timeout_seconds: 600,
                    env: BTreeMap::new(),
                },
                codex: BackendConfig {
                    command: "codex".to_owned(),
                    args: vec![
                        "exec".to_owned(),
                        "--dangerously-bypass-approvals-and-sandbox".to_owned(),
                        "-".to_owned(),
                    ],
                    timeout_seconds: 600,
                    env: BTreeMap::new(),
                },
            },
            workflow: WorkflowConfig {
                max_review_iterations: 5,
                auto_commit: true,
                commit_message_style: CommitMessageStyle::Conventional,
                commit_tag_format: "ralph/{project_id}/loop-{loop_number}".to_owned(),
                prompt_change_action: PromptChangeAction::Abort,
            },
            templates: TemplateConfig {
                planner: "templates/planner.md".to_owned(),
                implementer: "templates/implementer.md".to_owned(),
                reviewer: "templates/reviewer.md".to_owned(),
                completer: "templates/completer.md".to_owned(),
            },
            git: GitConfig {
                auto_branch: true,
                branch_format: "ralph/{project_id}".to_owned(),
                sign_commits: false,
                base_branch: "master".to_owned(),
            },
        }
    }
}

impl GlobalConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let config: Self = toml::from_str(&raw)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }

    pub fn backend_config(&self, name: &str) -> Option<&BackendConfig> {
        match name {
            "claude" => Some(&self.backends.claude),
            "codex" => Some(&self.backends.codex),
            _ => None,
        }
    }
}
