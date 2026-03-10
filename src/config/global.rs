use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use clap::ValueEnum;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml_edit::DocumentMut;

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
    #[serde(default)]
    pub amendments: AmendmentsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct AmendmentsConfig {
    #[serde(default)]
    pub unify_final_review: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkspaceConfig {
    #[serde(default = "default_workspace_version")]
    pub version: String,
    #[serde(default = "default_workspace_default_backend")]
    pub default_backend: String,
    #[serde(default = "default_workspace_git_bin")]
    pub git_bin: String,
    #[serde(default = "default_workspace_gh_bin")]
    pub gh_bin: String,
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
    #[serde(default = "default_daemon_auto_rebase_enabled")]
    pub daemon_auto_rebase_enabled: bool,
    #[serde(default = "default_daemon_rebase_interval_seconds")]
    pub daemon_rebase_interval_seconds: u64,
    #[serde(default = "default_daemon_max_rebases_per_cycle")]
    pub daemon_max_rebases_per_cycle: u32,
    #[serde(default = "default_daemon_rebase_timeout_seconds")]
    pub daemon_rebase_timeout_seconds: u64,
    #[serde(default = "default_daemon_rebase_agent_backend")]
    pub daemon_rebase_agent_backend: String,
    #[serde(default = "default_daemon_prd_enabled")]
    pub daemon_prd_enabled: bool,
    #[serde(default = "default_daemon_prd_question_backends")]
    pub daemon_prd_question_backends: Vec<String>,
    #[serde(default = "default_daemon_prd_writer_backend")]
    pub daemon_prd_writer_backend: String,
    #[serde(default = "default_daemon_prd_reviewer_backend")]
    pub daemon_prd_reviewer_backend: String,
    #[serde(default = "default_daemon_prd_max_revisions")]
    pub daemon_prd_max_revisions: u32,
    #[serde(default = "default_daemon_prd_backend_timeout_secs")]
    pub daemon_prd_backend_timeout_secs: u64,
    #[serde(default = "default_daemon_prd_shutdown_timeout_secs")]
    pub daemon_prd_shutdown_timeout_secs: u64,
    /// Maximum backend timeout retries per invocation (default: 3, max: 10).
    #[serde(default)]
    pub daemon_max_backend_retries: Option<u8>,
    #[serde(default)]
    pub daemon_pr_review_whitelist: Vec<String>,
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
    #[serde(
        default = "default_openrouter_backend_config",
        deserialize_with = "deserialize_openrouter_backend_config"
    )]
    pub openrouter: BackendConfig,
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
    pub enabled: BackendEnabled,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub models: BackendRoleModels,
    #[serde(default)]
    pub role_timeouts: RoleTimeouts,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BackendEnabled {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

impl Serialize for BackendEnabled {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Enabled => serializer.serialize_bool(true),
            Self::Disabled => serializer.serialize_bool(false),
        }
    }
}

impl<'de> Deserialize<'de> for BackendEnabled {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BackendEnabledVisitor;

        impl<'de> Visitor<'de> for BackendEnabledVisitor {
            type Value = BackendEnabled;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("true, false, or \"auto\"")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(if value {
                    BackendEnabled::Enabled
                } else {
                    BackendEnabled::Disabled
                })
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "auto" => Ok(BackendEnabled::Auto),
                    _ => Err(E::custom(format!(
                        "invalid backend enabled mode '{value}'; expected true, false, or \"auto\""
                    ))),
                }
            }
        }

        deserializer.deserialize_any(BackendEnabledVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct BackendRoleModels {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub final_reviewer: Option<String>,
    pub arbiter: Option<String>,
    pub qa: Option<String>,
    pub completer: Option<String>,
    pub acceptance_qa: Option<String>,
    pub reformatter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct RoleTimeouts {
    pub planner: Option<u64>,
    pub implementer: Option<u64>,
    pub reviewer: Option<u64>,
    pub final_reviewer: Option<u64>,
    pub arbiter: Option<u64>,
    pub qa: Option<u64>,
    pub completer: Option<u64>,
    pub acceptance_qa: Option<u64>,
    pub reformatter: Option<u64>,
    pub prompt_reviewer: Option<u64>,
}

impl BackendRoleModels {
    pub fn for_role(&self, role: &str) -> Option<&str> {
        match role {
            "planner" => self.planner.as_deref(),
            "implementer" => self.implementer.as_deref(),
            "reviewer" => self.reviewer.as_deref(),
            "final_reviewer" => self.final_reviewer.as_deref(),
            "arbiter" => self.arbiter.as_deref(),
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
        if self.final_reviewer.is_none() {
            self.final_reviewer.clone_from(&defaults.final_reviewer);
        }
        if self.arbiter.is_none() {
            self.arbiter.clone_from(&defaults.arbiter);
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

impl RoleTimeouts {
    pub fn for_role(&self, role: &str) -> Option<u64> {
        match role {
            "planner" => self.planner,
            "implementer" => self.implementer,
            "reviewer" => self.reviewer,
            "final_reviewer" => self.final_reviewer,
            "arbiter" => self.arbiter,
            "qa" => self.qa,
            "completer" => self.completer,
            "acceptance_qa" => self.acceptance_qa,
            "reformatter" => self.reformatter,
            "prompt_reviewer" => self.prompt_reviewer,
            _ => None,
        }
    }

    /// Fill any `None` fields from `defaults`.
    pub fn fill_from(&mut self, defaults: &RoleTimeouts) {
        if self.planner.is_none() {
            self.planner = defaults.planner;
        }
        if self.implementer.is_none() {
            self.implementer = defaults.implementer;
        }
        if self.reviewer.is_none() {
            self.reviewer = defaults.reviewer;
        }
        if self.final_reviewer.is_none() {
            self.final_reviewer = defaults.final_reviewer;
        }
        if self.arbiter.is_none() {
            self.arbiter = defaults.arbiter;
        }
        if self.qa.is_none() {
            self.qa = defaults.qa;
        }
        if self.completer.is_none() {
            self.completer = defaults.completer;
        }
        if self.acceptance_qa.is_none() {
            self.acceptance_qa = defaults.acceptance_qa;
        }
        if self.reformatter.is_none() {
            self.reformatter = defaults.reformatter;
        }
        if self.prompt_reviewer.is_none() {
            self.prompt_reviewer = defaults.prompt_reviewer;
        }
    }
}

impl BackendConfig {
    pub fn timeout_for_role(&self, role: &str) -> Duration {
        match self.role_timeouts.for_role(role) {
            Some(timeout) => Duration::from_secs(timeout),
            None => Duration::from_secs(self.timeout_seconds),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    pub prompt_review_backends: Option<Vec<String>>,
    #[serde(default = "default_prompt_review_min_reviewers")]
    pub prompt_review_min_reviewers: u32,
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
    #[serde(default = "default_final_review_enabled")]
    pub final_review_enabled: bool,
    #[serde(default = "default_final_review_backends")]
    pub final_review_backends: Vec<String>,
    #[serde(default = "default_final_review_arbiter_backend")]
    pub final_review_arbiter_backend: String,
    #[serde(default = "default_final_review_min_reviewers")]
    pub final_review_min_reviewers: u32,
    #[serde(default = "default_final_review_consensus_threshold")]
    pub final_review_consensus_threshold: f64,
    #[serde(default = "default_max_final_review_restarts")]
    pub max_final_review_restarts: u32,
    #[serde(default = "default_completion_backends")]
    pub completion_backends: Vec<String>,
    #[serde(default = "default_completion_min_completers")]
    pub completion_min_completers: u32,
    #[serde(default = "default_completion_consensus_threshold")]
    pub completion_consensus_threshold: f64,
    #[serde(default = "default_qa_enabled")]
    pub qa_enabled: bool,
    #[serde(default = "default_max_qa_iterations")]
    pub max_qa_iterations: u32,
    #[serde(default)]
    pub planner_state_in_prompt: PlannerStateInPrompt,
    #[serde(default)]
    pub planner_previous_specs_in_prompt: PreviousSpecsInPrompt,
    #[serde(default = "default_planner_max_prior_loops")]
    pub planner_max_prior_loops: Option<usize>,
    #[serde(default = "default_max_review_history_entries_in_prompt")]
    pub max_review_history_entries_in_prompt: usize,
    #[serde(default = "default_max_qa_history_entries_in_prompt")]
    pub max_qa_history_entries_in_prompt: usize,
    #[serde(default = "default_include_history_when_session_reuse_enabled")]
    pub include_history_when_session_reuse_enabled: bool,
    #[serde(default)]
    pub session_reuse_enabled: bool,
    #[serde(default = "default_session_reuse_roles")]
    pub session_reuse_roles: Vec<String>,
    #[serde(default = "default_session_reuse_reset_on_prompt_change")]
    pub session_reuse_reset_on_prompt_change: bool,
    #[serde(default = "default_session_reuse_reset_on_rollback")]
    pub session_reuse_reset_on_rollback: bool,
    #[serde(default = "default_pre_commit_fmt")]
    pub pre_commit_fmt: bool,
    #[serde(default = "default_pre_commit_clippy")]
    pub pre_commit_clippy: bool,
    #[serde(default)]
    pub pre_commit_nix_build: bool,
    #[serde(default)]
    pub pre_commit_fmt_auto_fix: bool,
}

impl Eq for WorkflowConfig {}

impl WorkflowConfig {
    pub fn prompt_review_backends_or_default(&self) -> Vec<String> {
        self.prompt_review_backends
            .clone()
            .unwrap_or_else(|| vec![self.prompt_review_backend.clone()])
    }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlannerStateInPrompt {
    FullJson,
    #[default]
    Summary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PreviousSpecsInPrompt {
    None,
    #[default]
    Titles,
    FullText,
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
    #[serde(default = "default_prompt_review_validator_template_path")]
    pub prompt_review_validator: String,
    #[serde(default = "default_completer_template_path")]
    pub completer: String,
    #[serde(default = "default_qa_template_path")]
    pub qa: String,
    #[serde(default = "default_final_reviewer_template_path")]
    pub final_reviewer: String,
    #[serde(default = "default_quick_dev_plan_implement_template_path")]
    pub quick_dev_plan_implement: String,
    #[serde(default = "default_quick_dev_codex_review_template_path")]
    pub quick_dev_codex_review: String,
    #[serde(default = "default_quick_dev_apply_fixes_template_path")]
    pub quick_dev_apply_fixes: String,
    #[serde(default = "default_planner_position_template_path")]
    pub planner_position: String,
    #[serde(default = "default_vote_template_path")]
    pub vote: String,
    #[serde(default = "default_arbiter_template_path")]
    pub arbiter: String,
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
            git_bin: default_workspace_git_bin(),
            gh_bin: default_workspace_gh_bin(),
            tmux: false,
            tmux_session: default_tmux_session(),
            tmux_window_keep_seconds: default_tmux_window_keep_seconds(),
            daemon_poll_seconds: default_daemon_poll_seconds(),
            daemon_max_concurrent: default_daemon_max_concurrent(),
            daemon_labels: default_daemon_labels(),
            daemon_repo: None,
            daemon_refinement_enabled: default_daemon_refinement_enabled(),
            daemon_refinement_backend: default_daemon_refinement_backend(),
            daemon_auto_rebase_enabled: default_daemon_auto_rebase_enabled(),
            daemon_rebase_interval_seconds: default_daemon_rebase_interval_seconds(),
            daemon_max_rebases_per_cycle: default_daemon_max_rebases_per_cycle(),
            daemon_rebase_timeout_seconds: default_daemon_rebase_timeout_seconds(),
            daemon_rebase_agent_backend: default_daemon_rebase_agent_backend(),
            daemon_prd_enabled: default_daemon_prd_enabled(),
            daemon_prd_question_backends: default_daemon_prd_question_backends(),
            daemon_prd_writer_backend: default_daemon_prd_writer_backend(),
            daemon_prd_reviewer_backend: default_daemon_prd_reviewer_backend(),
            daemon_prd_max_revisions: default_daemon_prd_max_revisions(),
            daemon_prd_backend_timeout_secs: default_daemon_prd_backend_timeout_secs(),
            daemon_prd_shutdown_timeout_secs: default_daemon_prd_shutdown_timeout_secs(),
            daemon_max_backend_retries: None,
            daemon_pr_review_whitelist: Vec::new(),
        }
    }
}

impl Default for BackendConfigs {
    fn default() -> Self {
        Self {
            claude: default_claude_backend_config(),
            codex: default_codex_backend_config(),
            openrouter: default_openrouter_backend_config(),
        }
    }
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            command: default_backend_command(),
            args: default_backend_args(),
            timeout_seconds: default_backend_timeout_seconds(),
            enabled: BackendEnabled::default(),
            env: BTreeMap::new(),
            models: BackendRoleModels::default(),
            role_timeouts: RoleTimeouts::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct PartialBackendConfig {
    command: Option<String>,
    args: Option<Vec<String>>,
    timeout_seconds: Option<u64>,
    enabled: Option<BackendEnabled>,
    env: Option<BTreeMap<String, String>>,
    models: Option<BackendRoleModels>,
    role_timeouts: Option<RoleTimeouts>,
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
        if let Some(enabled) = self.enabled {
            defaults.enabled = enabled;
        }
        if let Some(env) = self.env {
            defaults.env = env;
        }
        if let Some(mut models) = self.models {
            models.fill_from(&defaults.models);
            defaults.models = models;
        }
        if let Some(mut role_timeouts) = self.role_timeouts {
            role_timeouts.fill_from(&defaults.role_timeouts);
            defaults.role_timeouts = role_timeouts;
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

fn deserialize_openrouter_backend_config<'de, D>(
    deserializer: D,
) -> std::result::Result<BackendConfig, D::Error>
where
    D: Deserializer<'de>,
{
    let partial = PartialBackendConfig::deserialize(deserializer)?;
    Ok(partial.into_backend_config_with_defaults(default_openrouter_backend_config()))
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
            prompt_review_backends: None,
            prompt_review_min_reviewers: default_prompt_review_min_reviewers(),
            planner_backend: None,
            implementer_backend: None,
            reviewer_backend: None,
            qa_backend: None,
            completer_backend: None,
            final_review_enabled: default_final_review_enabled(),
            final_review_backends: default_final_review_backends(),
            final_review_arbiter_backend: default_final_review_arbiter_backend(),
            final_review_min_reviewers: default_final_review_min_reviewers(),
            final_review_consensus_threshold: default_final_review_consensus_threshold(),
            max_final_review_restarts: default_max_final_review_restarts(),
            completion_backends: default_completion_backends(),
            completion_min_completers: default_completion_min_completers(),
            completion_consensus_threshold: default_completion_consensus_threshold(),
            qa_enabled: default_qa_enabled(),
            max_qa_iterations: default_max_qa_iterations(),
            planner_state_in_prompt: PlannerStateInPrompt::default(),
            planner_previous_specs_in_prompt: PreviousSpecsInPrompt::default(),
            planner_max_prior_loops: default_planner_max_prior_loops(),
            max_review_history_entries_in_prompt: default_max_review_history_entries_in_prompt(),
            max_qa_history_entries_in_prompt: default_max_qa_history_entries_in_prompt(),
            include_history_when_session_reuse_enabled:
                default_include_history_when_session_reuse_enabled(),
            session_reuse_enabled: false,
            session_reuse_roles: default_session_reuse_roles(),
            session_reuse_reset_on_prompt_change: default_session_reuse_reset_on_prompt_change(),
            session_reuse_reset_on_rollback: default_session_reuse_reset_on_rollback(),
            pre_commit_fmt: default_pre_commit_fmt(),
            pre_commit_clippy: default_pre_commit_clippy(),
            pre_commit_nix_build: false,
            pre_commit_fmt_auto_fix: false,
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
            prompt_review_validator: default_prompt_review_validator_template_path(),
            completer: default_completer_template_path(),
            qa: default_qa_template_path(),
            final_reviewer: default_final_reviewer_template_path(),
            quick_dev_plan_implement: default_quick_dev_plan_implement_template_path(),
            quick_dev_codex_review: default_quick_dev_codex_review_template_path(),
            quick_dev_apply_fixes: default_quick_dev_apply_fixes_template_path(),
            planner_position: default_planner_position_template_path(),
            vote: default_vote_template_path(),
            arbiter: default_arbiter_template_path(),
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

fn default_workspace_git_bin() -> String {
    "git".to_owned()
}

fn default_workspace_gh_bin() -> String {
    "gh".to_owned()
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
        enabled: BackendEnabled::Auto,
        env: BTreeMap::new(),
        models: BackendRoleModels {
            planner: Some("opus".to_owned()),
            implementer: Some("opus".to_owned()),
            reviewer: Some("opus".to_owned()),
            final_reviewer: Some("opus".to_owned()),
            arbiter: Some("opus".to_owned()),
            qa: Some("opus".to_owned()),
            completer: Some("opus".to_owned()),
            acceptance_qa: Some("opus".to_owned()),
            reformatter: Some("sonnet".to_owned()),
        },
        role_timeouts: RoleTimeouts::default(),
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
        enabled: BackendEnabled::Auto,
        env: BTreeMap::new(),
        models: BackendRoleModels {
            planner: Some("gpt-5.3-codex-xhigh".to_owned()),
            implementer: Some("gpt-5.3-codex-high".to_owned()),
            reviewer: Some("gpt-5.3-codex-high".to_owned()),
            final_reviewer: Some("gpt-5.3-codex-high".to_owned()),
            arbiter: Some("gpt-5.3-codex-xhigh".to_owned()),
            qa: Some("gpt-5.3-codex-high".to_owned()),
            completer: Some("gpt-5.3-codex-xhigh".to_owned()),
            acceptance_qa: Some("gpt-5.3-codex-xhigh".to_owned()),
            reformatter: Some("gpt-5.3-codex-medium".to_owned()),
        },
        role_timeouts: RoleTimeouts::default(),
    }
}

fn default_openrouter_backend_config() -> BackendConfig {
    BackendConfig {
        command: "goose".to_owned(),
        args: vec![
            "run".to_owned(),
            "--no-profile".to_owned(),
            "--quiet".to_owned(),
            "--with-builtin".to_owned(),
            "developer".to_owned(),
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "-i".to_owned(),
            "-".to_owned(),
        ],
        timeout_seconds: default_backend_timeout_seconds(),
        enabled: BackendEnabled::Disabled,
        env: BTreeMap::new(),
        models: BackendRoleModels::default(),
        role_timeouts: RoleTimeouts::default(),
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

fn default_daemon_auto_rebase_enabled() -> bool {
    true
}

fn default_daemon_rebase_interval_seconds() -> u64 {
    1800
}

fn default_daemon_max_rebases_per_cycle() -> u32 {
    3
}

fn default_daemon_rebase_timeout_seconds() -> u64 {
    120
}

fn default_daemon_rebase_agent_backend() -> String {
    "claude(opus)".to_owned()
}

fn default_daemon_prd_enabled() -> bool {
    true
}

fn default_daemon_prd_question_backends() -> Vec<String> {
    vec!["claude".to_owned(), "codex".to_owned()]
}

fn default_daemon_prd_writer_backend() -> String {
    "claude".to_owned()
}

fn default_daemon_prd_reviewer_backend() -> String {
    "codex".to_owned()
}

fn default_daemon_prd_max_revisions() -> u32 {
    3
}

fn default_daemon_prd_backend_timeout_secs() -> u64 {
    3600
}

fn default_daemon_prd_shutdown_timeout_secs() -> u64 {
    60
}

fn default_planner_max_prior_loops() -> Option<usize> {
    Some(10)
}

fn default_max_review_history_entries_in_prompt() -> usize {
    3
}

fn default_max_qa_history_entries_in_prompt() -> usize {
    2
}

fn default_include_history_when_session_reuse_enabled() -> bool {
    false
}

fn default_session_reuse_roles() -> Vec<String> {
    vec![
        "implementer".to_owned(),
        "reviewer".to_owned(),
        "qa".to_owned(),
        "final_reviewer".to_owned(),
    ]
}

fn default_session_reuse_reset_on_prompt_change() -> bool {
    true
}

fn default_session_reuse_reset_on_rollback() -> bool {
    true
}

fn default_pre_commit_fmt() -> bool {
    false
}

fn default_pre_commit_clippy() -> bool {
    false
}

fn default_qa_enabled() -> bool {
    true
}

fn default_final_review_enabled() -> bool {
    true
}

fn default_final_review_backends() -> Vec<String> {
    vec![
        "claude".to_owned(),
        "codex".to_owned(),
        "?openrouter".to_owned(),
    ]
}

fn default_final_review_arbiter_backend() -> String {
    "claude".to_owned()
}

fn default_final_review_min_reviewers() -> u32 {
    2
}

fn default_final_review_consensus_threshold() -> f64 {
    1.0
}

fn default_max_final_review_restarts() -> u32 {
    25
}

fn default_completion_backends() -> Vec<String> {
    vec![
        "claude".to_owned(),
        "codex".to_owned(),
        "?openrouter".to_owned(),
    ]
}

fn default_completion_min_completers() -> u32 {
    2
}

fn default_completion_consensus_threshold() -> f64 {
    1.0
}

fn default_prompt_review_enabled() -> bool {
    true
}

fn default_prompt_review_backend() -> String {
    "codex(gpt-5.3-codex-xhigh)".to_owned()
}

fn default_prompt_review_min_reviewers() -> u32 {
    1
}

fn default_max_qa_iterations() -> u32 {
    3
}

fn default_prompt_reviewer_template_path() -> String {
    "templates/prompt_reviewer.md".to_owned()
}

fn default_prompt_review_validator_template_path() -> String {
    "templates/prompt_review_validator.md".to_owned()
}

fn default_qa_template_path() -> String {
    "templates/qa.md".to_owned()
}

fn default_final_reviewer_template_path() -> String {
    "templates/final_reviewer.md".to_owned()
}

fn default_quick_dev_plan_implement_template_path() -> String {
    "templates/quick_dev_plan_implement.md".to_owned()
}

fn default_quick_dev_codex_review_template_path() -> String {
    "templates/quick_dev_codex_review.md".to_owned()
}

fn default_quick_dev_apply_fixes_template_path() -> String {
    "templates/quick_dev_apply_fixes.md".to_owned()
}

fn default_planner_position_template_path() -> String {
    "templates/planner_position.md".to_owned()
}

fn default_vote_template_path() -> String {
    "templates/vote.md".to_owned()
}

fn default_arbiter_template_path() -> String {
    "templates/arbiter.md".to_owned()
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
            .claude
            .role_timeouts
            .fill_from(&defaults.backends.claude.role_timeouts);
        config
            .backends
            .codex
            .models
            .fill_from(&defaults.backends.codex.models);
        config
            .backends
            .codex
            .role_timeouts
            .fill_from(&defaults.backends.codex.role_timeouts);
        config
            .backends
            .openrouter
            .models
            .fill_from(&defaults.backends.openrouter.models);
        config
            .backends
            .openrouter
            .role_timeouts
            .fill_from(&defaults.backends.openrouter.role_timeouts);
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
            "openrouter" => Some(&self.backends.openrouter),
            _ => None,
        }
    }

    pub fn backend_config_mut(&mut self, name: &str) -> Option<&mut BackendConfig> {
        match name {
            "claude" => Some(&mut self.backends.claude),
            "codex" => Some(&mut self.backends.codex),
            "openrouter" => Some(&mut self.backends.openrouter),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Sparse in-place TOML persistence
// ---------------------------------------------------------------------------

/// Persist a single config key change in-place using `toml_edit`, preserving
/// comments, formatting, and unknown user keys. The `config` argument is the
/// already-mutated in-memory `GlobalConfig` (after `set_global_config_value`).
///
/// `canonical_key` is the resolved dotted key (e.g. `"workflow.qa_backend"`).
///
/// The function reads the existing file, patches only the targeted key, and
/// writes the result back. If the file does not exist yet it falls back to a
/// full serialization.
pub fn save_sparse(path: &Path, canonical_key: &str, config: &GlobalConfig) -> Result<()> {
    let raw = match fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No existing file — do a full write.
            return config.save(path);
        }
        Err(e) => return Err(e.into()),
    };

    let mut doc: DocumentMut = raw.parse().map_err(|e| {
        crate::error::RalphError::Orchestration(format!("failed to parse ralph.toml: {e}"))
    })?;

    // Serialize the full in-memory config so we can extract the target value.
    let full_raw = toml::to_string_pretty(config)?;
    let full_doc: DocumentMut = full_raw.parse().map_err(|e| {
        crate::error::RalphError::Orchestration(format!("failed to parse serialized config: {e}"))
    })?;

    // Split the canonical key into TOML table path + leaf key.
    let segments = sparse_key_segments(canonical_key);
    let (table_path, leaf) = segments.split_at(segments.len() - 1);

    // Look up the value in the full serialization.
    let target_value = lookup_toml_value(full_doc.as_table(), table_path, leaf[0]);

    match target_value {
        Some(item) => {
            // Ensure intermediate tables exist in the existing doc.
            let table = ensure_tables(&mut doc, table_path)?;
            table.insert(leaf[0], item.clone());
        }
        None => {
            // Value is absent in full serialization → remove from disk (clear semantics).
            if let Some(table) = navigate_tables_mut(&mut doc, table_path)? {
                table.remove(leaf[0]);
            }
        }
    }

    fs::write(path, doc.to_string())?;
    Ok(())
}

/// Split a canonical config key into TOML path segments.
///
/// Special handling:
/// - `backends.<backend>.env.<rest>`: `<rest>` is treated as a single literal
///   key (even if it contains dots), matching the `BTreeMap<String, String>` env
///   field.
/// - All other keys split on dots normally.
fn sparse_key_segments(canonical_key: &str) -> Vec<&str> {
    // Check for the `backends.<name>.env.<rest>` pattern.
    if let Some(rest) = canonical_key.strip_prefix("backends.").and_then(|s| {
        // Find the backend name (first segment after "backends.")
        let dot = s.find('.')?;
        let after_backend = &s[dot + 1..];
        // Check if next segment is "env."
        after_backend
            .strip_prefix("env.")
            .map(|env_rest| (s, dot, env_rest))
    }) {
        let (s, dot, env_rest) = rest;
        let backend_name = &s[..dot];
        return vec!["backends", backend_name, "env", env_rest];
    }

    canonical_key.split('.').collect()
}

/// Look up a value in a TOML table given a table path and leaf key.
fn lookup_toml_value<'a>(
    root: &'a toml_edit::Table,
    table_path: &[&str],
    leaf: &str,
) -> Option<&'a toml_edit::Item> {
    let mut current = root;
    for &segment in table_path {
        current = current.get(segment)?.as_table()?;
    }
    let item = current.get(leaf)?;
    // Treat `Item::None` as absent.
    if item.is_none() {
        return None;
    }
    Some(item)
}

/// Navigate to a nested table in the document (mutable), returning `None` if
/// any segment along the path is missing. Returns an error if a path segment
/// resolves to a non-table value (scalar, array, etc.).
fn navigate_tables_mut<'a>(
    doc: &'a mut DocumentMut,
    path: &[&str],
) -> Result<Option<&'a mut toml_edit::Table>> {
    if path.is_empty() {
        return Ok(Some(doc.as_table_mut()));
    }
    // Use recursive helper to satisfy the borrow checker.
    navigate_table_recursive(doc.as_table_mut(), path)
}

fn navigate_table_recursive<'a>(
    table: &'a mut toml_edit::Table,
    path: &[&str],
) -> Result<Option<&'a mut toml_edit::Table>> {
    if path.is_empty() {
        return Ok(Some(table));
    }
    let segment = path[0];
    let rest = &path[1..];
    let Some(item) = table.get_mut(segment) else {
        return Ok(None);
    };
    // Check what kind of item it is before taking a mutable borrow.
    let is_table = item.as_table().is_some();
    let is_inline_table = item.as_inline_table().is_some();
    if is_inline_table {
        // Convert inline table to a regular table to allow mutable traversal.
        let inline = item.as_inline_table().unwrap().clone();
        let mut regular = inline.into_table();
        regular.set_implicit(true);
        *item = toml_edit::Item::Table(regular);
    }
    if is_table || is_inline_table {
        let t = item.as_table_mut().expect("verified to be a table");
        return navigate_table_recursive(t, rest);
    }
    Err(crate::error::RalphError::Orchestration(format!(
        "config path segment '{segment}' is not a table"
    )))
}

/// Ensure the table hierarchy exists, creating missing intermediate tables.
/// If a path segment is an inline table, it is converted to a regular table
/// preserving existing keys. Returns an error if a path segment is a non-table
/// value (scalar, array, etc.).
fn ensure_tables<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> Result<&'a mut toml_edit::Table> {
    ensure_tables_recursive(doc.as_table_mut(), path)
}

fn ensure_tables_recursive<'a>(
    table: &'a mut toml_edit::Table,
    path: &[&str],
) -> Result<&'a mut toml_edit::Table> {
    if path.is_empty() {
        return Ok(table);
    }
    let segment = path[0];
    let rest = &path[1..];
    if !table.contains_key(segment) {
        table.insert(segment, toml_edit::Item::Table(toml_edit::Table::new()));
    } else {
        let item = table.get_mut(segment).expect("key exists");
        if item.as_table().is_some() {
            // Already a regular table — continue.
        } else if let Some(inline) = item.as_inline_table() {
            // Convert inline table to regular table, preserving existing keys.
            let mut regular = inline.clone().into_table();
            regular.set_implicit(true);
            *item = toml_edit::Item::Table(regular);
        } else {
            return Err(crate::error::RalphError::Orchestration(format!(
                "config path segment '{segment}' is not a table"
            )));
        }
    }
    let child = table
        .get_mut(segment)
        .expect("just ensured")
        .as_table_mut()
        .expect("ensured to be a table");
    ensure_tables_recursive(child, rest)
}

// ---------------------------------------------------------------------------
// Shared global config mutation helpers
// ---------------------------------------------------------------------------

/// Apply a key/value mutation to a `GlobalConfig`, using the same key coverage
/// and validation as `ralph config set --global`.
///
/// This is the single source of truth for global config key mutations, used by
/// both the CLI `config set` path and fast test harness helpers.
pub(crate) fn set_global_config_value(
    config: &mut GlobalConfig,
    key: &str,
    raw_value: &str,
) -> Result<()> {
    match key {
        "workspace.version" => config.workspace.version = raw_value.to_owned(),
        "workspace.default_backend" => {
            cfg_ensure_backend(raw_value)?;
            config.workspace.default_backend = raw_value.to_owned();
        }
        "workspace.git_bin" => {
            config.workspace.git_bin = raw_value.to_owned();
        }
        "workspace.gh_bin" => {
            config.workspace.gh_bin = raw_value.to_owned();
        }
        "workspace.tmux" => {
            config.workspace.tmux = cfg_parse_bool(raw_value, key)?;
        }
        "workspace.tmux_session" => {
            config.workspace.tmux_session = raw_value.to_owned();
        }
        "workspace.tmux_window_keep_seconds" => {
            config.workspace.tmux_window_keep_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "workspace.daemon_poll_seconds" => {
            config.workspace.daemon_poll_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "workspace.daemon_max_concurrent" => {
            config.workspace.daemon_max_concurrent = cfg_parse_u32(raw_value, key)?;
        }
        "workspace.daemon_labels" => {
            config.workspace.daemon_labels = cfg_parse_string_list(raw_value)?;
        }
        "workspace.daemon_repo" => {
            config.workspace.daemon_repo = cfg_parse_optional_string(raw_value);
        }
        "workspace.daemon_refinement_enabled" => {
            config.workspace.daemon_refinement_enabled = cfg_parse_bool(raw_value, key)?;
        }
        "workspace.daemon_refinement_backend" => {
            cfg_ensure_backend(raw_value)?;
            config.workspace.daemon_refinement_backend = raw_value.to_owned();
        }
        "workspace.daemon_auto_rebase_enabled" => {
            config.workspace.daemon_auto_rebase_enabled = cfg_parse_bool(raw_value, key)?;
        }
        "workspace.daemon_rebase_interval_seconds" => {
            config.workspace.daemon_rebase_interval_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "workspace.daemon_max_rebases_per_cycle" => {
            config.workspace.daemon_max_rebases_per_cycle = cfg_parse_u32(raw_value, key)?;
        }
        "workspace.daemon_rebase_timeout_seconds" => {
            config.workspace.daemon_rebase_timeout_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "workspace.daemon_rebase_agent_backend" => {
            crate::daemon::rebase_agent::parse_rebase_agent_backend(raw_value)?;
            config.workspace.daemon_rebase_agent_backend = raw_value.trim().to_owned();
        }
        "workspace.daemon_pr_review_whitelist" => {
            config.workspace.daemon_pr_review_whitelist = cfg_parse_string_list(raw_value)?;
        }
        "workflow.max_review_iterations" => {
            config.workflow.max_review_iterations = cfg_parse_u32(raw_value, key)?;
        }
        "workflow.auto_commit" => {
            config.workflow.auto_commit = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.commit_message_style" => {
            config.workflow.commit_message_style = cfg_parse_commit_message_style(raw_value)?;
        }
        "workflow.commit_tag_format" => {
            config.workflow.commit_tag_format = raw_value.to_owned();
        }
        "workflow.prompt_change_action" => {
            config.workflow.prompt_change_action = cfg_parse_prompt_change_action(raw_value)?;
        }
        "workflow.prompt_review_enabled" => {
            config.workflow.prompt_review_enabled = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.prompt_review_backend" => {
            cfg_ensure_required_backend(raw_value, "workflow.prompt_review_backend")?;
            config.workflow.prompt_review_backend = raw_value.to_owned();
        }
        "workflow.prompt_review_backends" => {
            config.workflow.prompt_review_backends = Some(cfg_parse_string_list(raw_value)?);
        }
        "workflow.prompt_review_min_reviewers" => {
            config.workflow.prompt_review_min_reviewers = cfg_parse_u32(raw_value, key)?;
        }
        "workflow.planner_backend" => {
            config.workflow.planner_backend = cfg_parse_optional_backend(raw_value)?;
        }
        "workflow.implementer_backend" => {
            config.workflow.implementer_backend = cfg_parse_optional_backend(raw_value)?;
        }
        "workflow.reviewer_backend" => {
            config.workflow.reviewer_backend = cfg_parse_optional_backend(raw_value)?;
        }
        "workflow.qa_backend" => {
            config.workflow.qa_backend = cfg_parse_optional_backend(raw_value)?;
        }
        "workflow.completer_backend" => {
            config.workflow.completer_backend = cfg_parse_optional_backend(raw_value)?;
        }
        "workflow.final_review_enabled" => {
            config.workflow.final_review_enabled = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.final_review_backends" => {
            config.workflow.final_review_backends = cfg_parse_string_list(raw_value)?;
        }
        "workflow.final_review_arbiter_backend" => {
            cfg_ensure_backend(raw_value)?;
            config.workflow.final_review_arbiter_backend = raw_value.to_owned();
        }
        "workflow.final_review_min_reviewers" => {
            config.workflow.final_review_min_reviewers = cfg_parse_u32(raw_value, key)?;
        }
        "workflow.final_review_consensus_threshold" => {
            let v: f64 = raw_value.parse().map_err(|_| {
                crate::error::RalphError::Validation(format!("key '{key}' expects float value"))
            })?;
            config.workflow.final_review_consensus_threshold = v;
        }
        "workflow.max_final_review_restarts" => {
            config.workflow.max_final_review_restarts = cfg_parse_u32(raw_value, key)?;
        }
        "workflow.completion_backends" => {
            config.workflow.completion_backends = cfg_parse_string_list(raw_value)?;
        }
        "workflow.completion_min_completers" => {
            config.workflow.completion_min_completers = cfg_parse_u32(raw_value, key)?;
        }
        "workflow.completion_consensus_threshold" => {
            let v: f64 = raw_value.parse().map_err(|_| {
                crate::error::RalphError::Validation(format!("key '{key}' expects float value"))
            })?;
            config.workflow.completion_consensus_threshold = v;
        }
        "workflow.qa_enabled" => {
            config.workflow.qa_enabled = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.max_qa_iterations" => {
            config.workflow.max_qa_iterations = cfg_parse_u32(raw_value, key)?;
        }
        "workflow.planner_state_in_prompt" => {
            config.workflow.planner_state_in_prompt = cfg_parse_planner_state_in_prompt(raw_value)?;
        }
        "workflow.planner_previous_specs_in_prompt" => {
            config.workflow.planner_previous_specs_in_prompt =
                cfg_parse_previous_specs_in_prompt(raw_value)?;
        }
        "workflow.planner_max_prior_loops" => {
            config.workflow.planner_max_prior_loops =
                cfg_parse_optional_usize_or_none(raw_value, key)?;
        }
        "workflow.max_review_history_entries_in_prompt" => {
            config.workflow.max_review_history_entries_in_prompt = cfg_parse_usize(raw_value, key)?;
        }
        "workflow.max_qa_history_entries_in_prompt" => {
            config.workflow.max_qa_history_entries_in_prompt = cfg_parse_usize(raw_value, key)?;
        }
        "workflow.include_history_when_session_reuse_enabled" => {
            config.workflow.include_history_when_session_reuse_enabled =
                cfg_parse_bool(raw_value, key)?;
        }
        "workflow.session_reuse_enabled" => {
            config.workflow.session_reuse_enabled = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.session_reuse_roles" => {
            config.workflow.session_reuse_roles = cfg_parse_session_reuse_roles(raw_value)?;
        }
        "workflow.session_reuse_reset_on_prompt_change" => {
            config.workflow.session_reuse_reset_on_prompt_change = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.session_reuse_reset_on_rollback" => {
            config.workflow.session_reuse_reset_on_rollback = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.pre_commit_fmt" => {
            config.workflow.pre_commit_fmt = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.pre_commit_clippy" => {
            config.workflow.pre_commit_clippy = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.pre_commit_nix_build" => {
            config.workflow.pre_commit_nix_build = cfg_parse_bool(raw_value, key)?;
        }
        "workflow.pre_commit_fmt_auto_fix" => {
            config.workflow.pre_commit_fmt_auto_fix = cfg_parse_bool(raw_value, key)?;
        }
        "templates.planner" => config.templates.planner = raw_value.to_owned(),
        "templates.implementer" => config.templates.implementer = raw_value.to_owned(),
        "templates.reviewer" => config.templates.reviewer = raw_value.to_owned(),
        "templates.prompt_reviewer" => config.templates.prompt_reviewer = raw_value.to_owned(),
        "templates.prompt_review_validator" => {
            config.templates.prompt_review_validator = raw_value.to_owned()
        }
        "templates.completer" => config.templates.completer = raw_value.to_owned(),
        "templates.qa" => config.templates.qa = raw_value.to_owned(),
        "templates.final_reviewer" => config.templates.final_reviewer = raw_value.to_owned(),
        "templates.quick_dev_plan_implement" => {
            config.templates.quick_dev_plan_implement = raw_value.to_owned()
        }
        "templates.quick_dev_codex_review" => {
            config.templates.quick_dev_codex_review = raw_value.to_owned()
        }
        "templates.quick_dev_apply_fixes" => {
            config.templates.quick_dev_apply_fixes = raw_value.to_owned()
        }
        "templates.planner_position" => config.templates.planner_position = raw_value.to_owned(),
        "templates.vote" => config.templates.vote = raw_value.to_owned(),
        "templates.arbiter" => config.templates.arbiter = raw_value.to_owned(),
        "amendments.unify_final_review" => {
            config.amendments.unify_final_review = cfg_parse_bool(raw_value, key)?;
        }
        "git.auto_branch" => config.git.auto_branch = cfg_parse_bool(raw_value, key)?,
        "git.branch_format" => config.git.branch_format = raw_value.to_owned(),
        "git.sign_commits" => config.git.sign_commits = cfg_parse_bool(raw_value, key)?,
        "git.base_branch" => config.git.base_branch = raw_value.to_owned(),
        "backends.claude.command" => config.backends.claude.command = raw_value.to_owned(),
        "backends.codex.command" => config.backends.codex.command = raw_value.to_owned(),
        "backends.openrouter.command" => config.backends.openrouter.command = raw_value.to_owned(),
        "backends.claude.timeout_seconds" => {
            config.backends.claude.timeout_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "backends.codex.timeout_seconds" => {
            config.backends.codex.timeout_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "backends.openrouter.timeout_seconds" => {
            config.backends.openrouter.timeout_seconds = cfg_parse_u64(raw_value, key)?;
        }
        "backends.claude.enabled" => {
            config.backends.claude.enabled = cfg_parse_backend_enabled(raw_value, key)?;
        }
        "backends.codex.enabled" => {
            config.backends.codex.enabled = cfg_parse_backend_enabled(raw_value, key)?;
        }
        "backends.openrouter.enabled" => {
            config.backends.openrouter.enabled = cfg_parse_backend_enabled(raw_value, key)?;
        }
        _ if key.starts_with("backends.claude.role_timeouts.") => {
            let role = key.trim_start_matches("backends.claude.role_timeouts.");
            cfg_set_role_timeout(&mut config.backends.claude.role_timeouts, role, raw_value)?;
        }
        _ if key.starts_with("backends.codex.role_timeouts.") => {
            let role = key.trim_start_matches("backends.codex.role_timeouts.");
            cfg_set_role_timeout(&mut config.backends.codex.role_timeouts, role, raw_value)?;
        }
        _ if key.starts_with("backends.openrouter.role_timeouts.") => {
            let role = key.trim_start_matches("backends.openrouter.role_timeouts.");
            cfg_set_role_timeout(
                &mut config.backends.openrouter.role_timeouts,
                role,
                raw_value,
            )?;
        }
        "backends.claude.args" => config.backends.claude.args = cfg_parse_string_list(raw_value)?,
        "backends.codex.args" => config.backends.codex.args = cfg_parse_string_list(raw_value)?,
        "backends.openrouter.args" => {
            config.backends.openrouter.args = cfg_parse_string_list(raw_value)?
        }
        _ if key.starts_with("backends.claude.models.") => {
            let role = key.trim_start_matches("backends.claude.models.");
            cfg_set_backend_model(&mut config.backends.claude.models, role, raw_value)?;
        }
        _ if key.starts_with("backends.codex.models.") => {
            let role = key.trim_start_matches("backends.codex.models.");
            cfg_set_backend_model(&mut config.backends.codex.models, role, raw_value)?;
        }
        _ if key.starts_with("backends.openrouter.models.") => {
            let role = key.trim_start_matches("backends.openrouter.models.");
            cfg_set_backend_model(&mut config.backends.openrouter.models, role, raw_value)?;
        }
        _ if key.starts_with("backends.claude.env.") => {
            let env_key = key.trim_start_matches("backends.claude.env.");
            config
                .backends
                .claude
                .env
                .insert(env_key.to_owned(), raw_value.to_owned());
        }
        _ if key.starts_with("backends.codex.env.") => {
            let env_key = key.trim_start_matches("backends.codex.env.");
            config
                .backends
                .codex
                .env
                .insert(env_key.to_owned(), raw_value.to_owned());
        }
        _ if key.starts_with("backends.openrouter.env.") => {
            let env_key = key.trim_start_matches("backends.openrouter.env.");
            config
                .backends
                .openrouter
                .env
                .insert(env_key.to_owned(), raw_value.to_owned());
        }
        _ => {
            return Err(crate::error::RalphError::Validation(format!(
                "unsupported global config key: {key}"
            )))
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing helpers used by set_global_config_value
// ---------------------------------------------------------------------------

fn cfg_parse_bool(raw: &str, key: &str) -> Result<bool> {
    raw.parse::<bool>().map_err(|_| {
        crate::error::RalphError::Validation(format!(
            "key '{key}' expects boolean value (true/false)"
        ))
    })
}

fn cfg_parse_u32(raw: &str, key: &str) -> Result<u32> {
    raw.parse::<u32>().map_err(|_| {
        crate::error::RalphError::Validation(format!("key '{key}' expects unsigned integer value"))
    })
}

fn cfg_parse_u64(raw: &str, key: &str) -> Result<u64> {
    raw.parse::<u64>().map_err(|_| {
        crate::error::RalphError::Validation(format!("key '{key}' expects unsigned integer value"))
    })
}

fn cfg_parse_usize(raw: &str, key: &str) -> Result<usize> {
    raw.parse::<usize>().map_err(|_| {
        crate::error::RalphError::Validation(format!("key '{key}' expects unsigned integer value"))
    })
}

fn cfg_parse_backend_enabled(raw: &str, key: &str) -> Result<BackendEnabled> {
    match raw {
        "true" => Ok(BackendEnabled::Enabled),
        "false" => Ok(BackendEnabled::Disabled),
        "auto" => Ok(BackendEnabled::Auto),
        _ => Err(crate::error::RalphError::Validation(format!(
            "key '{key}' expects true, false, or auto"
        ))),
    }
}

fn cfg_parse_commit_message_style(raw: &str) -> Result<CommitMessageStyle> {
    match raw {
        "conventional" => Ok(CommitMessageStyle::Conventional),
        "descriptive" => Ok(CommitMessageStyle::Descriptive),
        "minimal" => Ok(CommitMessageStyle::Minimal),
        _ => Err(crate::error::RalphError::Validation(
            "commit_message_style must be one of: conventional, descriptive, minimal".to_owned(),
        )),
    }
}

fn cfg_parse_prompt_change_action(raw: &str) -> Result<PromptChangeAction> {
    match raw {
        "continue" => Ok(PromptChangeAction::Continue),
        "restart-loop" => Ok(PromptChangeAction::RestartLoop),
        "abort" => Ok(PromptChangeAction::Abort),
        _ => Err(crate::error::RalphError::Validation(
            "prompt_change_action must be one of: continue, restart-loop, abort".to_owned(),
        )),
    }
}

fn cfg_parse_planner_state_in_prompt(raw: &str) -> Result<PlannerStateInPrompt> {
    match raw {
        "full-json" => Ok(PlannerStateInPrompt::FullJson),
        "summary" => Ok(PlannerStateInPrompt::Summary),
        _ => Err(crate::error::RalphError::Validation(
            "planner_state_in_prompt must be one of: full-json, summary".to_owned(),
        )),
    }
}

fn cfg_parse_previous_specs_in_prompt(raw: &str) -> Result<PreviousSpecsInPrompt> {
    match raw {
        "none" => Ok(PreviousSpecsInPrompt::None),
        "titles" => Ok(PreviousSpecsInPrompt::Titles),
        "full-text" => Ok(PreviousSpecsInPrompt::FullText),
        _ => Err(crate::error::RalphError::Validation(
            "planner_previous_specs_in_prompt must be one of: none, titles, full-text".to_owned(),
        )),
    }
}

fn cfg_parse_optional_usize_or_none(raw: &str, key: &str) -> Result<Option<usize>> {
    if raw == "none" {
        return Ok(None);
    }
    let n = raw.parse::<usize>().map_err(|_| {
        crate::error::RalphError::Validation(format!(
            "key '{key}' expects unsigned integer or \"none\" for unlimited"
        ))
    })?;
    Ok(Some(n))
}

fn cfg_parse_optional_backend(raw: &str) -> Result<Option<String>> {
    if raw == "null" {
        return Ok(None);
    }
    cfg_ensure_backend(raw)?;
    Ok(Some(raw.to_owned()))
}

fn cfg_ensure_backend(raw: &str) -> Result<()> {
    crate::cli::backend_spec::validate_backend_spec_name(raw)
}

fn cfg_ensure_required_backend(raw: &str, label: &str) -> Result<()> {
    let parsed = crate::backend::parse_backend_spec(raw)?;
    if parsed.optional {
        return Err(crate::error::RalphError::Validation(format!(
            "optional backend specs (?backend) are not supported for {label}; optional syntax is allowed only in panel backend lists"
        )));
    }
    cfg_ensure_backend(raw)
}

const CFG_KNOWN_ROLES: &[&str] = &[
    "planner",
    "implementer",
    "reviewer",
    "qa",
    "completer",
    "final_reviewer",
];

fn cfg_parse_session_reuse_roles(raw: &str) -> Result<Vec<String>> {
    let roles = cfg_parse_string_list(raw)?;
    for role in &roles {
        if !CFG_KNOWN_ROLES.contains(&role.as_str()) {
            return Err(crate::error::RalphError::Validation(format!(
                "unknown role '{}' in session_reuse_roles; valid roles: {}",
                role,
                CFG_KNOWN_ROLES.join(", ")
            )));
        }
    }
    Ok(roles)
}

fn cfg_parse_optional_string(raw: &str) -> Option<String> {
    if raw == "null" {
        None
    } else {
        Some(raw.to_owned())
    }
}

fn cfg_parse_string_list(raw: &str) -> Result<Vec<String>> {
    if raw.trim().starts_with('[') {
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| {
            crate::error::RalphError::Validation(
                "args must be JSON array (e.g. [\"--flag\"]) or comma-separated list".to_owned(),
            )
        })?;
        let arr = value.as_array().ok_or_else(|| {
            crate::error::RalphError::Validation(
                "args must be JSON array (e.g. [\"--flag\"]) or comma-separated list".to_owned(),
            )
        })?;
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let Some(s) = item.as_str() else {
                return Err(crate::error::RalphError::Validation(
                    "args JSON array items must be strings".to_owned(),
                ));
            };
            out.push(s.to_owned());
        }
        return Ok(out);
    }

    let parts = raw
        .split(',')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_owned())
        .collect::<Vec<_>>();
    Ok(parts)
}

fn cfg_set_backend_model(
    models: &mut BackendRoleModels,
    role: &str,
    raw_value: &str,
) -> Result<()> {
    let value = if raw_value == "null" {
        None
    } else {
        Some(raw_value.to_owned())
    };
    match role {
        "planner" => models.planner = value,
        "implementer" => models.implementer = value,
        "reviewer" => models.reviewer = value,
        "final_reviewer" => models.final_reviewer = value,
        "arbiter" => models.arbiter = value,
        "qa" => models.qa = value,
        "completer" => models.completer = value,
        "acceptance_qa" => models.acceptance_qa = value,
        "reformatter" => models.reformatter = value,
        _ => {
            return Err(crate::error::RalphError::Validation(format!(
                "unknown backend model role: {role}"
            )))
        }
    }
    Ok(())
}

fn cfg_parse_optional_u64(raw: &str, key: &str) -> Result<Option<u64>> {
    if raw == "null" {
        return Ok(None);
    }
    Ok(Some(cfg_parse_u64(raw, key)?))
}

fn cfg_set_role_timeout(
    role_timeouts: &mut RoleTimeouts,
    role: &str,
    raw_value: &str,
) -> Result<()> {
    let parse_key = format!("backends.<backend>.role_timeouts.{role}");
    let value = cfg_parse_optional_u64(raw_value, &parse_key)?;
    match role {
        "planner" => role_timeouts.planner = value,
        "implementer" => role_timeouts.implementer = value,
        "reviewer" => role_timeouts.reviewer = value,
        "final_reviewer" => role_timeouts.final_reviewer = value,
        "arbiter" => role_timeouts.arbiter = value,
        "qa" => role_timeouts.qa = value,
        "completer" => role_timeouts.completer = value,
        "acceptance_qa" => role_timeouts.acceptance_qa = value,
        "reformatter" => role_timeouts.reformatter = value,
        "prompt_reviewer" => role_timeouts.prompt_reviewer = value,
        _ => {
            return Err(crate::error::RalphError::Validation(format!(
                "unknown backend timeout role: {role}"
            )))
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        set_global_config_value, BackendConfig, BackendEnabled, BackendRoleModels, GlobalConfig,
        PartialBackendConfig, PlannerStateInPrompt, PreviousSpecsInPrompt, RoleTimeouts,
    };
    use crate::cli::init::MINIMAL_TOML;

    #[test]
    fn empty_toml_deserializes_to_defaults() {
        let config: GlobalConfig = toml::from_str("").expect("empty TOML should deserialize");
        assert_eq!(config, GlobalConfig::default());
    }

    #[test]
    fn minimal_toml_parses_to_default() {
        let config: GlobalConfig = toml::from_str(MINIMAL_TOML).expect("minimal TOML should parse");
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
        assert_eq!(
            config.templates.quick_dev_plan_implement,
            defaults.templates.quick_dev_plan_implement
        );
        assert_eq!(
            config.templates.quick_dev_codex_review,
            defaults.templates.quick_dev_codex_review
        );
        assert_eq!(
            config.templates.quick_dev_apply_fixes,
            defaults.templates.quick_dev_apply_fixes
        );

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
        assert_eq!(config.workspace.git_bin, "git");
        assert_eq!(config.workspace.gh_bin, "gh");
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
        assert!(config.workspace.daemon_auto_rebase_enabled);
        assert_eq!(config.workspace.daemon_rebase_interval_seconds, 1800);
        assert_eq!(config.workspace.daemon_max_rebases_per_cycle, 3);
        assert_eq!(config.workspace.daemon_rebase_timeout_seconds, 120);
        assert_eq!(config.workspace.daemon_rebase_agent_backend, "claude(opus)");
        assert!(config.workspace.daemon_prd_enabled);
        assert_eq!(
            config.workspace.daemon_prd_question_backends,
            vec!["claude".to_owned(), "codex".to_owned()]
        );
        assert_eq!(config.workspace.daemon_prd_writer_backend, "claude");
        assert_eq!(config.workspace.daemon_prd_reviewer_backend, "codex");
        assert_eq!(config.workspace.daemon_prd_max_revisions, 3);
        assert_eq!(config.workspace.daemon_prd_backend_timeout_secs, 3600);
        assert_eq!(config.workspace.daemon_prd_shutdown_timeout_secs, 60);
        assert!(config.workflow.qa_enabled);
        assert_eq!(config.workflow.max_qa_iterations, 3);
        assert_eq!(config.workflow.max_review_history_entries_in_prompt, 3);
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, 2);
        assert!(!config.workflow.include_history_when_session_reuse_enabled);
        assert!(config.workflow.prompt_review_enabled);
        assert_eq!(
            config.workflow.prompt_review_backend,
            "codex(gpt-5.3-codex-xhigh)"
        );
        assert!(config.workflow.prompt_review_backends.is_none());
        assert_eq!(
            config.workflow.prompt_review_backends_or_default(),
            vec!["codex(gpt-5.3-codex-xhigh)".to_owned()]
        );
        assert_eq!(config.workflow.prompt_review_min_reviewers, 1);
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
        assert_eq!(
            config.templates.prompt_review_validator,
            "templates/prompt_review_validator.md"
        );
        assert_eq!(
            config.templates.final_reviewer,
            "templates/final_reviewer.md"
        );
        assert_eq!(
            config.templates.quick_dev_plan_implement,
            "templates/quick_dev_plan_implement.md"
        );
        assert_eq!(
            config.templates.quick_dev_codex_review,
            "templates/quick_dev_codex_review.md"
        );
        assert_eq!(
            config.templates.quick_dev_apply_fixes,
            "templates/quick_dev_apply_fixes.md"
        );
        assert_eq!(
            config.templates.planner_position,
            "templates/planner_position.md"
        );
        assert_eq!(config.templates.vote, "templates/vote.md");
        assert_eq!(config.templates.arbiter, "templates/arbiter.md");
    }

    #[test]
    fn backend_role_models_default_is_empty() {
        let models = BackendRoleModels::default();
        assert!(models.planner.is_none());
        assert!(models.implementer.is_none());
        assert!(models.reviewer.is_none());
        assert!(models.final_reviewer.is_none());
        assert!(models.arbiter.is_none());
        assert!(models.qa.is_none());
        assert!(models.completer.is_none());
        assert!(models.reformatter.is_none());
    }

    #[test]
    fn openrouter_defaults_match_expected_values() {
        let config = GlobalConfig::default();
        assert_eq!(config.backends.openrouter.command, "goose");
        assert_eq!(
            config.backends.openrouter.args,
            vec![
                "run".to_owned(),
                "--no-profile".to_owned(),
                "--quiet".to_owned(),
                "--with-builtin".to_owned(),
                "developer".to_owned(),
                "--output-format".to_owned(),
                "stream-json".to_owned(),
                "-i".to_owned(),
                "-".to_owned(),
            ]
        );
        assert!(config.backends.openrouter.models.final_reviewer.is_none());
        assert!(config.backends.openrouter.models.arbiter.is_none());
        assert!(config.backends.openrouter.models.completer.is_none());
        assert!(config.backends.openrouter.models.planner.is_none());
        assert_eq!(config.backends.openrouter.enabled, BackendEnabled::Disabled);
        assert_eq!(
            config.workflow.final_review_backends,
            vec![
                "claude".to_owned(),
                "codex".to_owned(),
                "?openrouter".to_owned()
            ]
        );
    }

    #[test]
    fn backend_enabled_accepts_bool_and_auto_string() {
        let enabled_raw = r#"
[backends.openrouter]
enabled = true
"#;
        let enabled_cfg: GlobalConfig =
            toml::from_str(enabled_raw).expect("enabled=true should deserialize");
        assert_eq!(
            enabled_cfg.backends.openrouter.enabled,
            BackendEnabled::Enabled
        );

        let disabled_raw = r#"
[backends.openrouter]
enabled = false
"#;
        let disabled_cfg: GlobalConfig =
            toml::from_str(disabled_raw).expect("enabled=false should deserialize");
        assert_eq!(
            disabled_cfg.backends.openrouter.enabled,
            BackendEnabled::Disabled
        );

        let auto_raw = r#"
[backends.openrouter]
enabled = "auto"
"#;
        let auto_cfg: GlobalConfig =
            toml::from_str(auto_raw).expect("enabled=\"auto\" should deserialize");
        assert_eq!(auto_cfg.backends.openrouter.enabled, BackendEnabled::Auto);
    }

    #[test]
    fn backend_enabled_serde_roundtrip_preserves_values() {
        for (source, expected) in [
            (
                "[backends.openrouter]\nenabled = true\n",
                BackendEnabled::Enabled,
            ),
            (
                "[backends.openrouter]\nenabled = false\n",
                BackendEnabled::Disabled,
            ),
            (
                "[backends.openrouter]\nenabled = \"auto\"\n",
                BackendEnabled::Auto,
            ),
        ] {
            let config: GlobalConfig = toml::from_str(source).expect("deserialize backend enabled");
            assert_eq!(config.backends.openrouter.enabled, expected);
            let encoded = toml::to_string(&config).expect("serialize backend enabled");
            let reparsed: GlobalConfig =
                toml::from_str(&encoded).expect("roundtrip deserialize backend enabled");
            assert_eq!(reparsed.backends.openrouter.enabled, expected);
        }
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
        assert_eq!(config.workspace.git_bin, "git");
        assert_eq!(config.workspace.gh_bin, "gh");
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
        assert!(config.workspace.daemon_auto_rebase_enabled);
        assert_eq!(config.workspace.daemon_rebase_interval_seconds, 1800);
        assert_eq!(config.workspace.daemon_max_rebases_per_cycle, 3);
        assert_eq!(config.workspace.daemon_rebase_timeout_seconds, 120);
        assert_eq!(config.workspace.daemon_rebase_agent_backend, "claude(opus)");
        assert!(config.workspace.daemon_prd_enabled);
        assert_eq!(
            config.workspace.daemon_prd_question_backends,
            vec!["claude".to_owned(), "codex".to_owned()]
        );
        assert_eq!(config.workspace.daemon_prd_writer_backend, "claude");
        assert_eq!(config.workspace.daemon_prd_reviewer_backend, "codex");
        assert_eq!(config.workspace.daemon_prd_max_revisions, 3);
        assert_eq!(config.workspace.daemon_prd_backend_timeout_secs, 3600);
        let defaults = GlobalConfig::default();
        assert_eq!(
            config.backends.claude.models,
            defaults.backends.claude.models
        );
        assert_eq!(config.backends.codex.models, defaults.backends.codex.models);
        assert!(config.workflow.qa_enabled);
        assert_eq!(config.workflow.max_qa_iterations, 3);
        assert!(config.workflow.final_review_enabled);
        assert_eq!(
            config.workflow.final_review_backends,
            vec![
                "claude".to_owned(),
                "codex".to_owned(),
                "?openrouter".to_owned()
            ]
        );
        assert_eq!(config.workflow.final_review_arbiter_backend, "claude");
        assert_eq!(config.workflow.final_review_min_reviewers, 2);
        assert_eq!(config.workflow.final_review_consensus_threshold, 1.0);
        assert_eq!(config.workflow.max_final_review_restarts, 25);
        assert_eq!(config.workflow.max_review_history_entries_in_prompt, 3);
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, 2);
        assert!(!config.workflow.include_history_when_session_reuse_enabled);
        assert!(config.workflow.qa_backend.is_none());
        assert!(config.workflow.prompt_review_enabled);
        assert_eq!(
            config.workflow.prompt_review_backend,
            "codex(gpt-5.3-codex-xhigh)"
        );
        assert!(config.workflow.prompt_review_backends.is_none());
        assert_eq!(
            config.workflow.prompt_review_backends_or_default(),
            vec!["codex(gpt-5.3-codex-xhigh)".to_owned()]
        );
        assert_eq!(config.workflow.prompt_review_min_reviewers, 1);
        assert_eq!(config.templates.qa, "templates/qa.md");
        assert_eq!(
            config.templates.prompt_reviewer,
            "templates/prompt_reviewer.md"
        );
        assert_eq!(
            config.templates.prompt_review_validator,
            "templates/prompt_review_validator.md"
        );
        assert_eq!(
            config.templates.quick_dev_plan_implement,
            "templates/quick_dev_plan_implement.md"
        );
        assert_eq!(
            config.templates.quick_dev_codex_review,
            "templates/quick_dev_codex_review.md"
        );
        assert_eq!(
            config.templates.quick_dev_apply_fixes,
            "templates/quick_dev_apply_fixes.md"
        );
    }

    #[test]
    fn template_config_serde_defaults_quick_dev_fields() {
        let raw = r#"
[templates]
planner = "templates/custom-spec.md"
"#;
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");

        assert_eq!(
            config.templates.quick_dev_plan_implement,
            "templates/quick_dev_plan_implement.md"
        );
        assert_eq!(
            config.templates.quick_dev_codex_review,
            "templates/quick_dev_codex_review.md"
        );
        assert_eq!(
            config.templates.quick_dev_apply_fixes,
            "templates/quick_dev_apply_fixes.md"
        );
    }

    #[test]
    fn deserializes_workspace_tmux_fields_when_present() {
        let raw = r#"
[workspace]
version = "1.0"
default_backend = "claude"
git_bin = "/opt/custom/bin/git"
gh_bin = "/opt/custom/bin/gh"
tmux = true
tmux_session = "demo"
tmux_window_keep_seconds = 10
daemon_poll_seconds = 30
daemon_max_concurrent = 2
daemon_labels = ["ralph:ready", "triage"]
daemon_repo = "acme/widgets"
daemon_refinement_enabled = false
daemon_refinement_backend = "codex(gpt-5.3-codex-medium)"
daemon_auto_rebase_enabled = false
daemon_rebase_interval_seconds = 900
daemon_max_rebases_per_cycle = 5
daemon_rebase_timeout_seconds = 240
daemon_rebase_agent_backend = "none"
daemon_prd_enabled = false
daemon_prd_question_backends = ["claude(opus)", "codex(gpt-5.3-codex-high)"]
daemon_prd_writer_backend = "claude(sonnet)"
daemon_prd_reviewer_backend = "codex(gpt-5.3-codex-medium)"
daemon_prd_max_revisions = 7
daemon_prd_backend_timeout_secs = 300

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
        assert_eq!(config.workspace.git_bin, "/opt/custom/bin/git");
        assert_eq!(config.workspace.gh_bin, "/opt/custom/bin/gh");
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
        assert!(!config.workspace.daemon_auto_rebase_enabled);
        assert_eq!(config.workspace.daemon_rebase_interval_seconds, 900);
        assert_eq!(config.workspace.daemon_max_rebases_per_cycle, 5);
        assert_eq!(config.workspace.daemon_rebase_timeout_seconds, 240);
        assert_eq!(config.workspace.daemon_rebase_agent_backend, "none");
        assert!(!config.workspace.daemon_prd_enabled);
        assert_eq!(
            config.workspace.daemon_prd_question_backends,
            vec![
                "claude(opus)".to_owned(),
                "codex(gpt-5.3-codex-high)".to_owned()
            ]
        );
        assert_eq!(config.workspace.daemon_prd_writer_backend, "claude(sonnet)");
        assert_eq!(
            config.workspace.daemon_prd_reviewer_backend,
            "codex(gpt-5.3-codex-medium)"
        );
        assert_eq!(config.workspace.daemon_prd_max_revisions, 7);
        assert_eq!(config.workspace.daemon_prd_backend_timeout_secs, 300);
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
prompt_review_backends = ["claude(opus)", "codex"]
prompt_review_min_reviewers = 2

[templates]
planner = "templates/spec.md"
implementer = "templates/implementation.md"
reviewer = "templates/review.md"
prompt_reviewer = "templates/custom-prompt-reviewer.md"
prompt_review_validator = "templates/custom-prompt-review-validator.md"
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
            config.workflow.prompt_review_backends,
            Some(vec!["claude(opus)".to_owned(), "codex".to_owned()])
        );
        assert_eq!(config.workflow.prompt_review_min_reviewers, 2);
        assert_eq!(
            config.templates.prompt_reviewer,
            "templates/custom-prompt-reviewer.md"
        );
        assert_eq!(
            config.templates.prompt_review_validator,
            "templates/custom-prompt-review-validator.md"
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
final_reviewer = "opus"
arbiter = "opus"
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
final_reviewer = "gpt-5.3-codex-high"
arbiter = "gpt-5.3-codex-xhigh"
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
        assert_eq!(
            config.backends.claude.models.final_reviewer.as_deref(),
            Some("opus")
        );
        assert_eq!(
            config.backends.claude.models.arbiter.as_deref(),
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
            config.backends.codex.models.final_reviewer.as_deref(),
            Some("gpt-5.3-codex-high")
        );
        assert_eq!(
            config.backends.codex.models.arbiter.as_deref(),
            Some("gpt-5.3-codex-xhigh")
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
            final_reviewer: Some("final-reviewer-model".to_owned()),
            arbiter: Some("arbiter-model".to_owned()),
            qa: Some("qa-model".to_owned()),
            completer: Some("completer-model".to_owned()),
            acceptance_qa: Some("acceptance-qa-model".to_owned()),
            reformatter: Some("reformatter-model".to_owned()),
        };

        assert_eq!(models.for_role("planner"), Some("planner-model"));
        assert_eq!(models.for_role("implementer"), Some("implementer-model"));
        assert_eq!(models.for_role("reviewer"), Some("reviewer-model"));
        assert_eq!(
            models.for_role("final_reviewer"),
            Some("final-reviewer-model")
        );
        assert_eq!(models.for_role("arbiter"), Some("arbiter-model"));
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
            final_reviewer: Some("custom-final-reviewer".to_owned()),
            arbiter: None,
            qa: None,
            completer: Some("custom-completer".to_owned()),
            acceptance_qa: None,
            reformatter: None,
        };
        let defaults = BackendRoleModels {
            planner: Some("default-planner".to_owned()),
            implementer: Some("default-implementer".to_owned()),
            reviewer: Some("default-reviewer".to_owned()),
            final_reviewer: Some("default-final-reviewer".to_owned()),
            arbiter: Some("default-arbiter".to_owned()),
            qa: Some("default-qa".to_owned()),
            completer: Some("default-completer".to_owned()),
            acceptance_qa: Some("default-acceptance-qa".to_owned()),
            reformatter: Some("default-reformatter".to_owned()),
        };
        models.fill_from(&defaults);
        assert_eq!(models.planner.as_deref(), Some("custom-planner"));
        assert_eq!(models.implementer.as_deref(), Some("default-implementer"));
        assert_eq!(models.reviewer.as_deref(), Some("default-reviewer"));
        assert_eq!(
            models.final_reviewer.as_deref(),
            Some("custom-final-reviewer")
        );
        assert_eq!(models.arbiter.as_deref(), Some("default-arbiter"));
        assert_eq!(models.qa.as_deref(), Some("default-qa"));
        assert_eq!(models.completer.as_deref(), Some("custom-completer"));
        assert_eq!(
            models.acceptance_qa.as_deref(),
            Some("default-acceptance-qa")
        );
        assert_eq!(models.reformatter.as_deref(), Some("default-reformatter"));
    }

    #[test]
    fn role_timeouts_for_role_returns_expected_timeout_for_each_role() {
        let role_timeouts = RoleTimeouts {
            planner: Some(10),
            implementer: Some(20),
            reviewer: Some(30),
            final_reviewer: Some(35),
            arbiter: Some(37),
            qa: Some(40),
            completer: Some(50),
            acceptance_qa: Some(60),
            reformatter: Some(70),
            prompt_reviewer: Some(80),
        };

        assert_eq!(role_timeouts.for_role("planner"), Some(10));
        assert_eq!(role_timeouts.for_role("implementer"), Some(20));
        assert_eq!(role_timeouts.for_role("reviewer"), Some(30));
        assert_eq!(role_timeouts.for_role("final_reviewer"), Some(35));
        assert_eq!(role_timeouts.for_role("arbiter"), Some(37));
        assert_eq!(role_timeouts.for_role("qa"), Some(40));
        assert_eq!(role_timeouts.for_role("completer"), Some(50));
        assert_eq!(role_timeouts.for_role("acceptance_qa"), Some(60));
        assert_eq!(role_timeouts.for_role("reformatter"), Some(70));
        assert_eq!(role_timeouts.for_role("prompt_reviewer"), Some(80));
        assert_eq!(role_timeouts.for_role("unknown-role"), None);
    }

    #[test]
    fn role_timeouts_fill_from_fills_none_fields_from_defaults() {
        let mut role_timeouts = RoleTimeouts {
            planner: Some(12),
            implementer: None,
            reviewer: None,
            final_reviewer: Some(38),
            arbiter: None,
            qa: None,
            completer: Some(56),
            acceptance_qa: None,
            reformatter: None,
            prompt_reviewer: None,
        };
        let defaults = RoleTimeouts {
            planner: Some(1),
            implementer: Some(2),
            reviewer: Some(3),
            final_reviewer: Some(4),
            arbiter: Some(5),
            qa: Some(4),
            completer: Some(5),
            acceptance_qa: Some(6),
            reformatter: Some(7),
            prompt_reviewer: Some(8),
        };

        role_timeouts.fill_from(&defaults);
        assert_eq!(role_timeouts.planner, Some(12));
        assert_eq!(role_timeouts.implementer, Some(2));
        assert_eq!(role_timeouts.reviewer, Some(3));
        assert_eq!(role_timeouts.final_reviewer, Some(38));
        assert_eq!(role_timeouts.arbiter, Some(5));
        assert_eq!(role_timeouts.qa, Some(4));
        assert_eq!(role_timeouts.completer, Some(56));
        assert_eq!(role_timeouts.acceptance_qa, Some(6));
        assert_eq!(role_timeouts.reformatter, Some(7));
        assert_eq!(role_timeouts.prompt_reviewer, Some(8));
    }

    #[test]
    fn backend_config_timeout_for_role_prefers_override_and_falls_back() {
        let config = BackendConfig {
            timeout_seconds: 77,
            role_timeouts: RoleTimeouts {
                planner: Some(33),
                ..RoleTimeouts::default()
            },
            ..BackendConfig::default()
        };

        assert_eq!(config.timeout_for_role("planner").as_secs(), 33);
        assert_eq!(config.timeout_for_role("qa").as_secs(), 77);
        assert_eq!(config.timeout_for_role("unknown").as_secs(), 77);
    }

    #[test]
    fn backend_role_timeouts_toml_deserialize_without_table_uses_fallback_timeout() {
        let raw = r#"
[backends.claude]
command = "claude-custom"
timeout_seconds = 91
"#;
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert_eq!(
            config.backends.claude.timeout_for_role("planner").as_secs(),
            91
        );
        assert_eq!(
            config.backends.claude.role_timeouts,
            RoleTimeouts::default()
        );
    }

    #[test]
    fn backend_role_timeouts_toml_deserialize_with_table_sets_overrides() {
        let raw = r#"
[backends.claude]
command = "claude-custom"
timeout_seconds = 91

[backends.claude.role_timeouts]
planner = 12
prompt_reviewer = 34
"#;
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert_eq!(
            config.backends.claude.timeout_for_role("planner").as_secs(),
            12
        );
        assert_eq!(
            config
                .backends
                .claude
                .timeout_for_role("prompt_reviewer")
                .as_secs(),
            34
        );
        assert_eq!(config.backends.claude.timeout_for_role("qa").as_secs(), 91);
    }

    #[test]
    fn partial_backend_config_merge_fills_role_timeouts_from_defaults() {
        let partial = PartialBackendConfig {
            role_timeouts: Some(RoleTimeouts {
                planner: Some(10),
                qa: Some(30),
                ..RoleTimeouts::default()
            }),
            ..PartialBackendConfig::default()
        };
        let defaults = BackendConfig {
            role_timeouts: RoleTimeouts {
                planner: Some(99),
                implementer: Some(20),
                reviewer: Some(21),
                final_reviewer: Some(22),
                arbiter: Some(23),
                qa: Some(22),
                completer: Some(24),
                acceptance_qa: Some(25),
                reformatter: Some(26),
                prompt_reviewer: Some(27),
            },
            ..BackendConfig::default()
        };

        let merged = partial.into_backend_config_with_defaults(defaults);
        assert_eq!(merged.role_timeouts.planner, Some(10));
        assert_eq!(merged.role_timeouts.implementer, Some(20));
        assert_eq!(merged.role_timeouts.reviewer, Some(21));
        assert_eq!(merged.role_timeouts.final_reviewer, Some(22));
        assert_eq!(merged.role_timeouts.arbiter, Some(23));
        assert_eq!(merged.role_timeouts.qa, Some(30));
        assert_eq!(merged.role_timeouts.completer, Some(24));
        assert_eq!(merged.role_timeouts.acceptance_qa, Some(25));
        assert_eq!(merged.role_timeouts.reformatter, Some(26));
        assert_eq!(merged.role_timeouts.prompt_reviewer, Some(27));
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
            config.backends.codex.models.final_reviewer.as_deref(),
            defaults.backends.codex.models.final_reviewer.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.arbiter.as_deref(),
            defaults.backends.codex.models.arbiter.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.qa.as_deref(),
            defaults.backends.codex.models.qa.as_deref(),
        );
        assert_eq!(
            config.backends.codex.models.reformatter.as_deref(),
            defaults.backends.codex.models.reformatter.as_deref(),
        );
        assert_eq!(
            config.backends.openrouter.models.final_reviewer.as_deref(),
            defaults
                .backends
                .openrouter
                .models
                .final_reviewer
                .as_deref(),
        );
        assert_eq!(
            config.backends.openrouter.models.arbiter.as_deref(),
            defaults.backends.openrouter.models.arbiter.as_deref(),
        );
        assert_eq!(
            config.backends.openrouter.models.completer.as_deref(),
            defaults.backends.openrouter.models.completer.as_deref(),
        );
    }

    #[test]
    fn planner_state_in_prompt_default_is_summary() {
        assert_eq!(
            PlannerStateInPrompt::default(),
            PlannerStateInPrompt::Summary
        );
    }

    #[test]
    fn previous_specs_in_prompt_default_is_titles() {
        assert_eq!(
            PreviousSpecsInPrompt::default(),
            PreviousSpecsInPrompt::Titles
        );
    }

    #[test]
    fn planner_state_in_prompt_serde_roundtrips() {
        let variants = [
            PlannerStateInPrompt::FullJson,
            PlannerStateInPrompt::Summary,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize");
            let parsed: PlannerStateInPrompt = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, variant, "roundtrip failed for {json}");
        }
    }

    #[test]
    fn previous_specs_in_prompt_serde_roundtrips() {
        let variants = [
            PreviousSpecsInPrompt::None,
            PreviousSpecsInPrompt::Titles,
            PreviousSpecsInPrompt::FullText,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).expect("serialize");
            let parsed: PreviousSpecsInPrompt = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, variant, "roundtrip failed for {json}");
        }
    }

    #[test]
    fn planner_state_in_prompt_kebab_case_serde() {
        let json = serde_json::to_string(&PlannerStateInPrompt::FullJson).expect("serialize");
        assert_eq!(json, "\"full-json\"");

        let json = serde_json::to_string(&PlannerStateInPrompt::Summary).expect("serialize");
        assert_eq!(json, "\"summary\"");
    }

    #[test]
    fn previous_specs_in_prompt_kebab_case_serde() {
        let json = serde_json::to_string(&PreviousSpecsInPrompt::None).expect("serialize");
        assert_eq!(json, "\"none\"");

        let json = serde_json::to_string(&PreviousSpecsInPrompt::Titles).expect("serialize");
        assert_eq!(json, "\"titles\"");

        let json = serde_json::to_string(&PreviousSpecsInPrompt::FullText).expect("serialize");
        assert_eq!(json, "\"full-text\"");
    }

    #[test]
    fn workflow_config_default_planner_compression_fields() {
        let config = GlobalConfig::default();
        assert_eq!(
            config.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::Summary
        );
        assert_eq!(
            config.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::Titles
        );
        assert_eq!(config.workflow.planner_max_prior_loops, Some(10));
        assert_eq!(config.workflow.max_review_history_entries_in_prompt, 3);
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, 2);
        assert!(!config.workflow.include_history_when_session_reuse_enabled);
    }

    #[test]
    fn workflow_config_deserializes_planner_compression_fields() {
        let raw = r#"
[workflow]
planner_state_in_prompt = "full-json"
planner_previous_specs_in_prompt = "full-text"
planner_max_prior_loops = 5
max_review_history_entries_in_prompt = 4
max_qa_history_entries_in_prompt = 6
include_history_when_session_reuse_enabled = true
"#;
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert_eq!(
            config.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::FullJson
        );
        assert_eq!(
            config.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::FullText
        );
        assert_eq!(config.workflow.planner_max_prior_loops, Some(5));
        assert_eq!(config.workflow.max_review_history_entries_in_prompt, 4);
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, 6);
        assert!(config.workflow.include_history_when_session_reuse_enabled);
    }

    #[test]
    fn workflow_config_deserializes_planner_max_prior_loops_absent_uses_default() {
        let raw = r#"
[workflow]
planner_state_in_prompt = "summary"
"#;
        let config: GlobalConfig = toml::from_str(raw).expect("config should deserialize");
        assert_eq!(config.workflow.planner_max_prior_loops, Some(10));
        assert_eq!(config.workflow.max_review_history_entries_in_prompt, 3);
        assert_eq!(config.workflow.max_qa_history_entries_in_prompt, 2);
        assert!(!config.workflow.include_history_when_session_reuse_enabled);
    }

    // ── Shared global config mutator parity tests ──────────────────────

    #[test]
    fn shared_mutator_sets_workspace_fields() {
        let mut config = GlobalConfig::default();

        set_global_config_value(&mut config, "workspace.default_backend", "codex")
            .expect("set default_backend");
        assert_eq!(config.workspace.default_backend, "codex");

        set_global_config_value(&mut config, "workspace.git_bin", "/usr/local/bin/git")
            .expect("set git_bin");
        assert_eq!(config.workspace.git_bin, "/usr/local/bin/git");

        set_global_config_value(&mut config, "workspace.gh_bin", "/usr/local/bin/gh")
            .expect("set gh_bin");
        assert_eq!(config.workspace.gh_bin, "/usr/local/bin/gh");

        set_global_config_value(&mut config, "workspace.tmux", "true").expect("set tmux");
        assert!(config.workspace.tmux);

        set_global_config_value(&mut config, "workspace.tmux_session", "custom-session")
            .expect("set tmux_session");
        assert_eq!(config.workspace.tmux_session, "custom-session");

        set_global_config_value(&mut config, "workspace.tmux_window_keep_seconds", "30")
            .expect("set tmux_window_keep_seconds");
        assert_eq!(config.workspace.tmux_window_keep_seconds, 30);

        set_global_config_value(&mut config, "workspace.daemon_poll_seconds", "120")
            .expect("set daemon_poll_seconds");
        assert_eq!(config.workspace.daemon_poll_seconds, 120);

        set_global_config_value(&mut config, "workspace.daemon_max_concurrent", "8")
            .expect("set daemon_max_concurrent");
        assert_eq!(config.workspace.daemon_max_concurrent, 8);

        set_global_config_value(
            &mut config,
            "workspace.daemon_labels",
            "[\"ralph:ready\",\"deploy\"]",
        )
        .expect("set daemon_labels");
        assert_eq!(
            config.workspace.daemon_labels,
            vec!["ralph:ready".to_owned(), "deploy".to_owned()]
        );

        set_global_config_value(&mut config, "workspace.daemon_repo", "acme/repo")
            .expect("set daemon_repo");
        assert_eq!(config.workspace.daemon_repo.as_deref(), Some("acme/repo"));

        set_global_config_value(&mut config, "workspace.daemon_repo", "null")
            .expect("clear daemon_repo");
        assert!(config.workspace.daemon_repo.is_none());
    }

    #[test]
    fn shared_mutator_sets_workflow_fields() {
        let mut config = GlobalConfig::default();

        set_global_config_value(&mut config, "workflow.max_review_iterations", "7")
            .expect("set max_review_iterations");
        assert_eq!(config.workflow.max_review_iterations, 7);

        set_global_config_value(&mut config, "workflow.auto_commit", "false")
            .expect("set auto_commit");
        assert!(!config.workflow.auto_commit);

        set_global_config_value(&mut config, "workflow.commit_message_style", "minimal")
            .expect("set commit_message_style");
        assert_eq!(
            config.workflow.commit_message_style,
            super::CommitMessageStyle::Minimal
        );

        set_global_config_value(&mut config, "workflow.prompt_change_action", "restart-loop")
            .expect("set prompt_change_action");
        assert_eq!(
            config.workflow.prompt_change_action,
            super::PromptChangeAction::RestartLoop
        );

        set_global_config_value(&mut config, "workflow.qa_enabled", "false")
            .expect("set qa_enabled");
        assert!(!config.workflow.qa_enabled);

        set_global_config_value(&mut config, "workflow.max_qa_iterations", "5")
            .expect("set max_qa_iterations");
        assert_eq!(config.workflow.max_qa_iterations, 5);

        set_global_config_value(&mut config, "workflow.planner_state_in_prompt", "full-json")
            .expect("set planner_state_in_prompt");
        assert_eq!(
            config.workflow.planner_state_in_prompt,
            PlannerStateInPrompt::FullJson
        );

        set_global_config_value(
            &mut config,
            "workflow.planner_previous_specs_in_prompt",
            "full-text",
        )
        .expect("set planner_previous_specs_in_prompt");
        assert_eq!(
            config.workflow.planner_previous_specs_in_prompt,
            PreviousSpecsInPrompt::FullText
        );

        set_global_config_value(&mut config, "workflow.planner_max_prior_loops", "5")
            .expect("set planner_max_prior_loops");
        assert_eq!(config.workflow.planner_max_prior_loops, Some(5));

        set_global_config_value(&mut config, "workflow.planner_max_prior_loops", "none")
            .expect("set planner_max_prior_loops to none (unlimited)");
        assert_eq!(config.workflow.planner_max_prior_loops, None);

        set_global_config_value(&mut config, "workflow.session_reuse_enabled", "true")
            .expect("set session_reuse_enabled");
        assert!(config.workflow.session_reuse_enabled);

        set_global_config_value(
            &mut config,
            "workflow.session_reuse_roles",
            "[\"planner\",\"implementer\"]",
        )
        .expect("set session_reuse_roles");
        assert_eq!(
            config.workflow.session_reuse_roles,
            vec!["planner".to_owned(), "implementer".to_owned()]
        );
    }

    #[test]
    fn shared_mutator_sets_backend_fields() {
        let mut config = GlobalConfig::default();

        set_global_config_value(
            &mut config,
            "backends.claude.command",
            "/usr/local/bin/claude",
        )
        .expect("set claude command");
        assert_eq!(config.backends.claude.command, "/usr/local/bin/claude");

        set_global_config_value(&mut config, "backends.claude.timeout_seconds", "3600")
            .expect("set claude timeout");
        assert_eq!(config.backends.claude.timeout_seconds, 3600);

        set_global_config_value(&mut config, "backends.codex.args", "[\"--flag\",\"value\"]")
            .expect("set codex args");
        assert_eq!(
            config.backends.codex.args,
            vec!["--flag".to_owned(), "value".to_owned()]
        );

        set_global_config_value(&mut config, "backends.openrouter.enabled", "false")
            .expect("set openrouter enabled");
        assert_eq!(config.backends.openrouter.enabled, BackendEnabled::Disabled);

        set_global_config_value(&mut config, "backends.openrouter.enabled", "auto")
            .expect("set openrouter enabled auto");
        assert_eq!(config.backends.openrouter.enabled, BackendEnabled::Auto);

        set_global_config_value(&mut config, "backends.claude.models.planner", "sonnet")
            .expect("set claude planner model");
        assert_eq!(
            config.backends.claude.models.planner.as_deref(),
            Some("sonnet")
        );

        set_global_config_value(&mut config, "backends.claude.models.planner", "null")
            .expect("clear claude planner model");
        assert!(config.backends.claude.models.planner.is_none());

        set_global_config_value(&mut config, "backends.claude.role_timeouts.planner", "120")
            .expect("set claude planner timeout");
        assert_eq!(config.backends.claude.role_timeouts.planner, Some(120));

        set_global_config_value(&mut config, "backends.claude.role_timeouts.planner", "null")
            .expect("clear claude planner timeout");
        assert!(config.backends.claude.role_timeouts.planner.is_none());
    }

    #[test]
    fn shared_mutator_sets_template_and_git_fields() {
        let mut config = GlobalConfig::default();

        set_global_config_value(&mut config, "templates.planner", "custom/spec.md")
            .expect("set templates.planner");
        assert_eq!(config.templates.planner, "custom/spec.md");

        set_global_config_value(&mut config, "templates.qa", "custom/qa.md")
            .expect("set templates.qa");
        assert_eq!(config.templates.qa, "custom/qa.md");

        set_global_config_value(
            &mut config,
            "templates.quick_dev_codex_review",
            "custom/quick-codex-review.md",
        )
        .expect("set templates.quick_dev_codex_review");
        assert_eq!(
            config.templates.quick_dev_codex_review,
            "custom/quick-codex-review.md"
        );

        set_global_config_value(&mut config, "git.base_branch", "main")
            .expect("set git.base_branch");
        assert_eq!(config.git.base_branch, "main");

        set_global_config_value(&mut config, "git.auto_branch", "false")
            .expect("set git.auto_branch");
        assert!(!config.git.auto_branch);

        set_global_config_value(&mut config, "git.sign_commits", "true")
            .expect("set git.sign_commits");
        assert!(config.git.sign_commits);

        set_global_config_value(&mut config, "git.branch_format", "feature/{project_id}")
            .expect("set git.branch_format");
        assert_eq!(config.git.branch_format, "feature/{project_id}");
    }

    #[test]
    fn shared_mutator_rejects_unknown_key() {
        let mut config = GlobalConfig::default();
        let result = set_global_config_value(&mut config, "nonexistent.key", "value");
        assert!(result.is_err());
    }

    #[test]
    fn shared_mutator_rejects_invalid_bool() {
        let mut config = GlobalConfig::default();
        let result = set_global_config_value(&mut config, "workflow.auto_commit", "maybe");
        assert!(result.is_err());
    }

    #[test]
    fn shared_mutator_rejects_invalid_integer() {
        let mut config = GlobalConfig::default();
        let result = set_global_config_value(&mut config, "workflow.max_review_iterations", "abc");
        assert!(result.is_err());
    }

    #[test]
    fn shared_mutator_rejects_daemon_prd_keys() {
        // These keys were not supported by `ralph config set --global` before
        // the shared-mutator refactor and must remain unsupported to preserve
        // CLI key-coverage parity.
        let unsupported = [
            "workspace.daemon_prd_enabled",
            "workspace.daemon_prd_question_backends",
            "workspace.daemon_prd_writer_backend",
            "workspace.daemon_prd_reviewer_backend",
            "workspace.daemon_prd_max_revisions",
            "workspace.daemon_prd_backend_timeout_secs",
        ];
        for key in &unsupported {
            let mut config = GlobalConfig::default();
            let result = set_global_config_value(&mut config, key, "test");
            assert!(
                result.is_err(),
                "key '{key}' should be rejected by the shared mutator"
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported global config key"),
                "key '{key}' should produce 'unsupported global config key' error"
            );
        }
    }

    // -----------------------------------------------------------------------
    // save_sparse unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn save_sparse_preserves_comments_and_unrelated_formatting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        let original = "# My workspace\n[workspace]\nversion = \"1.0\"\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.workspace.default_backend = "codex".to_owned();
        super::save_sparse(&path, "workspace.default_backend", &config).expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            result.contains("# My workspace"),
            "comment should be preserved, got:\n{result}"
        );
        assert!(
            result.contains("version = \"1.0\""),
            "unrelated key should be preserved, got:\n{result}"
        );
        assert!(
            result.contains("default_backend"),
            "new key should be written, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_creates_intermediate_tables() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        std::fs::write(&path, "[workspace]\nversion = \"1.0\"\n").expect("write");

        let mut config = GlobalConfig::default();
        config.workflow.auto_commit = false;
        super::save_sparse(&path, "workflow.auto_commit", &config).expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            result.contains("[workflow]"),
            "workflow table should be created, got:\n{result}"
        );
        assert!(
            result.contains("auto_commit = false"),
            "auto_commit should be set, got:\n{result}"
        );
        // Original content preserved.
        assert!(
            result.contains("version = \"1.0\""),
            "existing key should remain, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_removes_optional_key_on_null() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        let original = "[workflow]\nqa_backend = \"codex\"\nauto_commit = true\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.workflow.qa_backend = None; // cleared
        super::save_sparse(&path, "workflow.qa_backend", &config).expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            !result.contains("qa_backend"),
            "qa_backend should be removed, got:\n{result}"
        );
        assert!(
            result.contains("auto_commit = true"),
            "other keys should be preserved, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_handles_env_dotted_literal_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        std::fs::write(&path, "[workspace]\n").expect("write");

        let mut config = GlobalConfig::default();
        config
            .backends
            .claude
            .env
            .insert("MY.DOTTED.KEY".to_owned(), "value123".to_owned());
        super::save_sparse(&path, "backends.claude.env.MY.DOTTED.KEY", &config)
            .expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            result.contains("\"MY.DOTTED.KEY\"") || result.contains("MY.DOTTED.KEY"),
            "dotted env key should appear as literal key, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_handles_models_role_clear() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        let original = "[backends.claude.models]\nplanner = \"opus\"\nreviewer = \"sonnet\"\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.backends.claude.models.planner = None;
        super::save_sparse(&path, "backends.claude.models.planner", &config).expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        // planner should be removed since it's None in the full serialization
        // (the full serialization doesn't include None optional fields).
        assert!(
            !result.contains("planner"),
            "planner model should be removed, got:\n{result}"
        );
        assert!(
            result.contains("reviewer = \"sonnet\""),
            "other models should be preserved, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_handles_role_timeouts_clear() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        let original = "[backends.claude.role_timeouts]\nplanner = 42\nreviewer = 99\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.backends.claude.role_timeouts.planner = None;
        super::save_sparse(&path, "backends.claude.role_timeouts.planner", &config)
            .expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            !result.contains("planner"),
            "planner timeout should be removed, got:\n{result}"
        );
        assert!(
            result.contains("reviewer = 99"),
            "other timeouts should be preserved, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_falls_back_to_full_save_on_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");

        let config = GlobalConfig::default();
        super::save_sparse(&path, "workspace.version", &config).expect("save_sparse");

        let result = std::fs::read_to_string(&path).expect("read");
        let parsed: GlobalConfig = toml::from_str(&result).expect("parse");
        assert_eq!(parsed, GlobalConfig::default());
    }

    #[test]
    fn sparse_key_segments_splits_normal_keys() {
        assert_eq!(
            super::sparse_key_segments("workspace.default_backend"),
            vec!["workspace", "default_backend"]
        );
        assert_eq!(
            super::sparse_key_segments("workflow.auto_commit"),
            vec!["workflow", "auto_commit"]
        );
    }

    #[test]
    fn sparse_key_segments_treats_env_rest_as_literal() {
        assert_eq!(
            super::sparse_key_segments("backends.claude.env.MY.DOTTED.KEY"),
            vec!["backends", "claude", "env", "MY.DOTTED.KEY"]
        );
        assert_eq!(
            super::sparse_key_segments("backends.codex.env.SIMPLE"),
            vec!["backends", "codex", "env", "SIMPLE"]
        );
    }

    #[test]
    fn sparse_key_segments_splits_models_and_timeouts_normally() {
        assert_eq!(
            super::sparse_key_segments("backends.claude.models.planner"),
            vec!["backends", "claude", "models", "planner"]
        );
        assert_eq!(
            super::sparse_key_segments("backends.codex.role_timeouts.qa"),
            vec!["backends", "codex", "role_timeouts", "qa"]
        );
    }

    // -----------------------------------------------------------------------
    // Inline-table regression tests for save_sparse
    // -----------------------------------------------------------------------

    #[test]
    fn save_sparse_inline_table_path_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        // Use inline-table syntax for workspace.
        let original = "workspace = { version = \"1.0\" }\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.workspace.default_backend = "codex".to_owned();
        super::save_sparse(&path, "workspace.default_backend", &config)
            .expect("save_sparse should handle inline table path");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            result.contains("default_backend"),
            "default_backend should be written through inline table, got:\n{result}"
        );
        // The file should still be valid TOML.
        let parsed: GlobalConfig = toml::from_str(&result).expect("should parse");
        assert_eq!(parsed.workspace.default_backend, "codex");
        assert_eq!(parsed.workspace.version, "1.0");
    }

    #[test]
    fn save_sparse_inline_table_path_clear() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        // Inline table with an optional key.
        let original = "[workflow]\nqa_backend = \"codex\"\nauto_commit = true\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.workflow.qa_backend = None;
        super::save_sparse(&path, "workflow.qa_backend", &config)
            .expect("save_sparse should handle clear");

        let result = std::fs::read_to_string(&path).expect("read");
        assert!(
            !result.contains("qa_backend"),
            "qa_backend should be removed, got:\n{result}"
        );
        assert!(
            result.contains("auto_commit"),
            "other keys should be preserved, got:\n{result}"
        );
    }

    #[test]
    fn save_sparse_errors_on_non_table_path_segment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        // `workspace` is a string, not a table — traversal should fail.
        let original = "workspace = \"not-a-table\"\n";
        std::fs::write(&path, original).expect("write");

        let config = GlobalConfig::default();
        let err = super::save_sparse(&path, "workspace.version", &config);
        assert!(
            err.is_err(),
            "save_sparse should error on non-table path segment"
        );
        let err_msg = err.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a table"),
            "error should mention 'not a table', got: {err_msg}"
        );

        // File should be unchanged.
        let result = std::fs::read_to_string(&path).expect("read");
        assert_eq!(result, original, "file should be untouched on error");
    }

    #[test]
    fn set_global_config_value_daemon_pr_review_whitelist_roundtrip() {
        let mut config = GlobalConfig::default();
        assert!(config.workspace.daemon_pr_review_whitelist.is_empty());

        set_global_config_value(
            &mut config,
            "workspace.daemon_pr_review_whitelist",
            r#"["alice","bob"]"#,
        )
        .expect("set whitelist should succeed");

        assert_eq!(
            config.workspace.daemon_pr_review_whitelist,
            vec!["alice".to_owned(), "bob".to_owned()]
        );

        // Roundtrip through TOML serialization.
        let toml_str = toml::to_string(&config).expect("serialize");
        let reparsed: GlobalConfig = toml::from_str(&toml_str).expect("deserialize roundtrip");
        assert_eq!(
            reparsed.workspace.daemon_pr_review_whitelist,
            vec!["alice".to_owned(), "bob".to_owned()]
        );

        // Setting to empty list should clear.
        set_global_config_value(&mut config, "workspace.daemon_pr_review_whitelist", "[]")
            .expect("set empty whitelist should succeed");
        assert!(config.workspace.daemon_pr_review_whitelist.is_empty());
    }

    #[test]
    fn save_sparse_inline_table_nested_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ralph.toml");
        // Nested inline tables.
        let original = "workspace = { version = \"1.0\" }\nworkflow = { auto_commit = true }\n";
        std::fs::write(&path, original).expect("write");

        let mut config = GlobalConfig::default();
        config.workflow.qa_backend = Some("codex".to_owned());
        super::save_sparse(&path, "workflow.qa_backend", &config)
            .expect("save_sparse should handle inline table");

        let result = std::fs::read_to_string(&path).expect("read");
        let parsed: GlobalConfig = toml::from_str(&result).expect("should parse");
        assert_eq!(parsed.workflow.qa_backend.as_deref(), Some("codex"));
        assert!(parsed.workflow.auto_commit);
    }
}
