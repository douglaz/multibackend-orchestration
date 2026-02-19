use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::project::state::{LoopStatus, ProjectState, ProjectStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub status: ProjectStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_feature_loops: u32,
    pub total_completion_attempts: u32,
    pub last_loop_number: u32,
    pub parent_project: Option<String>,
}

pub fn summarize_project(id: &str, state: &ProjectState, project_dir: &Path) -> ProjectSummary {
    let created_at = if state.created_at == DateTime::<Utc>::MIN_UTC {
        state
            .loops
            .iter()
            .map(|loop_state| loop_state.started_at)
            .chain(
                state
                    .completion_attempts
                    .iter()
                    .map(|attempt| attempt.started_at),
            )
            .min()
            .or_else(|| {
                fs::metadata(project_dir.join("prompt.md"))
                    .or_else(|_| fs::metadata(project_dir))
                    .and_then(|meta| meta.modified())
                    .ok()
                    .map(DateTime::<Utc>::from)
            })
            .unwrap_or_else(Utc::now)
    } else {
        state.created_at
    };

    let completed_at = if state.status == ProjectStatus::Completed {
        state
            .loops
            .iter()
            .filter_map(|loop_state| loop_state.completed_at)
            .chain(
                state
                    .completion_attempts
                    .iter()
                    .filter_map(|attempt| attempt.completed_at),
            )
            .max()
    } else {
        None
    };

    ProjectSummary {
        id: id.to_owned(),
        name: state.project_name.clone(),
        status: state.status.clone(),
        created_at,
        completed_at,
        total_feature_loops: state
            .loops
            .iter()
            .filter(|loop_state| loop_state.status == LoopStatus::Completed)
            .count() as u32,
        total_completion_attempts: state
            .completion_attempts
            .iter()
            .filter(|attempt| attempt.status == LoopStatus::Completed)
            .count() as u32,
        last_loop_number: state.last_loop_number(),
        parent_project: state.parent_project.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{DateTime, Utc};
    use tempfile::tempdir;

    use super::summarize_project;
    use crate::project::state::{
        CompletionLoopArtifacts, CompletionLoopBackends, CompletionLoopState, FeatureLoopArtifacts,
        FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType, ProjectState, ProjectStatus,
    };

    fn parse_utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid RFC3339 timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn summarizes_project_from_state() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "prompt").expect("write placeholder prompt");

        let mut state = ProjectState::new(
            "state-id",
            "Demo Project",
            "hash",
            Some("parent-123".to_owned()),
        );
        state.created_at = DateTime::<Utc>::MIN_UTC;
        state.status = ProjectStatus::Completed;

        state.loops.push(FeatureLoopState {
            loop_number: 1,
            slug: "feature-a".to_owned(),
            feature_name: "Feature A".to_owned(),
            loop_type: LoopType::Feature,
            status: LoopStatus::Completed,
            backends: FeatureLoopBackends {
                planner: "planner".to_owned(),
                implementer: "implementer".to_owned(),
                reviewer: "reviewer".to_owned(),
                qa: "qa".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: "spec.md".to_owned(),
                impl_notes: None,
                reviews: Vec::new(),
                approval: None,
                qa_results: Vec::new(),
                pending_qa_feedback: None,
            },
            commit: None,
            started_at: parse_utc("2026-01-01T10:00:00Z"),
            completed_at: Some(parse_utc("2026-01-01T11:00:00Z")),
        });
        state.loops.push(FeatureLoopState {
            loop_number: 2,
            slug: "feature-b".to_owned(),
            feature_name: "Feature B".to_owned(),
            loop_type: LoopType::Feature,
            status: LoopStatus::InProgress,
            backends: FeatureLoopBackends {
                planner: "planner".to_owned(),
                implementer: "implementer".to_owned(),
                reviewer: "reviewer".to_owned(),
                qa: "qa".to_owned(),
            },
            artifacts: FeatureLoopArtifacts {
                spec: "spec.md".to_owned(),
                impl_notes: None,
                reviews: Vec::new(),
                approval: None,
                qa_results: Vec::new(),
                pending_qa_feedback: None,
            },
            commit: None,
            started_at: parse_utc("2026-01-02T10:00:00Z"),
            completed_at: None,
        });

        state.completion_attempts.push(CompletionLoopState {
            loop_number: 3,
            slug: "completion".to_owned(),
            loop_type: LoopType::Completion,
            status: LoopStatus::Completed,
            backends: CompletionLoopBackends {
                planner: "planner".to_owned(),
                completer: "completer".to_owned(),
            },
            artifacts: CompletionLoopArtifacts {
                termination_request: "termination.md".to_owned(),
                verdict: None,
                acceptance_results: Vec::new(),
                acceptance_result: None,
                acceptance_passed: None,
            },
            verdict: None,
            started_at: parse_utc("2026-01-03T10:00:00Z"),
            completed_at: Some(parse_utc("2026-01-03T12:00:00Z")),
        });

        let summary = summarize_project("dir-id", &state, &project_dir);

        assert_eq!(summary.id, "dir-id");
        assert_eq!(summary.name, "Demo Project");
        assert_eq!(summary.status, ProjectStatus::Completed);
        assert_eq!(summary.created_at, parse_utc("2026-01-01T10:00:00Z"));
        assert_eq!(
            summary.completed_at,
            Some(parse_utc("2026-01-03T12:00:00Z"))
        );
        assert_eq!(summary.total_feature_loops, 1);
        assert_eq!(summary.total_completion_attempts, 1);
        assert_eq!(summary.last_loop_number, 3);
        assert_eq!(summary.parent_project, Some("parent-123".to_owned()));
    }

    #[test]
    fn completed_at_is_none_for_non_completed_projects() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(project_dir.join("prompt.md"), "prompt").expect("write placeholder prompt");

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.status = ProjectStatus::InProgress;
        let summary = summarize_project("demo", &state, &project_dir);
        assert!(summary.completed_at.is_none());
    }

    #[test]
    fn legacy_state_without_created_at_falls_back_to_prompt_file_mtime() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&project_dir).expect("create project dir");
        let prompt_path = project_dir.join("prompt.md");
        fs::write(&prompt_path, "prompt").expect("write placeholder prompt");
        let expected_created_at = DateTime::<Utc>::from(
            fs::metadata(&prompt_path)
                .expect("prompt metadata")
                .modified()
                .expect("prompt mtime"),
        );

        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.created_at = DateTime::<Utc>::MIN_UTC;
        state.loops.clear();
        state.completion_attempts.clear();

        let summary = summarize_project("demo", &state, &project_dir);
        assert_eq!(summary.created_at, expected_created_at);
        assert_ne!(summary.created_at, DateTime::<Utc>::MIN_UTC);
    }

    #[test]
    fn legacy_state_uses_now_when_prompt_metadata_is_unavailable() {
        let temp = tempdir().expect("tempdir");
        let missing_project_dir = temp.path().join("missing");
        let mut state = ProjectState::new("demo", "Demo", "hash", None);
        state.created_at = DateTime::<Utc>::MIN_UTC;
        state.loops.clear();
        state.completion_attempts.clear();

        let before = Utc::now();
        let summary = summarize_project("demo", &state, &missing_project_dir);
        let after = Utc::now();

        assert!(summary.created_at >= before);
        assert!(summary.created_at <= after);
        assert_ne!(summary.created_at, DateTime::<Utc>::MIN_UTC);
    }
}
