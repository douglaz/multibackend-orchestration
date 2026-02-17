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
    pub planner_backend: Option<String>,
    pub implementer_backend: Option<String>,
    pub reviewer_backend: Option<String>,
    pub qa_backend: Option<String>,
    pub qa_enabled: Option<bool>,
    pub completer_backend: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectTemplateOverrides {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub prompt_reviewer: Option<String>,
    pub completer: Option<String>,
    pub qa: Option<String>,
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
