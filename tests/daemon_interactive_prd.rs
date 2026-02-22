//! Integration tests for the daemon interactive PRD workflow.
//!
//! These tests exercise state persistence, label conflict behavior, and
//! idempotent restart handling without requiring live GitHub API access.

use ralph::daemon::interactive_prd::{
    detect_approval, has_prd_label, prd_marker, prd_status_failed_marker, InteractivePrdState,
    PrdWorkflowState, PRD_LABELS, PRD_LABEL_NAMES,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Persistence across simulated restart
// ---------------------------------------------------------------------------

#[test]
fn state_persists_across_simulated_restart() {
    let tmp = TempDir::new().expect("create tempdir");
    let data_dir = tmp.path();

    let mut state = InteractivePrdState::new("acme", "widgets", 42);
    state.state = PrdWorkflowState::AwaitingAnswers;
    state.question_revision = 1;
    state.questions_comment_id = Some(12345);
    state.questions_posted_at = Some(chrono::Utc::now());
    state.last_advanced_at = Some(chrono::Utc::now());
    state.save(data_dir).expect("save state");

    // Simulate daemon restart: load from disk
    let loaded = InteractivePrdState::load(data_dir, "acme", "widgets", 42)
        .expect("load state")
        .expect("state should exist");

    assert_eq!(loaded.state, PrdWorkflowState::AwaitingAnswers);
    assert_eq!(loaded.question_revision, 1);
    assert_eq!(loaded.questions_comment_id, Some(12345));
    assert!(loaded.questions_posted_at.is_some());
}

#[test]
fn state_missing_returns_none() {
    let tmp = TempDir::new().expect("create tempdir");
    let loaded = InteractivePrdState::load(tmp.path(), "acme", "widgets", 999)
        .expect("load should succeed");
    assert!(loaded.is_none());
}

// ---------------------------------------------------------------------------
// Label conflict behavior
// ---------------------------------------------------------------------------

#[test]
fn has_prd_label_detects_all_prd_labels() {
    for &label_name in PRD_LABEL_NAMES {
        let labels = vec![label_name.to_owned()];
        assert!(
            has_prd_label(&labels),
            "has_prd_label should return true for {label_name}"
        );
    }
}

#[test]
fn has_prd_label_returns_false_for_non_prd_labels() {
    let labels = vec![
        "ralph:ready".to_owned(),
        "ralph:in-progress".to_owned(),
        "bug".to_owned(),
    ];
    assert!(!has_prd_label(&labels));
}

#[test]
fn has_prd_label_returns_false_for_empty() {
    assert!(!has_prd_label(&[]));
}

// ---------------------------------------------------------------------------
// Approval detection edge cases
// ---------------------------------------------------------------------------

#[test]
fn approval_detection_case_insensitive() {
    assert!(detect_approval("APPROVED"));
    assert!(detect_approval("Lgtm"));
    assert!(detect_approval("Ship It"));
    assert!(detect_approval("LOOKS GOOD"));
}

#[test]
fn approval_detection_no_false_positives_on_plain_text() {
    assert!(!detect_approval("I need to review this more"));
    assert!(!detect_approval("Please fix the formatting"));
    // "lgtm" as a standalone word should still match even in questions
    assert!(detect_approval("Can you check the lgtm flag?"));
    assert!(detect_approval("This is great, lgtm!"));
}

// ---------------------------------------------------------------------------
// Marker generation
// ---------------------------------------------------------------------------

#[test]
fn prd_marker_format() {
    assert_eq!(
        prd_marker(42, "questions", 1),
        "<!-- ralph:prd:42:questions-v1 -->"
    );
    assert_eq!(
        prd_marker(42, "draft", 3),
        "<!-- ralph:prd:42:draft-v3 -->"
    );
}

#[test]
fn prd_status_failed_marker_format() {
    assert_eq!(
        prd_status_failed_marker(42),
        "<!-- ralph:prd:42:status-failed -->"
    );
}

// ---------------------------------------------------------------------------
// PRD labels constant integrity
// ---------------------------------------------------------------------------

#[test]
fn prd_labels_have_expected_entries() {
    assert_eq!(PRD_LABELS.len(), 5);
    let names: Vec<&str> = PRD_LABELS.iter().map(|(name, _, _)| *name).collect();
    assert!(names.contains(&"ralph:prd"));
    assert!(names.contains(&"ralph:prd-active"));
    assert!(names.contains(&"ralph:prd-approved"));
    assert!(names.contains(&"ralph:prd-done"));
    assert!(names.contains(&"ralph:prd-failed"));
}

// ---------------------------------------------------------------------------
// Terminal state detection
// ---------------------------------------------------------------------------

#[test]
fn terminal_states_are_done_and_failed() {
    let mut state = InteractivePrdState::new("acme", "widgets", 1);

    for (ws, expected) in [
        (PrdWorkflowState::Pending, false),
        (PrdWorkflowState::AwaitingAnswers, false),
        (PrdWorkflowState::AwaitingFeedback, false),
        (PrdWorkflowState::Done, true),
        (PrdWorkflowState::Failed, true),
    ] {
        state.state = ws;
        assert_eq!(state.is_terminal(), expected);
    }
}

// ---------------------------------------------------------------------------
// Error count persistence
// ---------------------------------------------------------------------------

#[test]
fn error_count_persists_across_save_load() {
    let tmp = TempDir::new().expect("create tempdir");
    let data_dir = tmp.path();

    let mut state = InteractivePrdState::new("acme", "widgets", 10);
    state.error_count = 2;
    state.last_error = Some("timeout".to_owned());
    state.save(data_dir).expect("save state");

    let loaded = InteractivePrdState::load(data_dir, "acme", "widgets", 10)
        .expect("load state")
        .expect("state should exist");

    assert_eq!(loaded.error_count, 2);
    assert_eq!(loaded.last_error.as_deref(), Some("timeout"));
}
