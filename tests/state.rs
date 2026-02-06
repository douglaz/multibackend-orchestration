//! Unit tests for project state serialization and management.

use chrono::Utc;
use ralph::project::state::{
    CompletionLoopArtifacts, CompletionLoopBackends, CompletionLoopState, CompletionVerdict,
    FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType, Phase,
    ProjectState, ProjectStatus, ReviewExchange,
};
use tempfile::TempDir;

#[test]
fn test_new_project_state_defaults() {
    let state = ProjectState::new("01-poc", "Proof of Concept", "abc123", None);

    assert_eq!(state.project_id, "01-poc");
    assert_eq!(state.project_name, "Proof of Concept");
    assert_eq!(state.prompt_file, "prompt.md");
    assert_eq!(state.prompt_hash, "abc123");
    assert_eq!(state.prompt_hash_at_loop_start, "abc123");
    assert!(state.parent_project.is_none());
    assert_eq!(state.current_loop, 0);
    assert_eq!(state.current_phase, Phase::Planning);
    assert_eq!(state.phase_iteration, 1);
    assert_eq!(state.status, ProjectStatus::Pending);
    assert!(state.loops.is_empty());
    assert!(state.completion_attempts.is_empty());
}

#[test]
fn test_project_state_with_parent() {
    let state = ProjectState::new("02-alpha", "Alpha", "def456", Some("01-poc".to_owned()));
    assert_eq!(state.parent_project.as_deref(), Some("01-poc"));
}

#[test]
fn test_next_loop_number_empty() {
    let state = ProjectState::new("test", "Test", "hash", None);
    assert_eq!(state.next_loop_number(), 1);
}

#[test]
fn test_next_loop_number_with_loops() {
    let mut state = ProjectState::new("test", "Test", "hash", None);

    let backends = FeatureLoopBackends {
        planner: "claude".to_owned(),
        implementer: "codex".to_owned(),
        reviewer: "claude".to_owned(),
    };
    let artifacts = FeatureLoopArtifacts {
        spec: "loops/001-test/spec.md".to_owned(),
        impl_notes: None,
        reviews: vec![],
        approval: None,
    };

    state.loops.push(FeatureLoopState {
        loop_number: 1,
        slug: "test".to_owned(),
        feature_name: "Test Feature".to_owned(),
        loop_type: LoopType::Feature,
        status: LoopStatus::Completed,
        backends,
        artifacts,
        commit: Some("abc123".to_owned()),
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });

    assert_eq!(state.next_loop_number(), 2);
}

#[test]
fn test_next_loop_number_with_completion_attempts() {
    let mut state = ProjectState::new("test", "Test", "hash", None);

    state.completion_attempts.push(CompletionLoopState {
        loop_number: 5,
        slug: "completion".to_owned(),
        loop_type: LoopType::Completion,
        status: LoopStatus::Completed,
        backends: CompletionLoopBackends {
            planner: "claude".to_owned(),
            completer: "codex".to_owned(),
        },
        artifacts: CompletionLoopArtifacts {
            termination_request: "loops/005-completion/termination-request.md".to_owned(),
            verdict: Some("loops/005-completion/completer-verdict.md".to_owned()),
        },
        verdict: Some(CompletionVerdict::Continue),
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });

    assert_eq!(state.next_loop_number(), 6);
}

#[test]
fn test_save_and_load_round_trip() {
    let temp_dir = TempDir::new().unwrap();
    let state_path = temp_dir.path().join("state.json");

    let mut original = ProjectState::new("01-poc", "Proof of Concept", "abc123", None);
    original.status = ProjectStatus::InProgress;
    original.current_loop = 2;
    original.current_phase = Phase::Reviewing;
    original.phase_iteration = 3;

    let backends = FeatureLoopBackends {
        planner: "claude".to_owned(),
        implementer: "codex".to_owned(),
        reviewer: "claude".to_owned(),
    };
    original.loops.push(FeatureLoopState {
        loop_number: 1,
        slug: "user-auth".to_owned(),
        feature_name: "User Authentication".to_owned(),
        loop_type: LoopType::Feature,
        status: LoopStatus::Completed,
        backends: backends.clone(),
        artifacts: FeatureLoopArtifacts {
            spec: "loops/001-user-auth/spec.md".to_owned(),
            impl_notes: Some("loops/001-user-auth/impl-notes.md".to_owned()),
            reviews: vec![ReviewExchange {
                iteration: 1,
                feedback: "loops/001-user-auth/review-001-feedback.md".to_owned(),
                response: "loops/001-user-auth/impl-response-001.md".to_owned(),
            }],
            approval: Some("loops/001-user-auth/review-approved.md".to_owned()),
        },
        commit: Some("abc123".to_owned()),
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });

    original.save(&state_path).unwrap();
    let loaded = ProjectState::load(&state_path).unwrap();

    assert_eq!(loaded.project_id, original.project_id);
    assert_eq!(loaded.project_name, original.project_name);
    assert_eq!(loaded.status, original.status);
    assert_eq!(loaded.current_loop, original.current_loop);
    assert_eq!(loaded.current_phase, original.current_phase);
    assert_eq!(loaded.phase_iteration, original.phase_iteration);
    assert_eq!(loaded.loops.len(), 1);
    assert_eq!(loaded.loops[0].feature_name, "User Authentication");
    assert_eq!(loaded.loops[0].artifacts.reviews.len(), 1);
}

#[test]
fn test_phase_serialization() {
    // Verify all phases serialize correctly as snake_case
    let phases = vec![
        (Phase::Planning, "planning"),
        (Phase::Implementing, "implementing"),
        (Phase::Reviewing, "reviewing"),
        (Phase::Committing, "committing"),
        (Phase::Completing, "completing"),
    ];

    for (phase, expected) in phases {
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, format!("\"{}\"", expected));
    }
}

#[test]
fn test_project_status_serialization() {
    let statuses = vec![
        (ProjectStatus::Pending, "pending"),
        (ProjectStatus::InProgress, "in_progress"),
        (ProjectStatus::Completed, "completed"),
    ];

    for (status, expected) in statuses {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", expected));
    }
}

#[test]
fn test_loop_status_serialization() {
    let statuses = vec![
        (LoopStatus::Pending, "pending"),
        (LoopStatus::InProgress, "in_progress"),
        (LoopStatus::Completed, "completed"),
    ];

    for (status, expected) in statuses {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", expected));
    }
}

#[test]
fn test_completion_verdict_serialization() {
    let verdicts = vec![
        (CompletionVerdict::Continue, "continue"),
        (CompletionVerdict::Complete, "complete"),
    ];

    for (verdict, expected) in verdicts {
        let json = serde_json::to_string(&verdict).unwrap();
        assert_eq!(json, format!("\"{}\"", expected));
    }
}

#[test]
fn test_register_feature_loop() {
    let mut state = ProjectState::new("test", "Test", "hash", None);

    let backends = FeatureLoopBackends {
        planner: "claude".to_owned(),
        implementer: "codex".to_owned(),
        reviewer: "claude".to_owned(),
    };

    state.register_feature_loop(
        1,
        "user-auth".to_owned(),
        "User Authentication".to_owned(),
        backends,
        "loops/001-user-auth/spec.md".to_owned(),
        Utc::now(),
    );

    assert_eq!(state.current_loop, 1);
    assert_eq!(state.current_phase, Phase::Implementing);
    assert_eq!(state.phase_iteration, 1);
    assert_eq!(state.status, ProjectStatus::InProgress);
    assert_eq!(state.loops.len(), 1);
    assert_eq!(state.loops[0].status, LoopStatus::InProgress);
}

#[test]
fn test_register_completion_attempt() {
    let mut state = ProjectState::new("test", "Test", "hash", None);

    let backends = CompletionLoopBackends {
        planner: "claude".to_owned(),
        completer: "codex".to_owned(),
    };

    state.register_completion_attempt(
        3,
        backends,
        "loops/003-completion/termination-request.md".to_owned(),
        Utc::now(),
    );

    assert_eq!(state.current_loop, 3);
    assert_eq!(state.current_phase, Phase::Completing);
    assert_eq!(state.completion_attempts.len(), 1);
    assert_eq!(state.completion_attempts[0].slug, "completion");
}

#[test]
fn test_has_in_progress_loop() {
    let mut state = ProjectState::new("test", "Test", "hash", None);
    assert!(!state.has_in_progress_loop());

    state.loops.push(FeatureLoopState {
        loop_number: 1,
        slug: "test".to_owned(),
        feature_name: "Test".to_owned(),
        loop_type: LoopType::Feature,
        status: LoopStatus::InProgress,
        backends: FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "codex".to_owned(),
            reviewer: "claude".to_owned(),
        },
        artifacts: FeatureLoopArtifacts {
            spec: "spec.md".to_owned(),
            impl_notes: None,
            reviews: vec![],
            approval: None,
        },
        commit: None,
        started_at: Utc::now(),
        completed_at: None,
    });

    assert!(state.has_in_progress_loop());
}

#[test]
fn test_remove_loop() {
    let mut state = ProjectState::new("test", "Test", "hash", None);

    state.loops.push(FeatureLoopState {
        loop_number: 1,
        slug: "test".to_owned(),
        feature_name: "Test".to_owned(),
        loop_type: LoopType::Feature,
        status: LoopStatus::Completed,
        backends: FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "codex".to_owned(),
            reviewer: "claude".to_owned(),
        },
        artifacts: FeatureLoopArtifacts {
            spec: "spec.md".to_owned(),
            impl_notes: None,
            reviews: vec![],
            approval: None,
        },
        commit: None,
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });

    assert_eq!(state.loops.len(), 1);
    state.remove_loop(1);
    assert!(state.loops.is_empty());
}

#[test]
fn test_last_loop_number() {
    let mut state = ProjectState::new("test", "Test", "hash", None);
    assert_eq!(state.last_loop_number(), 0);

    state.loops.push(FeatureLoopState {
        loop_number: 3,
        slug: "test".to_owned(),
        feature_name: "Test".to_owned(),
        loop_type: LoopType::Feature,
        status: LoopStatus::Completed,
        backends: FeatureLoopBackends {
            planner: "claude".to_owned(),
            implementer: "codex".to_owned(),
            reviewer: "claude".to_owned(),
        },
        artifacts: FeatureLoopArtifacts {
            spec: "spec.md".to_owned(),
            impl_notes: None,
            reviews: vec![],
            approval: None,
        },
        commit: None,
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
    });

    assert_eq!(state.last_loop_number(), 3);
}
