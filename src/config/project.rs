use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Result;

use super::global::{CommitMessageStyle, PromptChangeAction};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    #[serde(default)]
    pub workflow: ProjectWorkflowOverrides,
    #[serde(default)]
    pub templates: ProjectTemplateOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectWorkflowOverrides {
    pub starting_backend: Option<String>,
    pub max_review_iterations: Option<u32>,
    pub auto_commit: Option<bool>,
    pub commit_message_style: Option<CommitMessageStyle>,
    pub prompt_change_action: Option<PromptChangeAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectTemplateOverrides {
    pub planner: Option<String>,
    pub implementer: Option<String>,
    pub reviewer: Option<String>,
    pub completer: Option<String>,
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
