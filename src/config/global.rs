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
    use super::GlobalConfig;

    #[test]
    fn default_workspace_tmux_settings_match_expected_values() {
        let config = GlobalConfig::default();
        assert!(!config.workspace.tmux);
        assert_eq!(config.workspace.tmux_session, "ralph");
        assert_eq!(config.workspace.tmux_window_keep_seconds, 5);
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
}
