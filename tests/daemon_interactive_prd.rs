//! Integration tests for the daemon interactive PRD workflow.
//!
//! These tests exercise state persistence, label conflict behavior, and
//! idempotent restart handling without requiring live GitHub API access.

use ralph::daemon::interactive_prd::{
    detect_approval, has_prd_label, prd_marker, prd_status_failed_marker, InteractivePrdState,
    PrdWorkflowState, PRD_LABELS, PRD_LABEL_NAMES,
};
use ralph::validate::assertions::assert_exit_code;
use ralph::validate::harness::RalphHarness;
use std::fs;
use std::path::PathBuf;
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
    let loaded =
        InteractivePrdState::load(tmp.path(), "acme", "widgets", 999).expect("load should succeed");
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
    assert_eq!(prd_marker(42, "draft", 3), "<!-- ralph:prd:42:draft-v3 -->");
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

#[test]
fn awaiting_answers_transition_posts_draft_and_persists_feedback_state() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script(
            "prd_writer_reviewer.sh",
            r#"#!/bin/sh
INPUT="$(cat)"
if echo "$INPUT" | grep -q "reviewing an engineering specification"; then
  cat <<'EOF'
```json
{"approved": true, "issues": []}
```
EOF
  exit 0
fi

cat <<'EOF'
## Summary
Draft summary.

## Acceptance Criteria
- [ ] Include a draft flow.

## Technical Approach
Use the interactive PRD daemon transition.

## Files & Modules
- src/daemon/interactive_prd.rs

## Testing Strategy
- daemon integration test

## Out of Scope
- webhook support
EOF
"#,
        )
        .expect("write backend script");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("77.json");
    fs::create_dir_all(state_path.parent().expect("state path should have parent"))
        .expect("create state dir");

    let seed_state = serde_json::json!({
        "issue_number": 77,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingAnswers",
        "question_revision": 1,
        "draft_revision": 0,
        "questions_comment_id": 700,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": null,
        "latest_draft_body": null,
        "user_answers": null,
        "last_processed_comment_id": null,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed_state).expect("serialize seed state"),
    )
    .expect("write state file");

    let draft_comment_log = h.temp_dir.path().join("draft_comment.log");
    let draft_comment_log_str = draft_comment_log.to_string_lossy().into_owned();
    let gh_script = format!(
        r#"#!/bin/sh
DRAFT_COMMENT_LOG="{draft_comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        has_active=0
        has_ready=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd) has_prd=1 ;;
            ralph:prd-active) has_active=1 ;;
            ralph:ready) has_ready=1 ;;
          esac
        done
        if [ "$has_prd" = "1" ]; then
          printf '[]'
        elif [ "$has_active" = "1" ]; then
          printf '[{{"number":77,"title":"Add event-driven PRD","labels":[{{"name":"ralph:prd-active"}}],"body":"Build an iterative engineering spec flow."}}]'
        elif [ "$has_ready" = "1" ]; then
          printf '[]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        want_title_body=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
            title,body) want_title_body=1 ;;
          esac
        done

        if [ "$want_comments" = "1" ]; then
          if [ -f "$DRAFT_COMMENT_LOG" ]; then
            draft_body="$(cat "$DRAFT_COMMENT_LOG" | sed 's/"/\\"/g' | tr '\n' ' ')"
            printf '{{"comments":[{{"id":700,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:77:questions-v1 -->\\n## Clarifying Questions\\n1. Which API shape is required?","createdAt":"2026-01-01T00:00:05Z"}},{{"id":701,"author":{{"login":"alice"}},"body":"Use REST endpoints and include retries.","createdAt":"2026-01-01T00:00:20Z"}},{{"id":702,"author":{{"login":"ralph-bot"}},"body":"%s","createdAt":"2026-01-01T00:00:30Z"}}]}}' "$draft_body"
          else
            printf '{{"comments":[{{"id":700,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:77:questions-v1 -->\\n## Clarifying Questions\\n1. Which API shape is required?","createdAt":"2026-01-01T00:00:05Z"}},{{"id":701,"author":{{"login":"alice"}},"body":"Use REST endpoints and include retries.","createdAt":"2026-01-01T00:00:20Z"}}]}}'
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          printf '{{"title":"Add event-driven PRD","body":"Build an iterative engineering spec flow."}}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body)
              printf '%s' "$2" > "$DRAFT_COMMENT_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      edit)
        exit 0
        ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  label)
    case "$2" in
      create) exit 0 ;;
    esac
    ;;
  repo)
    case "$2" in
      view) printf 'acme/widgets\n'; exit 0 ;;
    esac
    ;;
esac
exit 0
"#
    );
    let gh_path = h
        .write_mock_script("gh", &gh_script)
        .expect("write gh script");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("gh script parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mock_ralph = h
        .write_mock_script(
            "mock_ralph",
            r#"#!/bin/sh
case "$1" in
  auto) exit 0 ;;
  *) exit 1 ;;
esac
"#,
        )
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    let output = h
        .daemon_env(
            [
                "daemon",
                "start",
                "--repo",
                "acme/widgets",
                "--single-iteration",
            ],
            &[("PATH", &path_env), ("RALPH_DAEMON_BIN", &mock_ralph_str)],
        )
        .expect("daemon start should execute");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state should exist after run");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("state should parse");
    assert_eq!(state.state, PrdWorkflowState::AwaitingFeedback);
    assert_eq!(state.draft_revision, 1);
    assert_eq!(state.last_processed_comment_id, Some(701));
    assert_eq!(
        state.user_answers.as_deref(),
        Some("Use REST endpoints and include retries.")
    );
    assert_eq!(state.latest_draft_comment_id, Some(702));
    assert!(state
        .latest_draft_body
        .as_deref()
        .unwrap_or_default()
        .contains("## Summary"));

    let draft_comment = fs::read_to_string(&draft_comment_log).expect("draft comment should exist");
    assert!(
        draft_comment.contains("<!-- ralph:prd:77:draft-v1 -->"),
        "draft marker should be present: {draft_comment}"
    );
    assert!(
        draft_comment.contains("## Draft Engineering Specification (Revision 1)"),
        "draft heading should be present: {draft_comment}"
    );
}

fn ralph_bin_absolute() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_ralph") {
        return PathBuf::from(p);
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set");
    let candidate = PathBuf::from(manifest)
        .join("target")
        .join("debug")
        .join("ralph");
    if candidate.exists() {
        return candidate;
    }

    panic!("ralph binary not found for integration test");
}
