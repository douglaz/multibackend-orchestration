use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::RalphError;
use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceIndex {
    pub workspace_version: String,
    pub created_at: DateTime<Utc>,
    pub active_project: Option<String>,
    pub projects: Vec<ProjectRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub status: ProjectLifecycleStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_feature_loops: u32,
    pub total_completion_attempts: u32,
    pub last_loop_number: u32,
    pub parent_project: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectLifecycleStatus {
    Pending,
    InProgress,
    Completed,
}

impl WorkspaceIndex {
    pub fn new(version: &str, created_at: DateTime<Utc>) -> Self {
        Self {
            workspace_version: version.to_owned(),
            created_at,
            active_project: None,
            projects: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let index = serde_json::from_str(&raw)?;
        Ok(index)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let value = serde_json::to_string_pretty(self)?;
        fs::write(path, value)?;
        Ok(())
    }

    pub fn get_project(&self, id: &str) -> Option<&ProjectRef> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn get_project_mut(&mut self, id: &str) -> Option<&mut ProjectRef> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    pub fn add_project(&mut self, project: ProjectRef) -> Result<()> {
        if self.projects.iter().any(|p| p.id == project.id) {
            return Err(RalphError::Validation(format!(
                "project '{}' already exists",
                project.id
            )));
        }
        self.projects.push(project);
        Ok(())
    }

    pub fn set_active_project(&mut self, id: &str) -> Result<()> {
        if self.projects.iter().all(|p| p.id != id) {
            return Err(RalphError::ProjectNotFound(id.to_owned()));
        }
        self.active_project = Some(id.to_owned());
        Ok(())
    }

    pub fn active_project_ref(&self) -> Option<&ProjectRef> {
        self.active_project
            .as_deref()
            .and_then(|id| self.get_project(id))
    }
}
