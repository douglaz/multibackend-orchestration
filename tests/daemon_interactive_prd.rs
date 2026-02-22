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
