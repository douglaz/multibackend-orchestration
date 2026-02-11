use std::collections::HashSet;
use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub project_id: String,
    pub project_name: String,
    pub prompt_file: String,
    pub prompt_hash: String,
    pub prompt_hash_at_loop_start: String,
    pub parent_project: Option<String>,
    pub current_loop: u32,
    pub current_phase: Phase,
    pub phase_iteration: u32,
    pub status: ProjectStatus,
    pub loops: Vec<FeatureLoopState>,
    pub completion_attempts: Vec<CompletionLoopState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Planning,
    Implementing,
    #[serde(rename = "qa")]
    QA,
    Reviewing,
    Committing,
    Completing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureLoopState {
    pub loop_number: u32,
    pub slug: String,
    pub feature_name: String,
    pub loop_type: LoopType,
    pub status: LoopStatus,
    pub backends: FeatureLoopBackends,
    pub artifacts: FeatureLoopArtifacts,
    pub commit: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureLoopBackends {
    pub planner: String,
    pub implementer: String,
    pub reviewer: String,
    #[serde(default)]
    pub qa: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureLoopArtifacts {
    pub spec: String,
    pub impl_notes: Option<String>,
    pub reviews: Vec<ReviewExchange>,
    pub approval: Option<String>,
    #[serde(default)]
    pub qa_results: Vec<QaExchange>,
    #[serde(default)]
    pub pending_qa_feedback: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewExchange {
    pub iteration: u32,
    pub feedback: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QaExchange {
    pub iteration: u32,
    pub passed: bool,
    pub report: String,
    pub implementer_response: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionLoopState {
    pub loop_number: u32,
    pub slug: String,
    pub loop_type: LoopType,
    pub status: LoopStatus,
    pub backends: CompletionLoopBackends,
    pub artifacts: CompletionLoopArtifacts,
    pub verdict: Option<CompletionVerdict>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionLoopBackends {
    pub planner: String,
    pub completer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionLoopArtifacts {
    pub termination_request: String,
    pub verdict: Option<String>,
    #[serde(default)]
    pub acceptance_result: Option<String>,
    #[serde(default)]
    pub acceptance_passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletionVerdict {
    Continue,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopType {
    Feature,
    Completion,
}

impl ProjectState {
    pub fn new(
        project_id: &str,
        project_name: &str,
        prompt_hash: &str,
        parent_project: Option<String>,
    ) -> Self {
        Self {
            project_id: project_id.to_owned(),
            project_name: project_name.to_owned(),
            prompt_file: "prompt.md".to_owned(),
            prompt_hash: prompt_hash.to_owned(),
            prompt_hash_at_loop_start: prompt_hash.to_owned(),
            parent_project,
            current_loop: 0,
            current_phase: Phase::Planning,
            phase_iteration: 1,
            status: ProjectStatus::Pending,
            loops: Vec::new(),
            completion_attempts: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let state = serde_json::from_str(&raw)?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(path, raw)?;
        Ok(())
    }

    pub fn next_loop_number(&self) -> u32 {
        self.loops
            .iter()
            .map(|l| l.loop_number)
            .chain(self.completion_attempts.iter().map(|c| c.loop_number))
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn has_in_progress_loop(&self) -> bool {
        self.loops.iter().any(|l| l.status != LoopStatus::Completed)
            || self
                .completion_attempts
                .iter()
                .any(|c| c.status != LoopStatus::Completed)
    }

    pub fn register_feature_loop(
        &mut self,
        loop_number: u32,
        slug: String,
        feature_name: String,
        backends: FeatureLoopBackends,
        spec_path: String,
        started_at: DateTime<Utc>,
    ) {
        self.current_loop = loop_number;
        self.current_phase = Phase::Implementing;
        self.phase_iteration = 1;
        self.status = ProjectStatus::InProgress;

        self.loops.push(FeatureLoopState {
            loop_number,
            slug,
            feature_name,
            loop_type: LoopType::Feature,
            status: LoopStatus::InProgress,
            backends,
            artifacts: FeatureLoopArtifacts {
                spec: spec_path,
                impl_notes: None,
                reviews: Vec::new(),
                approval: None,
                qa_results: Vec::new(),
                pending_qa_feedback: None,
            },
            commit: None,
            started_at,
            completed_at: None,
        });
    }

    pub fn register_completion_attempt(
        &mut self,
        loop_number: u32,
        backends: CompletionLoopBackends,
        termination_request_path: String,
        started_at: DateTime<Utc>,
    ) {
        self.current_loop = loop_number;
        self.current_phase = Phase::Completing;
        self.phase_iteration = 1;
        self.status = ProjectStatus::InProgress;

        self.completion_attempts.push(CompletionLoopState {
            loop_number,
            slug: "completion".to_owned(),
            loop_type: LoopType::Completion,
            status: LoopStatus::InProgress,
            backends,
            artifacts: CompletionLoopArtifacts {
                termination_request: termination_request_path,
                verdict: None,
                acceptance_result: None,
                acceptance_passed: None,
            },
            verdict: None,
            started_at,
            completed_at: None,
        });
    }

    pub fn current_feature_loop(&self) -> Option<&FeatureLoopState> {
        self.loops
            .iter()
            .find(|loop_state| loop_state.loop_number == self.current_loop)
    }

    pub fn current_feature_loop_mut(&mut self) -> Option<&mut FeatureLoopState> {
        self.loops
            .iter_mut()
            .find(|loop_state| loop_state.loop_number == self.current_loop)
    }

    pub fn current_completion_attempt(&self) -> Option<&CompletionLoopState> {
        self.completion_attempts
            .iter()
            .find(|loop_state| loop_state.loop_number == self.current_loop)
    }

    pub fn current_completion_attempt_mut(&mut self) -> Option<&mut CompletionLoopState> {
        self.completion_attempts
            .iter_mut()
            .find(|loop_state| loop_state.loop_number == self.current_loop)
    }

    pub fn remove_loop(&mut self, loop_number: u32) {
        self.loops
            .retain(|loop_state| loop_state.loop_number != loop_number);
        self.completion_attempts
            .retain(|loop_state| loop_state.loop_number != loop_number);
    }

    pub fn last_loop_number(&self) -> u32 {
        self.loops
            .iter()
            .map(|l| l.loop_number)
            .chain(self.completion_attempts.iter().map(|c| c.loop_number))
            .max()
            .unwrap_or(0)
    }

    pub fn validate_invariants(&self) -> std::result::Result<(), String> {
        if self.phase_iteration == 0 {
            return Err("phase_iteration must be >= 1".to_owned());
        }

        let mut seen = HashSet::new();

        for loop_state in &self.loops {
            if loop_state.loop_type != LoopType::Feature {
                return Err(format!(
                    "feature loop {} has invalid loop_type {:?}",
                    loop_state.loop_number, loop_state.loop_type
                ));
            }

            if !seen.insert(loop_state.loop_number) {
                return Err(format!(
                    "duplicate loop_number {} found across state arrays",
                    loop_state.loop_number
                ));
            }
        }

        for attempt in &self.completion_attempts {
            if attempt.loop_type != LoopType::Completion {
                return Err(format!(
                    "completion loop {} has invalid loop_type {:?}",
                    attempt.loop_number, attempt.loop_type
                ));
            }

            if attempt.slug != "completion" {
                return Err(format!(
                    "completion loop {} must use slug 'completion', got '{}'",
                    attempt.loop_number, attempt.slug
                ));
            }

            if !seen.insert(attempt.loop_number) {
                return Err(format!(
                    "duplicate loop_number {} found across state arrays",
                    attempt.loop_number
                ));
            }
        }

        if self.current_loop == 0 {
            if !seen.is_empty() {
                return Err("current_loop is 0 but loop entries exist".to_owned());
            }
        } else if !seen.contains(&self.current_loop) {
            return Err(format!(
                "current_loop {} does not refer to any loop entry",
                self.current_loop
            ));
        }

        Ok(())
    }
}
