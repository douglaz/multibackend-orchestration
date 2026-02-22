//! Integration tests for the daemon interactive PRD workflow.
//!
//! These tests exercise state persistence, label conflict behavior, and
//! idempotent restart handling without requiring live GitHub API access.

use chrono;
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

// ---------------------------------------------------------------------------
// Feedback revision loop: AwaitingFeedback -> AwaitingFeedback (revision)
// ---------------------------------------------------------------------------

#[test]
fn awaiting_feedback_revision_produces_incremented_draft() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    // Mock backend: writer produces revised spec, reviewer approves
    let backend_script = h
        .write_mock_script(
            "prd_feedback_revision.sh",
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
Revised summary based on feedback.

## Acceptance Criteria
- [ ] Updated criteria per feedback.

## Technical Approach
Revised approach with error handling.

## Files & Modules
- src/daemon/interactive_prd.rs

## Testing Strategy
- Updated test plan.

## Out of Scope
- webhooks
EOF
"#,
        )
        .expect("write backend script");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    // Seed state as AwaitingFeedback with draft_revision=1
    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("88.json");
    fs::create_dir_all(state_path.parent().expect("state path parent"))
        .expect("create state dir");

    let seed_state = serde_json::json!({
        "issue_number": 88,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 800,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 802,
        "latest_draft_body": "## Summary\nOriginal draft.\n\n## Acceptance Criteria\n- [ ] AC1\n\n## Technical Approach\nOriginal.\n\n## Files & Modules\n- file.rs\n\n## Testing Strategy\n- tests\n\n## Out of Scope\n- none",
        "user_answers": "Use REST endpoints.",
        "last_processed_comment_id": 801,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed_state).expect("serialize"),
    )
    .expect("write state file");

    let revision_comment_log = h.temp_dir.path().join("revision_comment.log");
    let revision_comment_log_str = revision_comment_log.to_string_lossy().into_owned();
    let gh_script = format!(
        r#"#!/bin/sh
REVISION_LOG="{revision_comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        has_active=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd) has_prd=1 ;;
            ralph:prd-active) has_active=1 ;;
          esac
        done
        if [ "$has_prd" = "1" ]; then
          printf '[]'
        elif [ "$has_active" = "1" ]; then
          printf '[{{"number":88,"title":"Feedback test","labels":[{{"name":"ralph:prd-active"}}],"body":"Test feedback loop."}}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          printf '{{"comments":[{{"id":800,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":801,"author":{{"login":"alice"}},"body":"Use REST endpoints.","createdAt":"2026-01-01T00:00:10Z"}},{{"id":802,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:88:draft-v1 -->\\nDraft v1","createdAt":"2026-01-01T00:00:15Z"}},{{"id":803,"author":{{"login":"bob"}},"body":"Please add error handling details.","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'
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
              printf '%s' "$2" > "$REVISION_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      edit) exit 0 ;;
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
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac
exit 0
"#
    );
    let gh_path = h
        .write_mock_script("gh", &gh_script)
        .expect("write gh script");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script(
            "mock_ralph",
            "#!/bin/sh\ncase \"$1\" in\n  auto) exit 0 ;;\n  *) exit 1 ;;\nesac\n",
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

    let state_raw = fs::read_to_string(&state_path).expect("state should exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("state should parse");

    // Should still be AwaitingFeedback (revision, not approval)
    assert_eq!(state.state, PrdWorkflowState::AwaitingFeedback);
    assert_eq!(state.draft_revision, 2, "draft_revision should be incremented to 2");
    assert_eq!(state.last_processed_comment_id, Some(803));
    assert!(
        state.latest_draft_body.as_deref().unwrap_or_default().contains("## Summary"),
        "revised draft should contain spec sections"
    );

    let revision_body = fs::read_to_string(&revision_comment_log).unwrap_or_default();
    assert!(
        revision_body.contains("<!-- ralph:prd:88:draft-v2 -->"),
        "revision comment should contain draft-v2 marker: {revision_body}"
    );
}

// ---------------------------------------------------------------------------
// Approval by comment: AwaitingFeedback -> Done
// ---------------------------------------------------------------------------

#[test]
fn awaiting_feedback_approval_by_comment_transitions_to_done() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    // Backend not needed for approval path (no revision), but must exist
    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("90.json");
    fs::create_dir_all(state_path.parent().expect("parent"))
        .expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 90,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 900,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 902,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 901,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let approval_log = h.temp_dir.path().join("approval_comment.log");
    let approval_log_str = approval_log.to_string_lossy().into_owned();
    let label_log = h.temp_dir.path().join("approval_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();
    let gh_script = format!(
        r#"#!/bin/sh
APPROVAL_LOG="{approval_log_str}"
LABEL_LOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        has_active=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd) has_prd=1 ;;
            ralph:prd-active) has_active=1 ;;
          esac
        done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":90,"title":"Approval test","labels":[{{"name":"ralph:prd-active"}}],"body":"Test."}}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          if [ -f "$APPROVAL_LOG" ]; then
            printf '{{"comments":[{{"id":900,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":901,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":902,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:90:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":903,"author":{{"login":"alice"}},"body":"LGTM, ship it!","createdAt":"2026-01-01T00:00:25Z"}},{{"id":904,"author":{{"login":"ralph-bot"}},"body":"approved marker","createdAt":"2026-01-01T00:00:30Z"}}]}}'
          else
            printf '{{"comments":[{{"id":900,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":901,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":902,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:90:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":903,"author":{{"login":"alice"}},"body":"LGTM, ship it!","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'
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
              printf '%s' "$2" > "$APPROVAL_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      edit)
        echo "$@" >> "$LABEL_LOG"
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
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label) exit 0 ;;
esac
exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
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

    let state_raw = fs::read_to_string(&state_path).expect("state should exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");
    assert_eq!(state.state, PrdWorkflowState::Done, "state should be Done after approval");
    assert!(state.is_terminal());

    // Verify approval comment was posted
    let approval_body = fs::read_to_string(&approval_log).unwrap_or_default();
    assert!(
        approval_body.contains("<!-- ralph:prd:90:status-approved-v1 -->"),
        "approval marker should be posted: {approval_body}"
    );

    // Verify label changes
    let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
    assert!(
        label_raw.contains("ralph:prd-done"),
        "ralph:prd-done should have been added: {label_raw}"
    );
}

// ---------------------------------------------------------------------------
// Approval by label: AwaitingFeedback -> Done via ralph:prd-approved label
// ---------------------------------------------------------------------------

#[test]
fn awaiting_feedback_approval_by_label_transitions_to_done() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("91.json");
    fs::create_dir_all(state_path.parent().expect("parent"))
        .expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 91,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 2,
        "questions_comment_id": 910,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 913,
        "latest_draft_body": "## Summary\nDraft v2.",
        "user_answers": "answers",
        "last_processed_comment_id": 912,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let comment_log = h.temp_dir.path().join("label_approval_comment.log");
    let comment_log_str = comment_log.to_string_lossy().into_owned();
    // gh returns ralph:prd-approved in labels
    let gh_script = format!(
        r#"#!/bin/sh
COMMENT_LOG="{comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do
          case "$arg" in ralph:prd-active) has_active=1 ;; esac
        done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":91,"title":"Label approval","labels":[{{"name":"ralph:prd-active"}},{{"name":"ralph:prd-approved"}}],"body":"Test."}}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          if [ -f "$COMMENT_LOG" ]; then
            printf '{{"comments":[{{"id":910,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":911,"author":{{"login":"alice"}},"body":"a","createdAt":"2026-01-01T00:00:10Z"}},{{"id":912,"author":{{"login":"alice"}},"body":"feedback","createdAt":"2026-01-01T00:00:15Z"}},{{"id":913,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:20Z"}},{{"id":914,"author":{{"login":"ralph-bot"}},"body":"approved","createdAt":"2026-01-01T00:00:30Z"}}]}}'
          else
            printf '{{"comments":[{{"id":910,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":911,"author":{{"login":"alice"}},"body":"a","createdAt":"2026-01-01T00:00:10Z"}},{{"id":912,"author":{{"login":"alice"}},"body":"feedback","createdAt":"2026-01-01T00:00:15Z"}},{{"id":913,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:20Z"}}]}}'
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}},{{"name":"ralph:prd-approved"}}]}}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) printf '%s' "$2" > "$COMMENT_LOG" ; shift 2 ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      edit) exit 0 ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then printf 'ralph-bot\n' ; exit 0 ; fi
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac
exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
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
        .expect("daemon start");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state should exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");
    assert_eq!(state.state, PrdWorkflowState::Done, "should be Done after label approval");
    assert_eq!(state.draft_revision, 2, "draft_revision should remain 2");

    let comment_body = fs::read_to_string(&comment_log).unwrap_or_default();
    assert!(
        comment_body.contains("<!-- ralph:prd:91:status-approved-v2 -->"),
        "approval marker should reference draft v2: {comment_body}"
    );
}

// ---------------------------------------------------------------------------
// Mixed comments: approval + non-approval feedback -> Done (not revision)
// ---------------------------------------------------------------------------

#[test]
fn awaiting_feedback_mixed_comments_approval_plus_feedback_transitions_to_done() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    // Backend not needed for approval path (no revision), but must exist
    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("95.json");
    fs::create_dir_all(state_path.parent().expect("parent"))
        .expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 95,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 950,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 952,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 951,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let approval_log = h.temp_dir.path().join("mixed_approval_comment.log");
    let approval_log_str = approval_log.to_string_lossy().into_owned();
    // Two new comments: one non-approval feedback, one approval
    let gh_script = format!(
        r#"#!/bin/sh
APPROVAL_LOG="{approval_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do
          case "$arg" in ralph:prd-active) has_active=1 ;; esac
        done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":95,"title":"Mixed test","labels":[{{"name":"ralph:prd-active"}}],"body":"Test."}}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          if [ -f "$APPROVAL_LOG" ]; then
            printf '{{"comments":[{{"id":950,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":951,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":952,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:95:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":953,"author":{{"login":"bob"}},"body":"Please fix the error handling section.","createdAt":"2026-01-01T00:00:25Z"}},{{"id":954,"author":{{"login":"alice"}},"body":"LGTM, ship it!","createdAt":"2026-01-01T00:00:30Z"}},{{"id":955,"author":{{"login":"ralph-bot"}},"body":"approved marker","createdAt":"2026-01-01T00:00:35Z"}}]}}'
          else
            printf '{{"comments":[{{"id":950,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":951,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":952,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:95:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":953,"author":{{"login":"bob"}},"body":"Please fix the error handling section.","createdAt":"2026-01-01T00:00:25Z"}},{{"id":954,"author":{{"login":"alice"}},"body":"LGTM, ship it!","createdAt":"2026-01-01T00:00:30Z"}}]}}'
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'
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
              printf '%s' "$2" > "$APPROVAL_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      edit) exit 0 ;;
    esac
    ;;
  api)
    if [ "$2" = "user" ]; then printf 'ralph-bot\n' ; exit 0 ; fi
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac
exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
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
        .expect("daemon start");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state should exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");

    // Key assertion: mixed comments (approval + non-approval) should trigger Done, not revision
    assert_eq!(
        state.state,
        PrdWorkflowState::Done,
        "mixed comments with any approval should transition to Done, not revision"
    );
    assert!(state.is_terminal());
    assert_eq!(state.draft_revision, 1, "draft_revision should remain 1 (no revision generated)");

    // Verify approval comment was posted
    let approval_body = fs::read_to_string(&approval_log).unwrap_or_default();
    assert!(
        approval_body.contains("<!-- ralph:prd:95:status-approved-v1 -->"),
        "approval marker should be posted: {approval_body}"
    );
}

// ---------------------------------------------------------------------------
// Multi-tick end-to-end: Pending -> AwaitingAnswers -> AwaitingFeedback -> Done
// ---------------------------------------------------------------------------

#[test]
fn multi_tick_pending_to_done_end_to_end() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    // Mock backend: produces questions for tick 1, draft for tick 2, reviewer approves
    let backend_script = h
        .write_mock_script(
            "prd_e2e_backend.sh",
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

if echo "$INPUT" | grep -q "merge and deduplicate"; then
  printf '1. What API?\n2. What errors?\n3. What scope?\n'
  exit 0
fi

if echo "$INPUT" | grep -q "engineering specification analyst"; then
  printf '1. What API?\n2. What errors?\n'
  exit 0
fi

cat <<'EOF'
## Summary
E2E draft spec.

## Acceptance Criteria
- [ ] End-to-end flow works.

## Technical Approach
Multi-tick daemon approach.

## Files & Modules
- src/daemon/interactive_prd.rs

## Testing Strategy
- E2E integration test.

## Out of Scope
- webhooks
EOF
"#,
        )
        .expect("write backend script");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    // State file path
    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("100.json");

    // Track which tick we're on via a counter file
    let tick_file = h.temp_dir.path().join("e2e_tick.txt");
    let tick_file_str = tick_file.to_string_lossy().into_owned();
    let comment_log = h.temp_dir.path().join("e2e_comment.log");
    let comment_log_str = comment_log.to_string_lossy().into_owned();
    let label_log = h.temp_dir.path().join("e2e_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // Use runtime-relative timestamps so the test is time-agnostic.
    // Tick 1 questions get a timestamp slightly in the past, tick 2 answer
    // is after that, tick 3 draft and approval are after that.
    // The daemon fetches comment metadata to get questions_posted_at, and
    // all mock timestamps are relative to a fixed base that's always "recent".
    // Compute runtime-relative timestamps — all mock times will be relative to "now"
    let base_ts = chrono::Utc::now();
    // Questions comment: base - 30s (always in the past)
    let ts_questions = (base_ts - chrono::Duration::seconds(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // Answer comment: base - 10s (after questions)
    let ts_answer = (base_ts - chrono::Duration::seconds(10))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // Draft comment: base - 5s (after answer)
    let ts_draft = (base_ts - chrono::Duration::seconds(5))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // Approval comment: base - 1s (after draft)
    let ts_approval = (base_ts - chrono::Duration::seconds(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let gh_script = format!(
        r#"#!/bin/sh
TICK_FILE="{tick_file_str}"
COMMENT_LOG="{comment_log_str}"
LABEL_LOG="{label_log_str}"

# Runtime-relative timestamps
TS_QUESTIONS="{ts_questions}"
TS_ANSWER="{ts_answer}"
TS_DRAFT="{ts_draft}"
TS_APPROVAL="{ts_approval}"

# Read tick number (default 1)
if [ -f "$TICK_FILE" ]; then
  TICK=$(cat "$TICK_FILE")
else
  TICK=1
fi

case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        has_active=0
        for arg in "$@"; do
          case "$arg" in
            ralph:prd) has_prd=1 ;;
            ralph:prd-active) has_active=1 ;;
          esac
        done
        if [ "$TICK" = "1" ] && [ "$has_prd" = "1" ]; then
          printf '[{{"number":100,"title":"E2E PRD","labels":[{{"name":"ralph:prd"}}],"body":"Build a multi-tick flow."}}]'
        elif [ "$has_active" = "1" ]; then
          printf '[{{"number":100,"title":"E2E PRD","labels":[{{"name":"ralph:prd-active"}}],"body":"Build a multi-tick flow."}}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          if [ "$TICK" = "1" ]; then
            # No comments yet (or just the questions we posted)
            if [ -f "$COMMENT_LOG" ]; then
              printf '{{"comments":[{{"id":1001,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:100:questions-v1 -->\\nQuestions","createdAt":"%s"}}]}}' "$TS_QUESTIONS"
            else
              printf '{{"comments":[]}}'
            fi
          elif [ "$TICK" = "2" ]; then
            # Questions posted, user answered
            printf '{{"comments":[{{"id":1001,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:100:questions-v1 -->\\n## Clarifying Questions\\n1. What API?","createdAt":"%s"}},{{"id":1002,"author":{{"login":"alice"}},"body":"Use REST with retries.","createdAt":"%s"}}]}}' "$TS_QUESTIONS" "$TS_ANSWER"
          elif [ "$TICK" = "3" ]; then
            # Draft posted, user approves
            printf '{{"comments":[{{"id":1001,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:100:questions-v1 -->\\nQ","createdAt":"%s"}},{{"id":1002,"author":{{"login":"alice"}},"body":"Use REST.","createdAt":"%s"}},{{"id":1003,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:100:draft-v1 -->\\nDraft","createdAt":"%s"}},{{"id":1004,"author":{{"login":"alice"}},"body":"LGTM, ship it!","createdAt":"%s"}}]}}' "$TS_QUESTIONS" "$TS_ANSWER" "$TS_DRAFT" "$TS_APPROVAL"
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'
          exit 0
        fi
        exit 0
        ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body)
              printf '%s\n' "$2" >> "$COMMENT_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
      edit)
        echo "$@" >> "$LABEL_LOG"
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
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label) exit 0 ;;
esac
exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    // Tick 1: Pending -> AwaitingAnswers
    fs::write(&tick_file, "1").expect("write tick 1");
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
        .expect("tick 1");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state after tick 1");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse tick 1");
    assert_eq!(
        state.state,
        PrdWorkflowState::AwaitingAnswers,
        "after tick 1: should be AwaitingAnswers"
    );
    assert_eq!(state.question_revision, 1);

    // Tick 2: AwaitingAnswers -> AwaitingFeedback
    fs::write(&tick_file, "2").expect("write tick 2");
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
        .expect("tick 2");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state after tick 2");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse tick 2");
    assert_eq!(
        state.state,
        PrdWorkflowState::AwaitingFeedback,
        "after tick 2: should be AwaitingFeedback"
    );
    assert_eq!(state.draft_revision, 1);
    assert!(state.latest_draft_body.is_some());

    // Tick 3: AwaitingFeedback -> Done (approval)
    fs::write(&tick_file, "3").expect("write tick 3");
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
        .expect("tick 3");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state after tick 3");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse tick 3");
    assert_eq!(
        state.state,
        PrdWorkflowState::Done,
        "after tick 3: should be Done"
    );
    assert!(state.is_terminal());

    // Verify approval was posted
    let comment_body = fs::read_to_string(&comment_log).unwrap_or_default();
    assert!(
        comment_body.contains("<!-- ralph:prd:100:status-approved-v1 -->"),
        "approval marker should be posted: {comment_body}"
    );

    // Verify labels
    let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
    assert!(
        label_raw.contains("ralph:prd-done"),
        "ralph:prd-done should be added: {label_raw}"
    );
}

// ---------------------------------------------------------------------------
// Pre-draft comment exclusion regression test
// ---------------------------------------------------------------------------

#[test]
fn pre_draft_comments_excluded_from_feedback_in_awaiting_feedback() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    // Backend not needed — no revision should be triggered
    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme")
        .join("widgets")
        .join(".ralph")
        .join("interactive-prd")
        .join("110.json");
    fs::create_dir_all(state_path.parent().expect("parent"))
        .expect("create state dir");

    // Seed: AwaitingFeedback, draft at id=1103, cursor at id=1100
    // There's a user comment (1102) that's pre-draft but post-cursor
    let seed = serde_json::json!({
        "issue_number": 110,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 1100,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 1103,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 1100,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    // gh returns comments with pre-draft user comment (id 1102) that has
    // approval text — this should be IGNORED because it's pre-draft
    let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":110,"title":"Pre-draft test","labels":[{"name":"ralph:prd-active"}],"body":"Test."}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{"comments":[{"id":1100,"author":{"login":"ralph-bot"},"body":"questions","createdAt":"2026-01-01T00:00:05Z"},{"id":1101,"author":{"login":"alice"},"body":"answers","createdAt":"2026-01-01T00:00:10Z"},{"id":1102,"author":{"login":"alice"},"body":"LGTM, approved!","createdAt":"2026-01-01T00:00:12Z"},{"id":1103,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:110:draft-v1 -->\nDraft","createdAt":"2026-01-01T00:00:15Z"}]}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{"labels":[{"name":"ralph:prd-active"}]}'; exit 0; fi
        exit 0 ;;
      comment) exit 0 ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#;
    let gh_path = h.write_mock_script("gh", gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
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
        .expect("daemon start");
    assert_exit_code(&output, 0);

    let state_raw = fs::read_to_string(&state_path).expect("state should exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");

    // The pre-draft "LGTM, approved!" comment should NOT have triggered Done.
    // State should remain AwaitingFeedback because no post-draft comments exist.
    assert_eq!(
        state.state,
        PrdWorkflowState::AwaitingFeedback,
        "pre-draft approval comment should be ignored; state should remain AwaitingFeedback"
    );
    assert_eq!(state.draft_revision, 1, "no revision should have happened");
}

// ---------------------------------------------------------------------------
// Bot-login retry exhaustion in AwaitingAnswers
// ---------------------------------------------------------------------------

#[test]
fn awaiting_answers_bot_login_failure_exhaustion_transitions_to_failed() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme/widgets/.ralph/interactive-prd/120.json");
    fs::create_dir_all(state_path.parent().expect("parent")).expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 120,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingAnswers",
        "question_revision": 1,
        "draft_revision": 0,
        "questions_comment_id": 1200,
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
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let label_log = h.temp_dir.path().join("bot_login_aa_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // gh mock: `gh api user` always fails; everything else works normally
    let gh_script = format!(
        r#"#!/bin/sh
LLOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":120,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":1200,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":1201,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:20Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api)
    if [ "$2" = "user" ]; then
      echo "gh: error resolving authenticated user" >&2
      exit 1
    fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    // Run 3 ticks — each should fail due to bot-login error
    for tick in 1..=3 {
        let _output = h
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
            .expect("daemon start");

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
                .unwrap_or_else(|e| panic!("parse state after tick {tick}: {e}"));

        if tick < 3 {
            assert_eq!(
                state.state,
                PrdWorkflowState::AwaitingAnswers,
                "tick {tick}: should remain AwaitingAnswers"
            );
            assert_eq!(
                state.error_count, tick as u32,
                "tick {tick}: error_count should be {tick}"
            );
            assert!(
                state.last_error.is_some(),
                "tick {tick}: last_error should be set"
            );
        } else {
            assert_eq!(
                state.state,
                PrdWorkflowState::Failed,
                "tick 3: should be Failed after bot-login exhaustion"
            );
            assert!(state.is_terminal());
            assert!(state.error_count >= 3);
        }
    }

    // Verify ralph:prd-failed label was applied
    let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
    assert!(
        label_raw.contains("ralph:prd-failed"),
        "ralph:prd-failed label should be added: {label_raw}"
    );
}

// ---------------------------------------------------------------------------
// Bot-login retry exhaustion in AwaitingFeedback
// ---------------------------------------------------------------------------

#[test]
fn awaiting_feedback_bot_login_failure_exhaustion_transitions_to_failed() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme/widgets/.ralph/interactive-prd/130.json");
    fs::create_dir_all(state_path.parent().expect("parent")).expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 130,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 1300,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 1302,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 1301,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let label_log = h.temp_dir.path().join("bot_login_af_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // gh mock: `gh api user` always fails
    let gh_script = format!(
        r#"#!/bin/sh
LLOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":130,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":1300,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":1301,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":1302,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":1303,"author":{{"login":"alice"}},"body":"fix the testing section","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api)
    if [ "$2" = "user" ]; then
      echo "gh: error resolving authenticated user" >&2
      exit 1
    fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    // Run 3 ticks — each should fail due to bot-login error
    for tick in 1..=3 {
        let _output = h
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
            .expect("daemon start");

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
                .unwrap_or_else(|e| panic!("parse state after tick {tick}: {e}"));

        if tick < 3 {
            assert_eq!(
                state.state,
                PrdWorkflowState::AwaitingFeedback,
                "tick {tick}: should remain AwaitingFeedback"
            );
            assert_eq!(
                state.error_count, tick as u32,
                "tick {tick}: error_count should be {tick}"
            );
            assert!(
                state.last_error.is_some(),
                "tick {tick}: last_error should be set"
            );
        } else {
            assert_eq!(
                state.state,
                PrdWorkflowState::Failed,
                "tick 3: should be Failed after bot-login exhaustion"
            );
            assert!(state.is_terminal());
            assert!(state.error_count >= 3);
        }
    }

    // Verify ralph:prd-failed label was applied
    let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
    assert!(
        label_raw.contains("ralph:prd-failed"),
        "ralph:prd-failed label should be added: {label_raw}"
    );
}

// ---------------------------------------------------------------------------
// Bot-login retry exhaustion in Pending stage
// ---------------------------------------------------------------------------

#[test]
fn pending_bot_login_failure_exhaustion_transitions_to_failed() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let label_log = h.temp_dir.path().join("bot_login_pending_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // gh mock: `gh api user` always fails; issue #130 starts fresh (Pending, no state file)
    let gh_script = format!(
        r#"#!/bin/sh
LLOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0; has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd) has_prd=1 ;; ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_prd" = "1" ]; then
          printf '[{{"number":130,"title":"T","labels":[{{"name":"ralph:prd"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0; want_tb=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; title,body) want_tb=1 ;; esac; done
        if [ "$want_c" = "1" ]; then printf '{{"comments":[]}}'; exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd"}}]}}'; exit 0; fi
        if [ "$want_tb" = "1" ]; then printf '{{"title":"T","body":"B"}}'; exit 0; fi
        exit 0 ;;
      comment) exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api)
    if [ "$2" = "user" ]; then
      echo "gh: error resolving authenticated user" >&2
      exit 1
    fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    let state_path = h
        .temp_dir
        .path()
        .join("acme/widgets/.ralph/interactive-prd/130.json");

    // Run 3 ticks — each should fail due to bot-login error
    for tick in 1..=3 {
        let _output = h
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
            .expect("daemon start");

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
                .unwrap_or_else(|e| panic!("parse state after tick {tick}: {e}"));

        if tick < 3 {
            assert_eq!(
                state.state,
                PrdWorkflowState::Pending,
                "tick {tick}: should remain Pending"
            );
            assert_eq!(
                state.error_count, tick as u32,
                "tick {tick}: error_count should be {tick}"
            );
            assert!(
                state.last_error.is_some(),
                "tick {tick}: last_error should be set"
            );
        } else {
            assert_eq!(
                state.state,
                PrdWorkflowState::Failed,
                "tick 3: should be Failed after bot-login exhaustion"
            );
            assert!(state.is_terminal());
            assert!(state.error_count >= 3);
        }
    }

    // Verify ralph:prd-failed label was applied
    let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
    assert!(
        label_raw.contains("ralph:prd-failed"),
        "ralph:prd-failed label should be added: {label_raw}"
    );
}

// ---------------------------------------------------------------------------
// Approval label-swap partial-failure recovery
// ---------------------------------------------------------------------------

#[test]
fn approval_label_swap_partial_failure_keeps_state_nonterminal() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_path = h
        .temp_dir
        .path()
        .join("acme/widgets/.ralph/interactive-prd/140.json");
    fs::create_dir_all(state_path.parent().expect("parent")).expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 140,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 1400,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 1402,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 1401,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let comment_log = h.temp_dir.path().join("partial_label_comment.log");
    let comment_log_str = comment_log.to_string_lossy().into_owned();
    let label_log = h.temp_dir.path().join("partial_label_labels.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // gh mock: `issue comment` succeeds (approval comment can be posted) but
    // `issue edit --add-label ralph:prd-done` always fails (simulating partial
    // label failure). The issue retains ralph:prd-active so it remains poll-visible.
    let gh_script = format!(
        r#"#!/bin/sh
CLOG="{comment_log_str}"
LLOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":140,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":1400,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":1401,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":1402,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:140:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":1403,"author":{{"login":"alice"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) printf '%s' "$2" >> "$CLOG"; shift 2 ;;
            *) shift ;;
          esac
        done
        exit 0 ;;
      edit)
        # Fail on add-label ralph:prd-done (the first label op in boundary-safe order)
        for arg in "$@"; do
          case "$arg" in
            *ralph:prd-done*)
              echo "gh: error adding label ralph:prd-done" >&2
              exit 1 ;;
          esac
        done
        echo "$@" >> "$LLOG"
        exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    // Run 1 tick — label add fails, state should NOT be Done
    let _output = h
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
        .expect("daemon start");

    let state: InteractivePrdState =
        serde_json::from_str(&fs::read_to_string(&state_path).expect("read state"))
            .expect("parse state");

    // State must remain AwaitingFeedback (not Done) since label add failed
    assert_eq!(
        state.state,
        PrdWorkflowState::AwaitingFeedback,
        "state should remain AwaitingFeedback on partial label failure"
    );
    assert!(
        state.error_count >= 1,
        "error_count should be incremented: {}",
        state.error_count
    );
    assert!(
        state.last_error.is_some(),
        "last_error should be set on label failure"
    );
    // Done is NOT persisted — approval comment was posted (idempotent) but
    // ralph:prd-active is still present so the issue remains poll-visible.
}

// ---------------------------------------------------------------------------
// Terminal save-failure recovery: approval path
// ---------------------------------------------------------------------------

/// When state save fails during the Done transition, the issue must remain
/// in AwaitingFeedback (poll-visible via ralph:prd-active) so it can be
/// retried.  The save failure is routed through retry accounting.
#[test]
fn terminal_save_failure_approval_path_keeps_issue_retryable() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    let state_dir = h.temp_dir.path().join("acme/widgets/.ralph/interactive-prd");
    fs::create_dir_all(&state_dir).expect("create state dir");

    let state_path = state_dir.join("150.json");
    let seed = serde_json::json!({
        "issue_number": 150,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 1500,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 1502,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 1501,
        "error_count": 0,
        "last_error": null,
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let label_log = h.temp_dir.path().join("save_fail_approval_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // gh mock: approval comment present; save failure injected via env var.
    let gh_script = format!(
        r#"#!/bin/sh
LLOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":150,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":1500,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":1501,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":1502,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:150:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":1503,"author":{{"login":"alice"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    // Inject save failure via env var — deterministic regardless of privilege level
    // Run one tick — save should fail, state should remain AwaitingFeedback
    let _output = h
        .daemon_env(
            [
                "daemon",
                "start",
                "--repo",
                "acme/widgets",
                "--single-iteration",
            ],
            &[
                ("PATH", &path_env),
                ("RALPH_DAEMON_BIN", &mock_ralph_str),
                ("RALPH_TEST_INJECT_SAVE_FAILURE", "1"),
            ],
        )
        .expect("daemon start");

    // State file should still exist with pre-transition content because save failed
    let state_raw = fs::read_to_string(&state_path).expect("state should still exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");

    // The state should NOT be Done (save failed, so terminal state not persisted)
    assert_ne!(
        state.state,
        PrdWorkflowState::Done,
        "state should not be Done when save fails"
    );
    // The issue should still be retryable (AwaitingFeedback or with error_count > 0)
    assert!(
        state.state == PrdWorkflowState::AwaitingFeedback || state.error_count > 0,
        "issue should remain retryable: state={:?}, error_count={}",
        state.state,
        state.error_count
    );
}

// ---------------------------------------------------------------------------
// User marker spoof: bot-scoped marker posting still works correctly
// ---------------------------------------------------------------------------

/// When a user spoofs a PRD marker comment (e.g. copying the exact questions
/// marker text), the bot should still post its own marker comment and hydrate
/// state from the bot-authored comment, not the user spoof.
#[test]
fn user_marker_spoof_does_not_block_bot_marker_posting() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    // Backend that produces valid questions and spec
    let backend_script = h
        .write_mock_script(
            "prd_spoof_backend.sh",
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

if echo "$INPUT" | grep -q "merge and deduplicate"; then
  printf '1. What API?\n2. What errors?\n'
  exit 0
fi

if echo "$INPUT" | grep -q "engineering specification analyst"; then
  printf '1. What API?\n2. What errors?\n'
  exit 0
fi

cat <<'EOF'
## Summary
Draft from spoof test.

## Acceptance Criteria
- [ ] AC1

## Technical Approach
Approach.

## Files & Modules
- file.rs

## Testing Strategy
- tests

## Out of Scope
- none
EOF
"#,
        )
        .expect("write backend script");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    // Seed: AwaitingAnswers state. The gh mock returns a user spoof marker
    // comment alongside the real bot question comment.
    let state_path = h
        .temp_dir
        .path()
        .join("acme/widgets/.ralph/interactive-prd/160.json");
    fs::create_dir_all(state_path.parent().expect("parent")).expect("create state dir");

    let seed = serde_json::json!({
        "issue_number": 160,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingAnswers",
        "question_revision": 1,
        "draft_revision": 0,
        "questions_comment_id": 1601,
        "questions_posted_at": "2026-01-01T00:00:10Z",
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
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let draft_comment_log = h.temp_dir.path().join("spoof_draft_comment.log");
    let draft_comment_log_str = draft_comment_log.to_string_lossy().into_owned();

    // gh mock: includes a user-spoofed questions marker comment BEFORE the bot comment.
    // The bot should correctly use its own comment for hydration.
    let gh_script = format!(
        r#"#!/bin/sh
DRAFT_LOG="{draft_comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":160,"title":"Spoof test","labels":[{{"name":"ralph:prd-active"}}],"body":"Spoof test body."}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":1600,"author":{{"login":"mallory"}},"body":"<!-- ralph:prd:160:questions-v1 -->\\nSpoofed questions by user","createdAt":"2026-01-01T00:00:05Z"}},{{"id":1601,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:160:questions-v1 -->\\n## Clarifying Questions\\n1. Real Q from bot","createdAt":"2026-01-01T00:00:10Z"}},{{"id":1602,"author":{{"login":"alice"}},"body":"Real answers from user.","createdAt":"2026-01-01T00:00:20Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in
            --body) printf '%s' "$2" > "$DRAFT_LOG"; shift 2 ;;
            *) shift ;;
          esac
        done; exit 0 ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
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
        .expect("daemon start");
    assert_exit_code(&output, 0);

    // Verify: state should transition to AwaitingFeedback (draft generated successfully)
    let state_raw = fs::read_to_string(&state_path).expect("state should exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");
    assert_eq!(
        state.state,
        PrdWorkflowState::AwaitingFeedback,
        "should transition to AwaitingFeedback despite user spoof"
    );
    assert_eq!(state.draft_revision, 1);
    assert_eq!(state.last_processed_comment_id, Some(1602));

    // Verify: draft comment was posted (bot marker posting works despite spoof)
    let draft_body = fs::read_to_string(&draft_comment_log).unwrap_or_default();
    assert!(
        draft_body.contains("<!-- ralph:prd:160:draft-v1 -->"),
        "draft marker should be posted despite user spoof: {draft_body}"
    );
}

// ---------------------------------------------------------------------------
// Terminal save-failure recovery: failure path
// ---------------------------------------------------------------------------

/// When state save fails during the Failed transition (triggered by retry
/// exhaustion), the issue must remain poll-visible (ralph:prd-active label
/// not removed) so a subsequent daemon tick can retry.
///
/// This mirrors `terminal_save_failure_approval_path_keeps_issue_retryable`
/// but targets the failure-exhaustion path instead.
#[test]
fn terminal_save_failure_failed_path_keeps_issue_retryable() {
    let h =
        RalphHarness::new_daemon(ralph_bin_absolute(), "acme", "widgets").expect("daemon harness");
    h.init_workspace().expect("init workspace");

    let backend_script = h
        .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
        .expect("write backend");
    h.setup_mock_backends_stable(&backend_script)
        .expect("setup mock backends");

    // Seed state with error_count=2 in AwaitingFeedback.  The gh mock will
    // return comments that trigger a revision attempt, and the revision
    // backend will produce an incomplete spec (missing sections) to force
    // an error on this tick.  error_count goes from 2→3, triggering
    // transition_to_failed.  We then make the state dir read-only so the
    // save inside transition_to_failed fails.
    let state_dir = h.temp_dir.path().join("acme/widgets/.ralph/interactive-prd");
    fs::create_dir_all(&state_dir).expect("create state dir");

    let state_path = state_dir.join("155.json");
    let seed = serde_json::json!({
        "issue_number": 155,
        "owner": "acme",
        "repo": "widgets",
        "state": "AwaitingFeedback",
        "question_revision": 1,
        "draft_revision": 1,
        "questions_comment_id": 1550,
        "questions_posted_at": "2026-01-01T00:00:05Z",
        "latest_draft_comment_id": 1552,
        "latest_draft_body": "## Summary\nDraft.",
        "user_answers": "answers",
        "last_processed_comment_id": 1551,
        "error_count": 2,
        "last_error": "previous error",
        "last_advanced_at": null
    });
    fs::write(
        &state_path,
        serde_json::to_string_pretty(&seed).expect("serialize"),
    )
    .expect("write state");

    let label_log = h.temp_dir.path().join("save_fail_failed_label.log");
    let label_log_str = label_log.to_string_lossy().into_owned();

    // gh mock: returns feedback comment to trigger revision, but the backend
    // produces an error (empty output) which will cause section validation to
    // fail, pushing error_count to 3.
    let gh_script = format!(
        r#"#!/bin/sh
LLOG="{label_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":155,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":1550,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":1551,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":1552,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:155:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":1553,"author":{{"login":"alice"}},"body":"Please revise the summary.","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) echo "$@" >> "$LLOG"; exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
    );
    let gh_path = h.write_mock_script("gh", &gh_script).expect("write gh");
    let path_env = format!(
        "{}:{}",
        gh_path.parent().expect("parent").display(),
        std::env::var("PATH").unwrap_or_default()
    );

    // Backend that returns empty output (triggers section validation failure)
    let fail_backend = h
        .write_mock_script("prd_fail_backend.sh", "#!/bin/sh\necho ''\n")
        .expect("write fail backend");
    h.setup_mock_backends_stable(&fail_backend)
        .expect("setup fail backends");

    let mock_ralph = h
        .write_mock_script("mock_ralph", "#!/bin/sh\nexit 0\n")
        .expect("write mock ralph");
    let mock_ralph_str = mock_ralph.to_string_lossy().into_owned();

    // Inject save failure via env var — deterministic regardless of privilege level.
    // Run one tick — backend error pushes error_count to 3, transition_to_failed
    // tries to save but fails, state should remain non-terminal
    let _output = h
        .daemon_env(
            [
                "daemon",
                "start",
                "--repo",
                "acme/widgets",
                "--single-iteration",
            ],
            &[
                ("PATH", &path_env),
                ("RALPH_DAEMON_BIN", &mock_ralph_str),
                ("RALPH_TEST_INJECT_SAVE_FAILURE", "1"),
            ],
        )
        .expect("daemon start");

    // State file should still exist with pre-transition content because save failed
    let state_raw = fs::read_to_string(&state_path).expect("state should still exist");
    let state: InteractivePrdState = serde_json::from_str(&state_raw).expect("parse state");

    // The state should NOT be Failed (save failed, so terminal state not persisted)
    assert_ne!(
        state.state,
        PrdWorkflowState::Failed,
        "state should not be Failed when save fails in transition_to_failed"
    );
    // The issue should still be retryable
    assert!(
        state.state == PrdWorkflowState::AwaitingFeedback,
        "issue should remain in AwaitingFeedback: state={:?}",
        state.state,
    );
}

/// Resolve the absolute path to the `ralph` binary for integration tests.
///
/// Uses a multi-layout strategy that works across Cargo, Nix, and cross-compile
/// environments:
/// 1. Compile-time `CARGO_BIN_EXE_ralph` (set by `cargo test` when the `ralph`
///    binary is a direct dependency of the test harness).
/// 2. Runtime `CARGO_BIN_EXE_ralph` environment variable.
/// 3. `RALPH_TEST_BIN` — explicit override for CI / Nix builds.
/// 4. `CARGO_TARGET_DIR` / `target` relative to `CARGO_MANIFEST_DIR`:
///    - `{root}/{debug,release}/ralph`
///    - `{root}/{triple}/{debug,release}/ralph`
///
/// Panics with a diagnostic message listing all searched paths if no binary is
/// found.
fn ralph_bin_absolute() -> PathBuf {
    // Collect all searched paths for diagnostics on failure
    let mut searched: Vec<String> = Vec::new();

    // 1. Compile-time injection (preferred when available)
    if let Some(p) = option_env!("CARGO_BIN_EXE_ralph") {
        let pb = PathBuf::from(p);
        searched.push(format!("CARGO_BIN_EXE_ralph (compile-time) = {}", pb.display()));
        if pb.exists() {
            return pb;
        }
    }

    // 2. Runtime env (set by `cargo test` for binary targets)
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_ralph") {
        let pb = PathBuf::from(p);
        searched.push(format!("CARGO_BIN_EXE_ralph (runtime) = {}", pb.display()));
        if pb.exists() {
            return pb;
        }
    }

    // 3. Explicit override for Nix / CI
    if let Ok(p) = std::env::var("RALPH_TEST_BIN") {
        let pb = PathBuf::from(p);
        searched.push(format!("RALPH_TEST_BIN = {}", pb.display()));
        if pb.exists() {
            return pb;
        }
    }

    // 4. Probe standard Cargo layouts under target dir

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_owned());
    let target_root = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(&manifest_dir).join("target"));

    // Profiles to check
    let profiles = ["debug", "release"];

    // Direct layout: target/{profile}/ralph
    for profile in &profiles {
        let candidate = target_root.join(profile).join("ralph");
        searched.push(candidate.display().to_string());
        if candidate.exists() {
            return candidate;
        }
    }

    // Cross-compile layout: target/{triple}/{profile}/ralph
    // Detect host triple from CARGO_CFG_TARGET_ARCH + friends, or probe common triples
    let triples: Vec<String> = {
        let mut ts = Vec::new();
        // Try to construct from environment
        if let (Ok(arch), Ok(os)) = (
            std::env::var("CARGO_CFG_TARGET_ARCH"),
            std::env::var("CARGO_CFG_TARGET_OS"),
        ) {
            let vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_else(|_| "unknown".to_owned());
            let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
            if env.is_empty() {
                ts.push(format!("{arch}-{vendor}-{os}"));
            } else {
                ts.push(format!("{arch}-{vendor}-{os}-{env}"));
            }
        }
        // Also try the TARGET env var set by some build systems
        if let Ok(t) = std::env::var("TARGET") {
            if !ts.contains(&t) {
                ts.push(t);
            }
        }
        // Common Linux triples as fallback
        for triple in [
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
        ] {
            if !ts.contains(&triple.to_owned()) {
                ts.push(triple.to_owned());
            }
        }
        ts
    };

    for triple in &triples {
        for profile in &profiles {
            let candidate = target_root.join(triple).join(profile).join("ralph");
            searched.push(candidate.display().to_string());
            if candidate.exists() {
                return candidate;
            }
        }
    }

    panic!(
        "ralph binary not found for integration test.\n\
         Searched paths:\n  {}\n\n\
         Set RALPH_TEST_BIN or CARGO_BIN_EXE_ralph to the absolute path of the ralph binary.",
        searched.join("\n  ")
    );
}
