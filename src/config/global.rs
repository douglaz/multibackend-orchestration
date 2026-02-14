use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use clap::ValueEnum;
use serde::{Deserialize, Deserializer, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct GlobalConfig {
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub backends: BackendConfigs,
    #[serde(default)]
    pub workflow: WorkflowConfig,
    #[serde(default)]
    pub templates: TemplateConfig,
    #[serde(default)]
    pub git: GitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_version")]
    pub version: String,
    #[serde(default = "default_workspace_default_backend")]
    pub default_backend: String,
    #[serde(default)]
    pub tmux: bool,
    #[serde(default = "default_tmux_session")]
    pub tmux_session: String,
    #[serde(default = "default_tmux_window_keep_seconds")]
    pub tmux_window_keep_seconds: u64,
    #[serde(default = "default_daemon_poll_seconds")]
    pub daemon_poll_seconds: u64,
    #[serde(default = "default_daemon_max_concurrent")]
    pub daemon_max_concurrent: u32,
    #[serde(default = "default_daemon_labels")]
    pub daemon_labels: Vec<String>,
    #[serde(default)]
    pub daemon_repo: Option<String>,
    #[serde(default = "default_daemon_refinement_enabled")]
    pub daemon_refinement_enabled: bool,
    #[serde(default = "default_daemon_refinement_backend")]
    pub daemon_refinement_backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackendConfigs {
    #[serde(
        default = "default_claude_backend_config",
        deserialize_with = "deserialize_claude_backend_config"
    )]
    pub claude: BackendConfig,
    #[serde(
        default = "default_codex_backend_config",
        deserialize_with = "deserialize_codex_backend_config"
    )]
    pub codex: BackendConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackendConfig {
    #[serde(default = "default_backend_command")]
    pub command: String,
    #[serde(default = "default_backend_args")]
    pub args: Vec<String>,
    #[serde(default = "default_backend_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub models: BackendRoleModels,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct BackendRoleModels {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub qa: Option<String>,
    pub completer: Option<String>,
    pub acceptance_qa: Option<String>,
    pub reformatter: Option<String>,
}

impl BackendRoleModels {
    pub fn for_role(&self, role: &str) -> Option<&str> {
        match role {
            "planner" => self.planner.as_deref(),
            "implementer" => self.implementer.as_deref(),
            "reviewer" => self.reviewer.as_deref(),
            "qa" => self.qa.as_deref(),
            "completer" => self.completer.as_deref(),
            "acceptance_qa" => self.acceptance_qa.as_deref(),
            "reformatter" => self.reformatter.as_deref(),
            _ => None,
        }
    }

    /// Fill any `None` fields from `defaults`.
    pub fn fill_from(&mut self, defaults: &BackendRoleModels) {
        if self.planner.is_none() {
            self.planner.clone_from(&defaults.planner);
        }
        if self.implementer.is_none() {
            self.implementer.clone_from(&defaults.implementer);
        }
        if self.reviewer.is_none() {
            self.reviewer.clone_from(&defaults.reviewer);
        }
        if self.qa.is_none() {
            self.qa.clone_from(&defaults.qa);
        }
        if self.completer.is_none() {
            self.completer.clone_from(&defaults.completer);
        }
        if self.acceptance_qa.is_none() {
            self.acceptance_qa.clone_from(&defaults.acceptance_qa);
        }
        if self.reformatter.is_none() {
            self.reformatter.clone_from(&defaults.reformatter);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkflowConfig {
    #[serde(default = "default_max_review_iterations")]
    pub max_review_iterations: u32,
    #[serde(default = "default_auto_commit")]
    pub auto_commit: bool,
    #[serde(default = "default_commit_message_style")]
    pub commit_message_style: CommitMessageStyle,
    #[serde(default = "default_commit_tag_format")]
    pub commit_tag_format: String,
    #[serde(default = "default_prompt_change_action")]
    pub prompt_change_action: PromptChangeAction,
    #[serde(default = "default_prompt_review_enabled")]
    pub prompt_review_enabled: bool,
    #[serde(default = "default_prompt_review_backend")]
    pub prompt_review_backend: String,
    #[serde(default)]
    pub planner_backend: Option<String>,
    #[serde(default)]
    pub implementer_backend: Option<String>,
    #[serde(default)]
    pub reviewer_backend: Option<String>,
    #[serde(default)]
    pub qa_backend: Option<String>,
    #[serde(default)]
    pub completer_backend: Option<String>,
    #[serde(default = "default_qa_enabled")]
    pub qa_enabled: bool,
    #[serde(default = "default_max_qa_iterations")]
    pub max_qa_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CommitMessageStyle {
    #[default]
    Conventional,
    Descriptive,
    Minimal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[clap(rename_all = "kebab-case")]
pub enum PromptChangeAction {
    Continue,
    RestartLoop,
    #[default]
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TemplateConfig {
    #[serde(default = "default_planner_template_path")]
    pub planner: String,
    #[serde(default = "default_implementer_template_path")]
    pub implementer: String,
    #[serde(default = "default_reviewer_template_path")]
    pub reviewer: String,
    #[serde(default = "default_prompt_reviewer_template_path")]
    pub prompt_reviewer: String,
    #[serde(default = "default_completer_template_path")]
    pub completer: String,
    #[serde(default = "default_qa_template_path")]
    pub qa: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GitConfig {
    #[serde(default = "default_git_auto_branch")]
    pub auto_branch: bool,
    #[serde(default = "default_git_branch_format")]
    pub branch_format: String,
    #[serde(default = "default_git_sign_commits")]
    pub sign_commits: bool,
    #[serde(default = "default_git_base_branch")]
    pub base_branch: String,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            version: default_workspace_version(),
            default_backend: default_workspace_default_backend(),
            tmux: false,
            tmux_session: default_tmux_session(),
            tmux_window_keep_seconds: default_tmux_window_keep_seconds(),
            daemon_poll_seconds: default_daemon_poll_seconds(),
            daemon_max_concurrent: default_daemon_max_concurrent(),
            daemon_labels: default_daemon_labels(),
            daemon_repo: None,
            daemon_refinement_enabled: default_daemon_refinement_enabled(),
            daemon_refinement_backend: default_daemon_refinement_backend(),
        }
    }
}

impl Default for BackendConfigs {
    fn default() -> Self {
        Self {
            claude: default_claude_backend_config(),
            codex: default_codex_backend_config(),
        }
    }
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            command: default_backend_command(),
            args: default_backend_args(),
            timeout_seconds: default_backend_timeout_seconds(),
            env: BTreeMap::new(),
            models: BackendRoleModels::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct PartialBackendConfig {
    command: Option<String>,
    args: Option<Vec<String>>,
    timeout_seconds: Option<u64>,
    env: Option<BTreeMap<String, String>>,
    models: Option<BackendRoleModels>,
}

impl PartialBackendConfig {
    fn into_backend_config_with_defaults(self, mut defaults: BackendConfig) -> BackendConfig {
        if let Some(command) = self.command {
            defaults.command = command;
        }
        if let Some(args) = self.args {
            defaults.args = args;
        }
        if let Some(timeout_seconds) = self.timeout_seconds {
            defaults.timeout_seconds = timeout_seconds;
        }
        if let Some(env) = self.env {
            defaults.env = env;
        }
        if let Some(mut models) = self.models {
            models.fill_from(&defaults.models);
            defaults.models = models;
        }
        defaults
    }
}

fn deserialize_claude_backend_config<'de, D>(
    deserializer: D,
) -> std::result::Result<BackendConfig, D::Error>
where
    D: Deserializer<'de>,
{
    let partial = PartialBackendConfig::deserialize(deserializer)?;
    Ok(partial.into_backend_config_with_defaults(default_claude_backend_config()))
}

fn deserialize_codex_backend_config<'de, D>(
    deserializer: D,
) -> std::result::Result<BackendConfig, D::Error>
where
    D: Deserializer<'de>,
{
    let partial = PartialBackendConfig::deserialize(deserializer)?;
    Ok(partial.into_backend_config_with_defaults(default_codex_backend_config()))
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_review_iterations: default_max_review_iterations(),
            auto_commit: default_auto_commit(),
            commit_message_style: default_commit_message_style(),
            commit_tag_format: default_commit_tag_format(),
            prompt_change_action: default_prompt_change_action(),
            prompt_review_enabled: default_prompt_review_enabled(),
            prompt_review_backend: default_prompt_review_backend(),
            planner_backend: None,
            implementer_backend: None,
            reviewer_backend: None,
            qa_backend: None,
            completer_backend: None,
            qa_enabled: default_qa_enabled(),
            max_qa_iterations: default_max_qa_iterations(),
        }
    }
}

impl Default for TemplateConfig {
    fn default() -> Self {
        Self {
            planner: default_planner_template_path(),
            implementer: default_implementer_template_path(),
            reviewer: default_reviewer_template_path(),
            prompt_reviewer: default_prompt_reviewer_template_path(),
            completer: default_completer_template_path(),
            qa: default_qa_template_path(),
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            auto_branch: default_git_auto_branch(),
            branch_format: default_git_branch_format(),
            sign_commits: default_git_sign_commits(),
            base_branch: default_git_base_branch(),
        }
    }
}

fn default_workspace_version() -> String {
    "1.0".to_owned()
}

fn default_workspace_default_backend() -> String {
    "claude".to_owned()
}

fn default_backend_command() -> String {
    "claude".to_owned()
}

fn default_backend_args() -> Vec<String> {
    Vec::new()
}

fn default_backend_timeout_seconds() -> u64 {
    7200
}

fn default_claude_backend_config() -> BackendConfig {
    BackendConfig {
        command: "claude".to_owned(),
        args: vec![
            "-p".to_owned(),
            "--permission-mode".to_owned(),
            "acceptEdits".to_owned(),
            "--allowedTools".to_owned(),
            "Bash,Edit,Write,Read,Glob,Grep,WebSearch,WebFetch,Task,TaskOutput,TaskStop".to_owned(),
        ],
        timeout_seconds: default_backend_timeout_seconds(),
        env: BTreeMap::new(),
        models: BackendRoleModels {
            planner: Some("opus".to_owned()),
            implementer: Some("opus".to_owned()),
            reviewer: Some("opus".to_owned()),
            qa: Some("opus".to_owned()),
            completer: Some("opus".to_owned()),
            acceptance_qa: Some("opus".to_owned()),
            reformatter: Some("sonnet".to_owned()),
        },
    }
}

fn default_codex_backend_config() -> BackendConfig {
    BackendConfig {
        command: "codex".to_owned(),
        args: vec![
            "exec".to_owned(),
            "--dangerously-bypass-approvals-and-sandbox".to_owned(),
            "-".to_owned(),
        ],
        timeout_seconds: default_backend_timeout_seconds(),
        env: BTreeMap::new(),
        models: BackendRoleModels {
            planner: Some("gpt-5.3-codex-xhigh".to_owned()),
            implementer: Some("gpt-5.3-codex-high".to_owned()),
            reviewer: Some("gpt-5.3-codex-high".to_owned()),
            qa: Some("gpt-5.3-codex-high".to_owned()),
            completer: Some("gpt-5.3-codex-xhigh".to_owned()),
            acceptance_qa: Some("gpt-5.3-codex-xhigh".to_owned()),
            reformatter: Some("gpt-5.3-codex-medium".to_owned()),
        },
    }
}

fn default_max_review_iterations() -> u32 {
    30
}

fn default_auto_commit() -> bool {
    true
}

fn default_commit_message_style() -> CommitMessageStyle {
    CommitMessageStyle::default()
}

fn default_commit_tag_format() -> String {
    "ralph/{project_id}/loop-{loop_number}".to_owned()
}

fn default_prompt_change_action() -> PromptChangeAction {
    PromptChangeAction::default()
}

fn default_planner_template_path() -> String {
    "templates/spec.md".to_owned()
}

fn default_implementer_template_path() -> String {
    "templates/implementation.md".to_owned()
}

fn default_reviewer_template_path() -> String {
    "templates/review.md".to_owned()
}

fn default_completer_template_path() -> String {
    "templates/completion.md".to_owned()
}

fn default_git_auto_branch() -> bool {
    true
}

fn default_git_branch_format() -> String {
    "ralph/{project_id}".to_owned()
}

fn default_git_sign_commits() -> bool {
    false
}

fn default_git_base_branch() -> String {
    "master".to_owned()
}

fn default_tmux_session() -> String {
    "ralph".to_owned()
}

fn default_tmux_window_keep_seconds() -> u64 {
    5
}

fn default_daemon_poll_seconds() -> u64 {
    60
}

fn default_daemon_max_concurrent() -> u32 {
    5
}

fn default_daemon_labels() -> Vec<String> {
    vec!["ralph:ready".to_owned()]
}

fn default_daemon_refinement_enabled() -> bool {
    true
}

fn default_daemon_refinement_backend() -> String {
    "claude(sonnet)".to_owned()
}

fn default_qa_enabled() -> bool {
    true
}

fn default_prompt_review_enabled() -> bool {
    true
}

fn default_prompt_review_backend() -> String {
    "codex(gpt-5.3-codex-xhigh)".to_owned()
}

fn default_max_qa_iterations() -> u32 {
    3
}

fn default_prompt_reviewer_template_path() -> String {
    "templates/prompt_reviewer.md".to_owned()
}

fn default_qa_template_path() -> String {
    "templates/qa.md".to_owned()
}

impl GlobalConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&raw)?;
        let defaults = Self::default();
        config
            .backends
            .claude
            .models
            .fill_from(&defaults.backends.claude.models);
        config
            .backends
            .codex
            .models
            .fill_from(&defaults.backends.codex.models);
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
    fn empty_toml_deserializes_to_defaults() {
        let config: GlobalConfig = toml::from_str("").expect("empty TOML should deserialize");
        assert_eq!(config, GlobalConfig::default());
    }

    #[test]
    fn whitespace_toml_deserializes_to_defaults() {
        let config: GlobalConfig =
            toml::from_str("   \n  ").expect("whitespace-only TOML should deserialize");
        assert_eq!(config, GlobalConfig::default());
    }

    #[test]
    fn partial_sections_default_correctly() {
        let raw = r#"
[workspace]
default_backend = "codex"

[workflow]
auto_commit = false

[templates]
planner = "templates/custom-spec.md"

[git]
base_branch = "main"
"#;

        let defaults = GlobalConfig::default();
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");

        assert_eq!(config.workspace.default_backend, "codex");
        assert_eq!(config.workspace.version, defaults.workspace.version);
        assert_eq!(
            config.workspace.tmux_session,
            defaults.workspace.tmux_session
        );

        assert_eq!(config.backends, defaults.backends);

        assert!(!config.workflow.auto_commit);
        assert_eq!(
            config.workflow.max_review_iterations,
            defaults.workflow.max_review_iterations
        );
        assert_eq!(
            config.workflow.commit_message_style,
            defaults.workflow.commit_message_style
        );

        assert_eq!(config.templates.planner, "templates/custom-spec.md");
        assert_eq!(config.templates.implementer, defaults.templates.implementer);
        assert_eq!(config.templates.qa, defaults.templates.qa);

        assert_eq!(config.git.base_branch, "main");
        assert_eq!(config.git.auto_branch, defaults.git.auto_branch);
        assert_eq!(config.git.branch_format, defaults.git.branch_format);
    }

    #[test]
    fn missing_single_backend_uses_backend_specific_default() {
        let raw = r#"
[backends.claude]
command = "claude-custom"
"#;

        let defaults = GlobalConfig::default();
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");

        assert_eq!(config.backends.claude.command, "claude-custom");
        assert_eq!(
            config.backends.claude.args,
            defaults.backends.claude.args,
            "present backend block should still receive backend-specific defaults for missing fields"
        );
        assert_eq!(
            config.backends.claude.models, defaults.backends.claude.models,
            "present backend block should inherit default role models"
        );
        assert_eq!(
            config.backends.codex, defaults.backends.codex,
            "missing codex block should deserialize to codex-specific defaults"
        );
    }

    #[test]
    fn default_workspace_tmux_settings_match_expected_values() {
        let config = GlobalConfig::default();
        assert!(!config.workspace.tmux);
        assert_eq!(config.workspace.tmux_session, "ralph");
        assert_eq!(config.workspace.tmux_window_keep_seconds, 5);
        assert_eq!(config.workspace.daemon_poll_seconds, 60);
        assert_eq!(config.workspace.daemon_max_concurrent, 5);
        assert_eq!(
            config.workspace.daemon_labels,
            vec!["ralph:ready".to_owned()]
        );
        assert!(config.workspace.daemon_repo.is_none());
        assert!(config.workspace.daemon_refinement_enabled);
        assert_eq!(config.workspace.daemon_refinement_backend, "claude(sonnet)");
        assert!(config.workflow.qa_enabled);
        assert_eq!(config.workflow.max_qa_iterations, 3);
        assert!(config.workflow.prompt_review_enabled);
        assert_eq!(
            config.workflow.prompt_review_backend,
            "codex(gpt-5.3-codex-xhigh)"
        );
        assert_eq!(
            config.backends.claude.models.qa.as_deref(),
            Some("opus"),
            "claude qa model should default to opus"
        );
        assert_eq!(
            config.backends.codex.models.qa.as_deref(),
            Some("gpt-5.3-codex-high"),
            "codex qa model should default to gpt-5.3-codex-high"
        );
        assert_eq!(config.templates.qa, "templates/qa.md");
        assert_eq!(
            config.templates.prompt_reviewer,
            "templates/prompt_reviewer.md"
        );
    }

    #[test]
    fn backend_role_models_default_is_empty() {
        let models = BackendRoleModels::default();
        assert!(models.planner.is_none());
        assert!(models.implementer.is_none());
        assert!(models.reviewer.is_none());
        assert!(models.qa.is_none());
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
timeout_seconds = 7200

[backends.codex]
command = "codex"
timeout_seconds = 7200

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
completer = "templates/completion.md"

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
        assert_eq!(config.workspace.daemon_poll_seconds, 60);
        assert_eq!(config.workspace.daemon_max_concurrent, 5);
        assert_eq!(
            config.workspace.daemon_labels,
            vec!["ralph:ready".to_owned()]
        );
        assert!(config.workspace.daemon_repo.is_none());
        assert!(config.workspace.daemon_refinement_enabled);
        assert_eq!(config.workspace.daemon_refinement_backend, "claude(sonnet)");
        let defaults = GlobalConfig::default();
        assert_eq!(
            config.backends.claude.models,
            defaults.backends.claude.models
        );
        assert_eq!(config.backends.codex.models, defaults.backends.codex.models);
        assert!(config.workflow.qa_enabled);
        assert_eq!(config.workflow.max_qa_iterations, 3);
        assert!(config.workflow.qa_backend.is_none());
        assert!(config.workflow.prompt_review_enabled);
        assert_eq!(
            config.workflow.prompt_review_backend,
            "codex(gpt-5.3-codex-xhigh)"
        );
        assert_eq!(config.templates.qa, "templates/qa.md");
        assert_eq!(
            config.templates.prompt_reviewer,
            "templates/prompt_reviewer.md"
        );
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
daemon_poll_seconds = 30
daemon_max_concurrent = 2
daemon_labels = ["ralph:ready", "triage"]
daemon_repo = "acme/widgets"
daemon_refinement_enabled = false
daemon_refinement_backend = "codex(gpt-5.3-codex-medium)"

[backends.claude]
command = "claude"
timeout_seconds = 7200

[backends.codex]
command = "codex"
timeout_seconds = 7200

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
completer = "templates/completion.md"

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
        assert_eq!(config.workspace.daemon_poll_seconds, 30);
        assert_eq!(config.workspace.daemon_max_concurrent, 2);
        assert_eq!(
            config.workspace.daemon_labels,
            vec!["ralph:ready".to_owned(), "triage".to_owned()]
        );
        assert_eq!(
            config.workspace.daemon_repo.as_deref(),
            Some("acme/widgets")
        );
        assert!(!config.workspace.daemon_refinement_enabled);
        assert_eq!(
            config.workspace.daemon_refinement_backend,
            "codex(gpt-5.3-codex-medium)"
        );
    }

    #[test]
    fn deserializes_prompt_review_fields_when_present() {
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"

[backends.claude]
command = "claude"
timeout_seconds = 7200

[backends.codex]
command = "codex"
timeout_seconds = 7200

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"
prompt_review_enabled = false
prompt_review_backend = "claude(opus)"

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
prompt_reviewer = "templates/custom-prompt-reviewer.md"
completer = "templates/completion.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
"#;

        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert!(!config.workflow.prompt_review_enabled);
        assert_eq!(config.workflow.prompt_review_backend, "claude(opus)");
        assert_eq!(
            config.templates.prompt_reviewer,
            "templates/custom-prompt-reviewer.md"
        );
    }

    #[test]
    fn deserializes_backend_models_when_present() {
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"

[backends.claude]
command = "claude"
timeout_seconds = 7200

[backends.claude.models]
planner = "opus"
implementer = "opus"
reviewer = "opus"
qa = "opus"
completer = "opus"
acceptance_qa = "opus"
reformatter = "sonnet"

[backends.codex]
command = "codex"
timeout_seconds = 7200

[backends.codex.models]
planner = "gpt-5.3-codex-xhigh"
implementer = "gpt-5.3-codex-high"
reviewer = "gpt-5.3-codex-high"
qa = "gpt-5.3-codex-high"
completer = "gpt-5.3-codex-xhigh"
acceptance_qa = "gpt-5.3-codex-xhigh"
reformatter = "gpt-5.3-codex-medium"

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
completer = "templates/completion.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
"#;

        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert_eq!(
            config.backends.claude.models.planner.as_deref(),
            Some("opus")
        );
        assert_eq!(
            config.backends.claude.models.implementer.as_deref(),
            Some("opus")
        );
        assert_eq!(
            config.backends.claude.models.reviewer.as_deref(),
            Some("opus")
        );
        assert_eq!(config.backends.claude.models.qa.as_deref(), Some("opus"));
        assert_eq!(
            config.backends.claude.models.completer.as_deref(),
            Some("opus")
        );
        assert_eq!(
            config.backends.claude.models.acceptance_qa.as_deref(),
            Some("opus")
        );
        assert_eq!(
            config.backends.claude.models.reformatter.as_deref(),
            Some("sonnet")
        );
        assert_eq!(
            config.backends.codex.models.planner.as_deref(),
            Some("gpt-5.3-codex-xhigh")
        );
        assert_eq!(
            config.backends.codex.models.implementer.as_deref(),
            Some("gpt-5.3-codex-high")
        );
        assert_eq!(
            config.backends.codex.models.reviewer.as_deref(),
            Some("gpt-5.3-codex-high")
        );
        assert_eq!(
            config.backends.codex.models.qa.as_deref(),
            Some("gpt-5.3-codex-high")
        );
        assert_eq!(
            config.backends.codex.models.completer.as_deref(),
            Some("gpt-5.3-codex-xhigh")
        );
        assert_eq!(
            config.backends.codex.models.acceptance_qa.as_deref(),
            Some("gpt-5.3-codex-xhigh")
        );
        assert_eq!(
            config.backends.codex.models.reformatter.as_deref(),
            Some("gpt-5.3-codex-medium")
        );
    }

    #[test]
    fn for_role_returns_expected_model_for_each_role() {
        let models = BackendRoleModels {
            planner: Some("planner-model".to_owned()),
            implementer: Some("implementer-model".to_owned()),
            reviewer: Some("reviewer-model".to_owned()),
            qa: Some("qa-model".to_owned()),
            completer: Some("completer-model".to_owned()),
            acceptance_qa: Some("acceptance-qa-model".to_owned()),
            reformatter: Some("reformatter-model".to_owned()),
        };

        assert_eq!(models.for_role("planner"), Some("planner-model"));
        assert_eq!(models.for_role("implementer"), Some("implementer-model"));
        assert_eq!(models.for_role("reviewer"), Some("reviewer-model"));
        assert_eq!(models.for_role("qa"), Some("qa-model"));
        assert_eq!(models.for_role("completer"), Some("completer-model"));
        assert_eq!(
            models.for_role("acceptance_qa"),
            Some("acceptance-qa-model")
        );
        assert_eq!(models.for_role("reformatter"), Some("reformatter-model"));
        assert_eq!(models.for_role("unknown-role"), None);
    }

    #[test]
    fn fill_from_fills_none_fields_from_defaults() {
        let mut models = BackendRoleModels {
            planner: Some("custom-planner".to_owned()),
            implementer: None,
            reviewer: None,
            qa: None,
            completer: Some("custom-completer".to_owned()),
            acceptance_qa: None,
            reformatter: None,
        };
        let defaults = BackendRoleModels {
            planner: Some("default-planner".to_owned()),
            implementer: Some("default-implementer".to_owned()),
            reviewer: Some("default-reviewer".to_owned()),
            qa: Some("default-qa".to_owned()),
            completer: Some("default-completer".to_owned()),
            acceptance_qa: Some("default-acceptance-qa".to_owned()),
            reformatter: Some("default-reformatter".to_owned()),
        };
        models.fill_from(&defaults);
        assert_eq!(models.planner.as_deref(), Some("custom-planner"));
        assert_eq!(models.implementer.as_deref(), Some("default-implementer"));
        assert_eq!(models.reviewer.as_deref(), Some("default-reviewer"));
        assert_eq!(models.qa.as_deref(), Some("default-qa"));
        assert_eq!(models.completer.as_deref(), Some("custom-completer"));
        assert_eq!(
            models.acceptance_qa.as_deref(),
            Some("default-acceptance-qa")
        );
        assert_eq!(models.reformatter.as_deref(), Some("default-reformatter"));
    }

    #[test]
    fn load_fills_missing_models_from_code_defaults() {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ralph.toml");
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"

[backends.claude]
command = "claude"
timeout_seconds = 7200

[backends.codex]
command = "codex"
timeout_seconds = 7200

[workflow]
max_review_iterations = 5
auto_commit = true
commit_message_style = "conventional"
commit_tag_format = "ralph/{project_id}/loop-{loop_number}"
prompt_change_action = "abort"

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
completer = "templates/completion.md"

[git]
auto_branch = true
branch_format = "ralph/{project_id}"
sign_commits = false
base_branch = "master"
"#;
        let mut f = std::fs::File::create(&path).expect("create file");
        f.write_all(raw.as_bytes()).expect("write file");
        drop(f);

        let config = GlobalConfig::load(&path).expect("load config");
        let defaults = GlobalConfig::default();
        assert_eq!(
            config.backends.claude.models.planner.as_deref(),
            defaults.backends.claude.models.planner.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.planner.as_deref(),
            defaults.backends.codex.models.planner.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.implementer.as_deref(),
            defaults.backends.codex.models.implementer.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.qa.as_deref(),
            defaults.backends.codex.models.qa.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.reformatter.as_deref(),
            defaults.backends.codex.models.reformatter.as_deref(),
        );
    }
}
