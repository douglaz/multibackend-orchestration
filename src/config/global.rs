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
    #[serde(default)]
    pub tmux: bool,
    #[serde(default = "default_tmux_session")]
    pub tmux_session: String,
    #[serde(default = "default_tmux_window_keep_seconds")]
    pub tmux_window_keep_seconds: u64,
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
    #[serde(default)]
    pub models: BackendRoleModels,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendRoleModels {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub completer: Option<String>,
    pub reformatter: Option<String>,
}

impl BackendRoleModels {
    pub fn for_role(&self, role: &str) -> Option<&str> {
        match role {
            "planner" => self.planner.as_deref(),
            "implementer" => self.implementer.as_deref(),
            "reviewer" => self.reviewer.as_deref(),
            "completer" => self.completer.as_deref(),
            "reformatter" => self.reformatter.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub max_review_iterations: u32,
    pub auto_commit: bool,
    pub commit_message_style: CommitMessageStyle,
    pub commit_tag_format: String,
    pub prompt_change_action: PromptChangeAction,
    #[serde(default)]
    pub planner_backend: Option<String>,
    #[serde(default)]
    pub implementer_backend: Option<String>,
    #[serde(default)]
    pub reviewer_backend: Option<String>,
    #[serde(default)]
    pub completer_backend: Option<String>,
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
                tmux: false,
                tmux_session: "ralph".to_owned(),
                tmux_window_keep_seconds: 5,
            },
            backends: BackendConfigs {
                claude: BackendConfig {
                    command: "claude".to_owned(),
                    args: vec!["--dangerously-skip-permissions".to_owned()],
                    timeout_seconds: 600,
                    env: BTreeMap::new(),
                    models: BackendRoleModels {
                        planner: Some("claude-sonnet-4-5-20250929".to_owned()),
                        implementer: Some("claude-sonnet-4-5-20250929".to_owned()),
                        reviewer: Some("claude-sonnet-4-5-20250929".to_owned()),
                        completer: Some("claude-sonnet-4-5-20250929".to_owned()),
                        reformatter: Some("claude-sonnet-4-5-20250929".to_owned()),
                    },
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
                    models: BackendRoleModels {
                        planner: Some("o3".to_owned()),
                        implementer: Some("o3".to_owned()),
                        reviewer: Some("o3".to_owned()),
                        completer: Some("o3".to_owned()),
                        reformatter: Some("o3".to_owned()),
                    },
                },
            },
            workflow: WorkflowConfig {
                max_review_iterations: 30,
                auto_commit: true,
                commit_message_style: CommitMessageStyle::Conventional,
                commit_tag_format: "ralph/{project_id}/loop-{loop_number}".to_owned(),
                prompt_change_action: PromptChangeAction::Abort,
                planner_backend: None,
                implementer_backend: None,
                reviewer_backend: None,
                completer_backend: None,
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

fn default_tmux_session() -> String {
    "ralph".to_owned()
}

fn default_tmux_window_keep_seconds() -> u64 {
    5
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

#[cfg(test)]
mod tests {
    use super::{BackendRoleModels, GlobalConfig};

    #[test]
    fn default_workspace_tmux_settings_match_expected_values() {
        let config = GlobalConfig::default();
        assert!(!config.workspace.tmux);
        assert_eq!(config.workspace.tmux_session, "ralph");
        assert_eq!(config.workspace.tmux_window_keep_seconds, 5);
    }

    #[test]
    fn backend_role_models_default_is_empty() {
        let models = BackendRoleModels::default();
        assert!(models.planner.is_none());
        assert!(models.implementer.is_none());
        assert!(models.reviewer.is_none());
        assert!(models.completer.is_none());
        assert!(models.reformatter.is_none());
    }

    #[test]
    fn deserializes_workspace_tmux_defaults_when_fields_are_missing() {
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"

[backends.claude]
command = "claude"
timeout_seconds = 600

[backends.codex]
command = "codex"
timeout_seconds = 600

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/planner.md"
implementer = "templates/implementer.md"
reviewer = "templates/reviewer.md"
completer = "templates/completer.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
"#;

        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert!(!config.workspace.tmux);
        assert_eq!(config.workspace.tmux_session, "ralph");
        assert_eq!(config.workspace.tmux_window_keep_seconds, 5);
        assert!(config.backends.claude.models.planner.is_none());
        assert!(config.backends.claude.models.implementer.is_none());
        assert!(config.backends.claude.models.reviewer.is_none());
        assert!(config.backends.claude.models.completer.is_none());
        assert!(config.backends.claude.models.reformatter.is_none());
        assert!(config.backends.codex.models.planner.is_none());
        assert!(config.backends.codex.models.implementer.is_none());
        assert!(config.backends.codex.models.reviewer.is_none());
        assert!(config.backends.codex.models.completer.is_none());
        assert!(config.backends.codex.models.reformatter.is_none());
    }

    #[test]
    fn deserializes_workspace_tmux_fields_when_present() {
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"
tmux = true
tmux_session = "demo"
tmux_window_keep_seconds = 10

[backends.claude]
command = "claude"
timeout_seconds = 600

[backends.codex]
command = "codex"
timeout_seconds = 600

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/planner.md"
implementer = "templates/implementer.md"
reviewer = "templates/reviewer.md"
completer = "templates/completer.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
"#;

        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert!(config.workspace.tmux);
        assert_eq!(config.workspace.tmux_session, "demo");
        assert_eq!(config.workspace.tmux_window_keep_seconds, 10);
    }

    #[test]
    fn deserializes_backend_models_when_present() {
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"

[backends.claude]
command = "claude"
timeout_seconds = 600

[backends.claude.models]
planner = "claude-sonnet-4-5-20250929"
implementer = "claude-sonnet-4-5-20250929"
reviewer = "claude-sonnet-4-5-20250929"
completer = "claude-sonnet-4-5-20250929"
reformatter = "claude-sonnet-4-5-20250929"

[backends.codex]
command = "codex"
timeout_seconds = 600

[backends.codex.models]
planner = "o3"
implementer = "o3"
reviewer = "o3"
completer = "o3"
reformatter = "o3"

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/planner.md"
implementer = "templates/implementer.md"
reviewer = "templates/reviewer.md"
completer = "templates/completer.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
"#;

        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert_eq!(
            config.backends.claude.models.planner.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(
            config.backends.claude.models.implementer.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(
            config.backends.claude.models.reviewer.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(
            config.backends.claude.models.completer.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(
            config.backends.claude.models.reformatter.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(config.backends.codex.models.planner.as_deref(), Some("o3"));
        assert_eq!(
            config.backends.codex.models.implementer.as_deref(),
            Some("o3")
        );
        assert_eq!(config.backends.codex.models.reviewer.as_deref(), Some("o3"));
        assert_eq!(
            config.backends.codex.models.completer.as_deref(),
            Some("o3")
        );
        assert_eq!(
            config.backends.codex.models.reformatter.as_deref(),
            Some("o3")
        );
    }

    #[test]
    fn for_role_returns_expected_model_for_each_role() {
        let models = BackendRoleModels {
            planner: Some("planner-model".to_owned()),
            implementer: Some("implementer-model".to_owned()),
            reviewer: Some("reviewer-model".to_owned()),
            completer: Some("completer-model".to_owned()),
            reformatter: Some("reformatter-model".to_owned()),
        };

        assert_eq!(models.for_role("planner"), Some("planner-model"));
        assert_eq!(models.for_role("implementer"), Some("implementer-model"));
        assert_eq!(models.for_role("reviewer"), Some("reviewer-model"));
        assert_eq!(models.for_role("completer"), Some("completer-model"));
        assert_eq!(models.for_role("reformatter"), Some("reformatter-model"));
        assert_eq!(models.for_role("unknown-role"), None);
    }
}
