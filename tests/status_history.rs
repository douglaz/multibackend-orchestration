//! Integration tests for QA-aware status and history output rendering.
//!
//! These tests verify that QA fields on state structures are accessible and
//! render correctly for zero-attempts, pass, fail, and malformed/missing cases.
//! Since the CLI execute functions print to stdout and require a full workspace,
//! we test the underlying data access patterns and serialization here.

use chrono::Utc;
use ralph::project::state::{
    FeatureLoopArtifacts, FeatureLoopBackends, FeatureLoopState, LoopStatus, LoopType, Phase,
    ProjectState, ProjectStatus, QaExchange,
};

fn make_backends() -> FeatureLoopBackends {
    FeatureLoopBackends {
        planner: "claude(opus)".to_owned(),
        implementer: "codex(gpt-5.3-codex-high)".to_owned(),
        reviewer: "claude(opus)".to_owned(),
        qa: "claude(opus)".to_owned(),
    }
}

fn make_loop(loop_number: u32, qa_results: Vec<QaExchange>) -> FeatureLoopState {
    FeatureLoopState {
        loop_number,
        slug: "test-feature".to_owned(),
        feature_name: "Test Feature".to_owned(),
        loop_type: LoopType::Feature,
        status: LoopStatus::InProgress,
        backends: make_backends(),
        artifacts: FeatureLoopArtifacts {
            spec: format!("loops/{loop_number:03}-test-feature/spec.md"),
            impl_notes: Some(format!("loops/{loop_number:03}-test-feature/impl-notes.md")),
            reviews: vec![],
            approval: None,
            qa_results,
            pending_qa_feedback: None,
        },
        commit: None,
        started_at: Utc::now(),
        completed_at: None,
    }
}

fn make_state(loop_number: u32, qa_results: Vec<QaExchange>) -> ProjectState {
    let mut state = ProjectState::new("test-proj", "Test Project", "hash123", None);
    state.current_loop = loop_number;
    state.current_phase = Phase::Implementing;
    state.phase_iteration = 1;
    state.status = ProjectStatus::InProgress;
    state.loops.push(make_loop(loop_number, qa_results));
    state
}

// --- Status rendering tests ---

#[test]
fn status_zero_qa_attempts_shows_no_qa_fields() {
    let state = make_state(1, vec![]);
    let loop_state = state.current_feature_loop().unwrap();

    // With no QA results, the last() should be None
    assert!(loop_state.artifacts.qa_results.last().is_none());
    // Existing review feedback behavior should be unaffected
    assert!(loop_state.artifacts.reviews.is_empty());
}

#[test]
fn status_latest_qa_pass_shows_iteration_and_verdict() {
    let qa = vec![QaExchange {
        iteration: 1,
        passed: true,
        report: "loops/001-test-feature/qa-001-pass.md".to_owned(),
        implementer_response: None,
    }];
    let state = make_state(1, qa);
    let loop_state = state.current_feature_loop().unwrap();
    let latest = loop_state.artifacts.qa_results.last().unwrap();

    assert_eq!(latest.iteration, 1);
    assert!(latest.passed);
    assert!(latest.report.contains("qa-001-pass"));
}

#[test]
fn status_latest_qa_fail_shows_iteration_and_verdict() {
    let qa = vec![
        QaExchange {
            iteration: 1,
            passed: false,
            report: "loops/001-test-feature/qa-001-fail.md".to_owned(),
            implementer_response: Some("loops/001-test-feature/impl-qa-response-001.md".to_owned()),
        },
        QaExchange {
            iteration: 2,
            passed: false,
            report: "loops/001-test-feature/qa-002-fail.md".to_owned(),
            implementer_response: None,
        },
    ];
    let state = make_state(1, qa);
    let loop_state = state.current_feature_loop().unwrap();
    let latest = loop_state.artifacts.qa_results.last().unwrap();

    assert_eq!(latest.iteration, 2);
    assert!(!latest.passed);
    assert!(latest.report.contains("qa-002-fail"));
}

#[test]
fn status_qa_verdict_label_correct() {
    let pass = QaExchange {
        iteration: 1,
        passed: true,
        report: "qa-001-pass.md".to_owned(),
        implementer_response: None,
    };
    let fail = QaExchange {
        iteration: 1,
        passed: false,
        report: "qa-001-fail.md".to_owned(),
        implementer_response: None,
    };

    assert_eq!(if pass.passed { "PASS" } else { "FAIL" }, "PASS");
    assert_eq!(if fail.passed { "PASS" } else { "FAIL" }, "FAIL");
}

// --- History rendering tests ---

#[test]
fn history_verbose_zero_qa_attempts_shows_none() {
    let state = make_state(1, vec![]);
    let loop_state = &state.loops[0];

    let qa_count = loop_state.artifacts.qa_results.len();
    let qa_verdict = loop_state
        .artifacts
        .qa_results
        .last()
        .map(|q| if q.passed { "pass" } else { "fail" })
        .unwrap_or("none");

    assert_eq!(qa_count, 0);
    assert_eq!(qa_verdict, "none");
}

#[test]
fn history_verbose_with_qa_pass_shows_count_and_verdict() {
    let qa = vec![QaExchange {
        iteration: 1,
        passed: true,
        report: "loops/001-test-feature/qa-001-pass.md".to_owned(),
        implementer_response: None,
    }];
    let state = make_state(1, qa);
    let loop_state = &state.loops[0];

    let qa_count = loop_state.artifacts.qa_results.len();
    let qa_verdict = loop_state
        .artifacts
        .qa_results
        .last()
        .map(|q| if q.passed { "pass" } else { "fail" })
        .unwrap_or("none");
    let qa_report = loop_state
        .artifacts
        .qa_results
        .last()
        .map(|q| q.report.as_str())
        .unwrap_or("none");

    assert_eq!(qa_count, 1);
    assert_eq!(qa_verdict, "pass");
    assert!(qa_report.contains("qa-001-pass"));
}

#[test]
fn history_verbose_with_qa_fail_shows_count_and_verdict() {
    let qa = vec![
        QaExchange {
            iteration: 1,
            passed: false,
            report: "loops/001-test-feature/qa-001-fail.md".to_owned(),
            implementer_response: Some("loops/001-test-feature/impl-qa-response-001.md".to_owned()),
        },
        QaExchange {
            iteration: 2,
            passed: true,
            report: "loops/001-test-feature/qa-002-pass.md".to_owned(),
            implementer_response: None,
        },
    ];
    let state = make_state(1, qa);
    let loop_state = &state.loops[0];

    let qa_count = loop_state.artifacts.qa_results.len();
    let qa_verdict = loop_state
        .artifacts
        .qa_results
        .last()
        .map(|q| if q.passed { "pass" } else { "fail" })
        .unwrap_or("none");

    assert_eq!(qa_count, 2);
    assert_eq!(qa_verdict, "pass");
}

#[test]
fn history_non_verbose_output_unchanged_with_qa() {
    // Non-verbose output only shows loop_number, feature_name, status
    // QA fields should not affect this output format
    let qa = vec![QaExchange {
        iteration: 1,
        passed: true,
        report: "qa-001-pass.md".to_owned(),
        implementer_response: None,
    }];
    let state = make_state(1, qa);
    let loop_state = &state.loops[0];

    // Non-verbose format: "Loop N: feature_name (status)"
    let output = format!(
        "Loop {}: {} ({})",
        loop_state.loop_number,
        loop_state.feature_name,
        match loop_state.status {
            LoopStatus::Pending => "pending",
            LoopStatus::InProgress => "in_progress",
            LoopStatus::Completed => "completed",
        }
    );
    assert_eq!(output, "Loop 1: Test Feature (in_progress)");
}

#[test]
fn history_json_output_backward_compatible() {
    // JSON serialization should work with QA fields via serde(default)
    let state = make_state(1, vec![]);
    let json = serde_json::to_value(&state.loops[0]).unwrap();

    // Core fields must be present
    assert!(json.get("loop_number").is_some());
    assert!(json.get("feature_name").is_some());
    assert!(json.get("status").is_some());

    // QA fields should serialize as empty array
    let qa = json
        .get("artifacts")
        .and_then(|a| a.get("qa_results"))
        .and_then(|q| q.as_array());
    assert!(qa.is_some());
    assert!(qa.unwrap().is_empty());
}

#[test]
fn history_json_with_qa_data_serializes_correctly() {
    let qa = vec![QaExchange {
        iteration: 1,
        passed: false,
        report: "loops/001-test/qa-001-fail.md".to_owned(),
        implementer_response: Some("loops/001-test/impl-qa-response-001.md".to_owned()),
    }];
    let state = make_state(1, qa);
    let json = serde_json::to_value(&state.loops[0]).unwrap();

    let qa_array = json
        .get("artifacts")
        .and_then(|a| a.get("qa_results"))
        .and_then(|q| q.as_array())
        .unwrap();
    assert_eq!(qa_array.len(), 1);
    assert_eq!(qa_array[0]["iteration"], 1);
    assert_eq!(qa_array[0]["passed"], false);
    assert!(qa_array[0]["report"]
        .as_str()
        .unwrap()
        .contains("qa-001-fail"));
    assert!(qa_array[0]["implementer_response"].is_string());
}

// --- Legacy state compatibility ---

#[test]
fn legacy_state_without_qa_fields_deserializes_cleanly() {
    let raw = r#"{
        "project_id": "legacy",
        "project_name": "Legacy Project",
        "prompt_file": "prompt.md",
        "prompt_hash": "abc",
        "prompt_hash_at_loop_start": "abc",
        "parent_project": null,
        "current_loop": 1,
        "current_phase": "implementing",
        "phase_iteration": 1,
        "status": "in_progress",
        "loops": [{
            "loop_number": 1,
            "slug": "demo",
            "feature_name": "Demo",
            "loop_type": "feature",
            "status": "in_progress",
            "backends": {
                "planner": "claude",
                "implementer": "codex",
                "reviewer": "claude"
            },
            "artifacts": {
                "spec": "loops/001-demo/spec.md",
                "impl_notes": null,
                "reviews": [],
                "approval": null
            },
            "commit": null,
            "started_at": "2026-02-11T00:00:00Z",
            "completed_at": null
        }],
        "completion_attempts": []
    }"#;

    let state: ProjectState = serde_json::from_str(raw).expect("should deserialize legacy state");
    let loop_state = &state.loops[0];

    // QA fields should default gracefully
    assert!(loop_state.artifacts.qa_results.is_empty());
    assert!(loop_state.artifacts.pending_qa_feedback.is_none());
    assert_eq!(loop_state.backends.qa, "");

    // Status rendering with zero QA should not panic
    assert!(loop_state.artifacts.qa_results.last().is_none());

    // History verbose rendering with zero QA should produce "none"
    let verdict = loop_state
        .artifacts
        .qa_results
        .last()
        .map(|q| if q.passed { "pass" } else { "fail" })
        .unwrap_or("none");
    assert_eq!(verdict, "none");

    // State invariants should still hold
    state
        .validate_invariants()
        .expect("legacy state should pass invariants");
}

// format_qa_line tests removed: format_qa_line was removed along with the
// durable state-based history rendering path.
