use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub project_id: String,
    pub project_name: String,
    #[serde(default = "default_created_at")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "default_prompt_file")]
    pub prompt_file: String,
    #[serde(default)]
    pub prompt_hash: String,
    #[serde(default)]
    pub prompt_hash_at_loop_start: String,
    #[serde(default)]
    pub prompt_review_completed: bool,
    pub parent_project: Option<String>,
    pub current_loop: u32,
    pub current_phase: Phase,
    pub phase_iteration: u32,
    pub status: ProjectStatus,
    pub loops: Vec<FeatureLoopState>,
    pub completion_attempts: Vec<CompletionLoopState>,
    #[serde(default)]
    pub session_store: SessionStore,
}

// ---------------------------------------------------------------------------
// Session reuse data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub backend_spec: String,
    pub role: String,
    pub loop_number: u32,
    pub bootstrap_hash: String,
    pub call_count: u32,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionStore {
    #[serde(default)]
    pub records: Vec<SessionRecord>,
}

impl SessionStore {
    pub fn lookup(
        &self,
        loop_number: u32,
        role: &str,
        backend_spec: &str,
    ) -> Option<&SessionRecord> {
        self.records.iter().find(|r| {
            r.loop_number == loop_number && r.role == role && r.backend_spec == backend_spec
        })
    }

    pub fn upsert(&mut self, record: SessionRecord) {
        if let Some(existing) = self.records.iter_mut().find(|r| {
            r.loop_number == record.loop_number
                && r.role == record.role
                && r.backend_spec == record.backend_spec
        }) {
            *existing = record;
        } else {
            self.records.push(record);
        }
    }

    pub fn remove_for_loop(&mut self, loop_number: u32) {
        self.records.retain(|r| r.loop_number != loop_number);
    }
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
    Failed,
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
    pub acceptance_results: Vec<AcceptanceQaResult>,
    #[serde(default, skip_serializing)]
    pub acceptance_result: Option<String>,
    #[serde(default, skip_serializing)]
    pub acceptance_passed: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AcceptanceQaResult {
    pub backend: String,
    pub passed: bool,
    pub artifact: String,
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

fn default_prompt_file() -> String {
    "prompt.md".to_owned()
}

pub(crate) fn default_created_at() -> DateTime<Utc> {
    DateTime::<Utc>::MIN_UTC
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
            created_at: Utc::now(),
            prompt_file: "prompt.md".to_owned(),
            prompt_hash: prompt_hash.to_owned(),
            prompt_hash_at_loop_start: prompt_hash.to_owned(),
            prompt_review_completed: false,
            parent_project,
            current_loop: 0,
            current_phase: Phase::Planning,
            phase_iteration: 1,
            status: ProjectStatus::Pending,
            loops: Vec::new(),
            completion_attempts: Vec::new(),
            session_store: SessionStore::default(),
        }
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
                acceptance_results: Vec::new(),
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
        // Also remove session records for this loop (spec D1/D6).
        self.session_store.remove_for_loop(loop_number);
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

    fn migrate_legacy_acceptance_results(&mut self) {
        for attempt in &mut self.completion_attempts {
            attempt.artifacts.migrate_legacy_acceptance_result();
        }
    }
}

impl CompletionLoopArtifacts {
    pub fn has_acceptance_result_for(&self, backend: &str) -> bool {
        self.acceptance_results
            .iter()
            .any(|result| result.backend == backend)
    }

    pub fn upsert_acceptance_result(&mut self, result: AcceptanceQaResult) {
        if let Some(existing) = self
            .acceptance_results
            .iter_mut()
            .find(|existing| existing.backend == result.backend)
        {
            *existing = result;
            return;
        }

        self.acceptance_results.push(result);
    }

    pub fn acceptance_all_required_passed(&self, required: &[&str]) -> bool {
        required.iter().all(|backend| {
            self.acceptance_results
                .iter()
                .any(|result| result.backend == *backend && result.passed)
        })
    }

    pub fn acceptance_any_required_failed(&self, required: &[&str]) -> bool {
        required.iter().any(|backend| {
            self.acceptance_results
                .iter()
                .any(|result| result.backend == *backend && !result.passed)
        })
    }

    fn migrate_legacy_acceptance_result(&mut self) {
        if self.acceptance_results.is_empty() {
            if let (Some(artifact), Some(passed)) =
                (self.acceptance_result.clone(), self.acceptance_passed)
            {
                self.acceptance_results.push(AcceptanceQaResult {
                    backend: "unknown".to_owned(),
                    passed,
                    artifact,
                });
            }
        }

        self.acceptance_result = None;
        self.acceptance_passed = None;
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use super::{default_created_at, ProjectState};

    #[test]
    fn new_state_defaults_prompt_review_completed_to_false() {
        let state = ProjectState::new("demo", "Demo", "abc123", None);
        assert!(!state.prompt_review_completed);
    }

    #[test]
    fn legacy_state_without_prompt_review_field_deserializes_to_false() {
        let state = ProjectState::new("demo", "Demo", "abc123", None);
        let mut value = serde_json::to_value(&state).expect("serialize state");
        value
            .as_object_mut()
            .expect("state should serialize as object")
            .remove("prompt_review_completed");

        let parsed: ProjectState = serde_json::from_value(value).expect("deserialize legacy state");
        assert!(!parsed.prompt_review_completed);
    }

    #[test]
    fn new_state_sets_created_at() {
        let before = Utc::now();
        let state = ProjectState::new("demo", "Demo", "abc123", None);
        let after = Utc::now();
        assert!(state.created_at >= before);
        assert!(state.created_at <= after);
    }

    #[test]
    fn legacy_state_without_created_at_deserializes_to_sentinel() {
        let state = ProjectState::new("demo", "Demo", "abc123", None);
        let mut value = serde_json::to_value(&state).expect("serialize state");
        value
            .as_object_mut()
            .expect("state should serialize as object")
            .remove("created_at");

        let parsed: ProjectState = serde_json::from_value(value).expect("deserialize legacy state");
        assert_eq!(parsed.created_at, DateTime::<Utc>::MIN_UTC);
        assert_eq!(parsed.created_at, default_created_at());
    }

    #[test]
    fn legacy_state_without_session_store_deserializes_to_empty() {
        let state = ProjectState::new("demo", "Demo", "abc123", None);
        let mut value = serde_json::to_value(&state).expect("serialize state");
        value
            .as_object_mut()
            .expect("state should serialize as object")
            .remove("session_store");

        let parsed: ProjectState = serde_json::from_value(value).expect("deserialize legacy state");
        assert!(parsed.session_store.records.is_empty());
    }

    #[test]
    fn session_store_lookup_finds_matching_record() {
        use super::SessionStore;
        let mut store = SessionStore::default();
        store.upsert(super::SessionRecord {
            session_id: "sid-1".to_owned(),
            backend_spec: "claude(opus)".to_owned(),
            role: "implementer".to_owned(),
            loop_number: 1,
            bootstrap_hash: "hash1".to_owned(),
            call_count: 1,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
        });

        assert!(store.lookup(1, "implementer", "claude(opus)").is_some());
        assert!(store.lookup(1, "reviewer", "claude(opus)").is_none());
        assert!(store.lookup(2, "implementer", "claude(opus)").is_none());
        assert!(store.lookup(1, "implementer", "codex").is_none());
    }

    #[test]
    fn session_store_upsert_replaces_existing_record() {
        use super::{SessionRecord, SessionStore};
        let mut store = SessionStore::default();
        let now = Utc::now();
        store.upsert(SessionRecord {
            session_id: "sid-old".to_owned(),
            backend_spec: "claude".to_owned(),
            role: "qa".to_owned(),
            loop_number: 2,
            bootstrap_hash: "hash".to_owned(),
            call_count: 1,
            created_at: now,
            last_used_at: now,
        });
        store.upsert(SessionRecord {
            session_id: "sid-new".to_owned(),
            backend_spec: "claude".to_owned(),
            role: "qa".to_owned(),
            loop_number: 2,
            bootstrap_hash: "hash".to_owned(),
            call_count: 2,
            created_at: now,
            last_used_at: now,
        });
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.records[0].session_id, "sid-new");
        assert_eq!(store.records[0].call_count, 2);
    }

    #[test]
    fn session_store_remove_for_loop_clears_matching_records() {
        use super::{SessionRecord, SessionStore};
        let mut store = SessionStore::default();
        let now = Utc::now();
        for (loop_number, role) in [(1, "implementer"), (1, "reviewer"), (2, "implementer")] {
            store.upsert(SessionRecord {
                session_id: format!("sid-{loop_number}-{role}"),
                backend_spec: "claude".to_owned(),
                role: role.to_owned(),
                loop_number,
                bootstrap_hash: "h".to_owned(),
                call_count: 1,
                created_at: now,
                last_used_at: now,
            });
        }
        assert_eq!(store.records.len(), 3);
        store.remove_for_loop(1);
        assert_eq!(store.records.len(), 1);
        assert_eq!(store.records[0].loop_number, 2);
    }

    #[test]
    fn session_store_serde_roundtrip() {
        use super::{SessionRecord, SessionStore};
        let now = Utc::now();
        let mut store = SessionStore::default();
        store.upsert(SessionRecord {
            session_id: "sid-rt".to_owned(),
            backend_spec: "codex(gpt-5.3-codex-high)".to_owned(),
            role: "reviewer".to_owned(),
            loop_number: 3,
            bootstrap_hash: "abcd1234".to_owned(),
            call_count: 5,
            created_at: now,
            last_used_at: now,
        });
        let json = serde_json::to_string(&store).expect("serialize session store");
        let parsed: SessionStore = serde_json::from_str(&json).expect("deserialize session store");
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].session_id, "sid-rt");
        assert_eq!(parsed.records[0].call_count, 5);
    }

    #[test]
    fn new_state_initializes_empty_session_store() {
        let state = ProjectState::new("demo", "Demo", "abc123", None);
        assert!(state.session_store.records.is_empty());
    }

    #[test]
    fn remove_loop_clears_session_records() {
        use super::SessionRecord;
        let mut state = ProjectState::new("demo", "Demo", "abc123", None);
        let now = Utc::now();
        state.session_store.upsert(SessionRecord {
            session_id: "sid-1".to_owned(),
            backend_spec: "claude".to_owned(),
            role: "implementer".to_owned(),
            loop_number: 1,
            bootstrap_hash: "h".to_owned(),
            call_count: 1,
            created_at: now,
            last_used_at: now,
        });
        state.session_store.upsert(SessionRecord {
            session_id: "sid-2".to_owned(),
            backend_spec: "claude".to_owned(),
            role: "reviewer".to_owned(),
            loop_number: 2,
            bootstrap_hash: "h".to_owned(),
            call_count: 1,
            created_at: now,
            last_used_at: now,
        });
        // remove_loop must clear session records for that loop (spec D1)
        state.remove_loop(1);
        assert_eq!(
            state.session_store.records.len(),
            1,
            "only loop 2 session should remain"
        );
        assert_eq!(state.session_store.records[0].loop_number, 2);
    }
}
