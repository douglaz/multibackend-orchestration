use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;

use super::global::{
    CommitMessageStyle, PlannerStateInPrompt, PreviousSpecsInPrompt, PromptChangeAction,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    #[serde(default)]
    pub workflow: ProjectWorkflowOverrides,
    #[serde(default)]
    pub templates: ProjectTemplateOverrides,
    #[serde(default)]
    pub daemon: ProjectDaemonOverrides,
    #[serde(default)]
    pub amendments: ProjectAmendmentsOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectAmendmentsOverrides {
    pub unify_final_review: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectWorkflowOverrides {
    pub starting_backend: Option<String>,
    pub max_review_iterations: Option<u32>,
    pub max_qa_iterations: Option<u32>,
    pub auto_commit: Option<bool>,
    pub commit_message_style: Option<CommitMessageStyle>,
    pub prompt_change_action: Option<PromptChangeAction>,
    pub prompt_review_enabled: Option<bool>,
    pub prompt_review_backend: Option<String>,
    pub prompt_review_backends: Option<Vec<String>>,
    pub prompt_review_min_reviewers: Option<u32>,
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub qa_backend: Option<String>,
    pub qa_enabled: Option<bool>,
    pub completer_backend: Option<String>,
    pub final_review_enabled: Option<bool>,
    pub final_review_backends: Option<Vec<String>>,
    pub final_review_arbiter_backend: Option<String>,
    pub final_review_min_reviewers: Option<u32>,
    pub final_review_consensus_threshold: Option<f64>,
    pub max_final_review_restarts: Option<u32>,
    pub completion_backends: Option<Vec<String>>,
    pub completion_min_completers: Option<u32>,
    pub completion_consensus_threshold: Option<f64>,
    pub planner_state_in_prompt: Option<PlannerStateInPrompt>,
    pub planner_previous_specs_in_prompt: Option<PreviousSpecsInPrompt>,
    /// `None` = inherit from global; `Some(None)` = override to unlimited; `Some(Some(n))` = cap at n.
    pub planner_max_prior_loops: Option<Option<usize>>,
    #[serde(default)]
    pub max_review_history_entries_in_prompt: Option<usize>,
    #[serde(default)]
    pub max_qa_history_entries_in_prompt: Option<usize>,
    #[serde(default)]
    pub include_history_when_session_reuse_enabled: Option<bool>,
    #[serde(default)]
    pub session_reuse_enabled: Option<bool>,
    #[serde(default)]
    pub session_reuse_roles: Option<Vec<String>>,
    #[serde(default)]
    pub session_reuse_reset_on_prompt_change: Option<bool>,
    #[serde(default)]
    pub session_reuse_reset_on_rollback: Option<bool>,
    #[serde(default)]
    pub pre_commit_fmt: Option<bool>,
    #[serde(default)]
    pub pre_commit_clippy: Option<bool>,
    #[serde(default)]
    pub pre_commit_nix_build: Option<bool>,
    #[serde(default)]
    pub pre_commit_fmt_auto_fix: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectTemplateOverrides {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub prompt_reviewer: Option<String>,
    pub prompt_review_validator: Option<String>,
    pub completer: Option<String>,
    pub qa: Option<String>,
    pub final_reviewer: Option<String>,
    pub quick_dev_plan_implement: Option<String>,
    pub quick_dev_codex_review: Option<String>,
    pub quick_dev_apply_fixes: Option<String>,
    pub planner_position: Option<String>,
    pub vote: Option<String>,
    pub arbiter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectDaemonOverrides {
    pub poll_seconds: Option<u64>,
    pub max_concurrent: Option<u32>,
    pub labels: Option<Vec<String>>,
    pub repo: Option<String>,
    pub refinement_enabled: Option<bool>,
    pub refinement_backend: Option<String>,
    pub auto_rebase_enabled: Option<bool>,
    pub rebase_interval_seconds: Option<u64>,
    pub max_rebases_per_cycle: Option<u32>,
    pub rebase_timeout_seconds: Option<u64>,
    pub rebase_agent_backend: Option<String>,
}

impl ProjectConfig {
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
}
