use super::*;

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;
use serde_json::Value;

use crate::daemon::interactive_prd::{
    InteractivePrdState, PrdDebugLogEntry, PrdDebugLogger, PrdWorkflowState, ValidationResult,
};
use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts;

static ENV_MUTEX: Mutex<()> = Mutex::new(());

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "interactive_prd_logging::log_file_creation_and_schema",
            func: log_file_creation_and_schema,
        },
        ConformanceTest {
            name: "interactive_prd_logging::collision_handling_same_second_same_label",
            func: collision_handling_same_second_same_label,
        },
        ConformanceTest {
            name: "interactive_prd_logging::prompt_truncation_metadata",
            func: prompt_truncation_metadata,
        },
        ConformanceTest {
            name: "interactive_prd_logging::review_retry_callback_captures_malformed_attempts",
            func: review_retry_callback_captures_malformed_attempts,
        },
        ConformanceTest {
            name: "interactive_prd_logging::review_retry_per_attempt_timing_guarantee",
            func: review_retry_per_attempt_timing_guarantee,
        },
        ConformanceTest {
            name: "interactive_prd_logging::question_gen_emits_expected_labels",
            func: question_gen_emits_expected_labels,
        },
        ConformanceTest {
            name: "interactive_prd_logging::draft_and_review_emit_expected_labels",
            func: draft_and_review_emit_expected_labels,
        },
        ConformanceTest {
            name: "interactive_prd_logging::state_file_path_unchanged",
            func: state_file_path_unchanged,
        },
    ]
}

fn log_file_creation_and_schema(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let logger = PrdDebugLogger::new(h.data_dir(), "acme", "widgets", 11);
        logger.log_attempt(
            "claude(opus)",
            "question-gen-a",
            "Prompt text",
            Some("1. A".to_owned()),
            None,
            ValidationResult::NotChecked,
        );

        let logs_dir = h
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/11/logs");
        let entries = load_logs(&logs_dir);
        assert_eq!(entries.len(), 1, "expected one log file");

        let entry = &entries[0];
        assert_eq!(entry["backend_spec"], Value::String("claude(opus)".to_owned()));
        assert_eq!(entry["label"], Value::String("question-gen-a".to_owned()));
        assert_eq!(entry["validation"]["status"], Value::String("not_checked".to_owned()));
        assert!(entry["timestamp"].as_str().is_some(), "missing timestamp");
        assert!(entry["prompt_chars"].as_u64().is_some(), "missing prompt_chars");
        assert!(entry["prompt"].as_str().is_some(), "missing prompt");
    })
}

fn collision_handling_same_second_same_label(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let logger = PrdDebugLogger::new(h.data_dir(), "acme", "widgets", 12);
        let entry = PrdDebugLogEntry {
            timestamp: "2026-02-26T12:00:00Z".to_owned(),
            backend_spec: "claude(opus)".to_owned(),
            label: "question-gen-a".to_owned(),
            prompt_chars: 3,
            prompt: "abc".to_owned(),
            raw_output: Some("ok".to_owned()),
            error: None,
            validation: ValidationResult::NotChecked,
        };

        let first = logger
            .write_entry("20260226T120000Z", "question-gen-a", &entry)
            .expect("first create_new write should succeed");
        let second = logger
            .write_entry("20260226T120000Z", "question-gen-a", &entry)
            .expect("collision write should suffix filename");

        assert_eq!(
            first.file_name().and_then(|n| n.to_str()),
            Some("20260226T120000Z-question-gen-a.json")
        );
        assert_eq!(
            second.file_name().and_then(|n| n.to_str()),
            Some("20260226T120000Z-001-question-gen-a.json")
        );
    })
}

fn prompt_truncation_metadata(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let _guard = ENV_MUTEX.lock().expect("env lock poisoned");
        let previous = std::env::var_os("RALPH_PRD_LOG_TRUNCATE");
        struct EnvRestore(Option<std::ffi::OsString>);
        impl Drop for EnvRestore {
            fn drop(&mut self) {
                if let Some(value) = &self.0 {
                    std::env::set_var("RALPH_PRD_LOG_TRUNCATE", value);
                } else {
                    std::env::remove_var("RALPH_PRD_LOG_TRUNCATE");
                }
            }
        }
        let _restore = EnvRestore(previous);
        std::env::set_var("RALPH_PRD_LOG_TRUNCATE", "7");

        let logger = PrdDebugLogger::new(h.data_dir(), "acme", "widgets", 13);
        let prompt = "hello🙂world";
        logger.log_attempt(
            "claude(opus)",
            "question-gen-b",
            prompt,
            Some("ok".to_owned()),
            None,
            ValidationResult::NotChecked,
        );

        let logs_dir = h
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/13/logs");
        let entries = load_logs(&logs_dir);
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(
            entry["prompt_chars"],
            Value::Number(serde_json::Number::from(prompt.chars().count() as u64))
        );
        let prompt_logged = entry["prompt"].as_str().unwrap_or_default();
        assert!(prompt_logged.contains("[truncated at"));
        assert!(prompt_logged.contains("full length:"));
    })
}

fn review_retry_callback_captures_malformed_attempts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script(
                "malformed-review-backend.sh",
                r#"#!/bin/sh
INPUT="$(cat)"
if echo "$INPUT" | grep -q 'Review the spec for\|\*\*Engineering Spec:\*\*\|review response could not be parsed'; then
  printf 'not-json\n'
else
  cat <<'EOF_SPEC'
## Summary
S
## Acceptance Criteria
A
## Technical Approach
T
## Files & Modules
F
## Testing Strategy
TS
## Out of Scope
O
EOF_SPEC
fi
"#,
            )
            .expect("write backend script");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup backends");

        let mut state = InteractivePrdState::new("acme", "widgets", 14);
        state.state = PrdWorkflowState::AwaitingAnswers;
        state.question_revision = 1;
        state.questions_posted_at = Some(
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
                .expect("timestamp")
                .with_timezone(&Utc),
        );
        state.questions_comment_id = Some(100);
        state.save(dh.data_dir()).expect("save state");

        let gh_script = r#"#!/bin/sh
set -eu

case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do
          if [ "$arg" = "ralph:prd-active" ]; then
            has_active=1
          fi
        done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":14,"title":"Draft issue","labels":[{"name":"ralph:prd-active"}],"body":"Body"}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        want_tb=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
            title,body) want_tb=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          printf '{"comments":[{"id":100,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:14:questions-v1 -->\\n## Clarifying Questions\\n1. Q?","createdAt":"2026-01-01T00:00:02Z"},{"id":101,"author":{"login":"octocat"},"body":"Answers here","createdAt":"2026-01-01T00:00:10Z"}]}'
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{"labels":[{"name":"ralph:prd-active"}]}'
          exit 0
        fi
        if [ "$want_tb" = "1" ]; then
          printf '{"title":"Draft issue","body":"Body"}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      edit)
        exit 0
        ;;
      comment)
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
  label)
    if [ "$2" = "create" ]; then
      exit 0
    fi
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
    esac
    ;;
esac

exit 1
"#;

        let gh_path = write_mock_gh(&dh, gh_script).expect("write gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                ["daemon", "start", "--repo", "acme/widgets", "--single-iteration"],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let logs_dir = dh
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/14/logs");
        let entries = load_logs(&logs_dir);
        let review_entries: Vec<&Value> = entries
            .iter()
            .filter(|entry| {
                entry["label"]
                    .as_str()
                    .map(|label| label.starts_with("draft-review-attempt-"))
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(
            review_entries.len(),
            3,
            "expected three review-attempt logs from production retry path"
        );

        for attempt in 1..=3 {
            let label = format!("draft-review-attempt-{attempt}-of-3");
            assert!(
                review_entries
                    .iter()
                    .any(|entry| entry["label"].as_str() == Some(label.as_str())),
                "missing {label} log entry"
            );
        }

        for entry in review_entries {
            assert!(
                entry["raw_output"].as_str().is_some(),
                "raw_output should be captured for malformed review output"
            );
            assert!(entry["error"].is_null(), "error should be null for parse failures");
            assert_eq!(
                entry["validation"]["status"],
                Value::String("review_parse_failed".to_owned()),
                "validation.status should be review_parse_failed"
            );
        }
    })
}

fn review_retry_per_attempt_timing_guarantee(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let logs_dir = dh
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/24/logs");
        let backend_body = format!(
            r#"#!/bin/sh
set -eu

INPUT="$(cat)"
if echo "$INPUT" | grep -q 'Review the spec for\|\*\*Engineering Spec:\*\*\|review response could not be parsed'; then
  COUNTER_FILE="$(dirname "$0")/review-attempt-counter"
  ATTEMPT=0
  if [ -f "$COUNTER_FILE" ]; then
    ATTEMPT="$(cat "$COUNTER_FILE")"
  fi
  ATTEMPT=$((ATTEMPT + 1))
  printf '%s' "$ATTEMPT" > "$COUNTER_FILE"

  if [ "$ATTEMPT" -ge 2 ]; then
    PREV=$((ATTEMPT - 1))
    if ! ls "{logs_dir}"/*-draft-review-attempt-"$PREV"-of-3.json >/dev/null 2>&1; then
      echo "missing prior attempt log for review attempt $PREV" >&2
      exit 41
    fi
  fi

  if [ "$ATTEMPT" -eq 1 ]; then
    printf 'not-json\n'
  else
    printf '```json\n{{"approved": true, "issues": []}}\n```\n'
  fi
else
  cat <<'EOF_SPEC'
## Summary
S
## Acceptance Criteria
A
## Technical Approach
T
## Files & Modules
F
## Testing Strategy
TS
## Out of Scope
O
EOF_SPEC
fi
"#,
            logs_dir = logs_dir.to_string_lossy()
        );

        let backend_script = dh
            .write_mock_script("review-timing-backend.sh", &backend_body)
            .expect("write backend script");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup backends");

        let mut state = InteractivePrdState::new("acme", "widgets", 24);
        state.state = PrdWorkflowState::AwaitingAnswers;
        state.question_revision = 1;
        state.questions_posted_at = Some(
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
                .expect("timestamp")
                .with_timezone(&Utc),
        );
        state.questions_comment_id = Some(100);
        state.save(dh.data_dir()).expect("save state");

        let gh_script = r#"#!/bin/sh
set -eu

case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do
          if [ "$arg" = "ralph:prd-active" ]; then
            has_active=1
          fi
        done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":24,"title":"Draft issue","labels":[{"name":"ralph:prd-active"}],"body":"Body"}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        want_tb=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
            title,body) want_tb=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          printf '{"comments":[{"id":100,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:24:questions-v1 -->\\n## Clarifying Questions\\n1. Q?","createdAt":"2026-01-01T00:00:02Z"},{"id":101,"author":{"login":"octocat"},"body":"Answers here","createdAt":"2026-01-01T00:00:10Z"}]}'
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{"labels":[{"name":"ralph:prd-active"}]}'
          exit 0
        fi
        if [ "$want_tb" = "1" ]; then
          printf '{"title":"Draft issue","body":"Body"}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      edit)
        exit 0
        ;;
      comment)
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
  label)
    if [ "$2" = "create" ]; then
      exit 0
    fi
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
    esac
    ;;
esac

exit 1
"#;

        let gh_path = write_mock_gh(&dh, gh_script).expect("write gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                ["daemon", "start", "--repo", "acme/widgets", "--single-iteration"],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let entries = load_logs(&logs_dir);
        let review_entries: Vec<&Value> = entries
            .iter()
            .filter(|entry| {
                entry["label"]
                    .as_str()
                    .map(|label| label.starts_with("draft-review-attempt-"))
                    .unwrap_or(false)
            })
            .collect();

        assert_eq!(
            review_entries.len(),
            2,
            "expected exactly two review-attempt logs (first malformed, second approved)"
        );
        assert!(
            review_entries
                .iter()
                .any(|entry| entry["label"].as_str() == Some("draft-review-attempt-1-of-3")),
            "missing attempt-1 review log"
        );
        assert!(
            review_entries
                .iter()
                .any(|entry| entry["label"].as_str() == Some("draft-review-attempt-2-of-3")),
            "missing attempt-2 review log"
        );
    })
}

fn question_gen_emits_expected_labels(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script(
                "prd-backend.sh",
                r#"#!/bin/sh
cat >/dev/null
printf '1. Question one?\n2. Question two?\n3. Question three?\n'
"#,
            )
            .expect("write backend script");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup backends");

        let gh_path = write_mock_gh(&dh, &mock_scripts::daemon_mock_gh_script()).expect("write gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");
        let issues = r#"[{"number":10,"title":"Need PRD","labels":[{"name":"ralph:prd"}],"body":"Body"}]"#;

        let output = dh
            .daemon_env(
                ["daemon", "start", "--repo", "acme/widgets", "--single-iteration"],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let logs_dir = dh
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/10/logs");
        let entries = load_logs(&logs_dir);
        let labels: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry["label"].as_str().map(str::to_owned))
            .collect();

        assert!(labels.contains(&"question-gen-a".to_owned()));
        assert!(labels.contains(&"question-gen-b".to_owned()));
        assert!(labels.contains(&"synthesis".to_owned()));
    })
}

fn draft_and_review_emit_expected_labels(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script(
                "draft-backend.sh",
                r#"#!/bin/sh
INPUT="$(cat)"
if echo "$INPUT" | grep -q 'Review the spec for\|\*\*Engineering Spec:\*\*\|review response could not be parsed'; then
  printf '```json\n{"approved": true, "issues": []}\n```\n'
else
  cat <<'EOF_SPEC'
## Summary
S
## Acceptance Criteria
A
## Technical Approach
T
## Files & Modules
F
## Testing Strategy
TS
## Out of Scope
O
EOF_SPEC
fi
"#,
            )
            .expect("write backend script");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup backends");

        let mut state = InteractivePrdState::new("acme", "widgets", 22);
        state.state = PrdWorkflowState::AwaitingAnswers;
        state.question_revision = 1;
        state.questions_posted_at = Some(
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
                .expect("timestamp")
                .with_timezone(&Utc),
        );
        state.questions_comment_id = Some(100);
        state.save(dh.data_dir()).expect("save state");

        let gh_script = r#"#!/bin/sh
set -eu

case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do
          if [ "$arg" = "ralph:prd-active" ]; then
            has_active=1
          fi
        done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":22,"title":"Draft issue","labels":[{"name":"ralph:prd-active"}],"body":"Body"}]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_comments=0
        want_labels=0
        want_tb=0
        for arg in "$@"; do
          case "$arg" in
            comments) want_comments=1 ;;
            labels) want_labels=1 ;;
            title,body) want_tb=1 ;;
          esac
        done
        if [ "$want_comments" = "1" ]; then
          printf '{"comments":[{"id":100,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:22:questions-v1 -->\\n## Clarifying Questions\\n1. Q?","createdAt":"2026-01-01T00:00:02Z"},{"id":101,"author":{"login":"octocat"},"body":"Answers here","createdAt":"2026-01-01T00:00:10Z"}]}'
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{"labels":[{"name":"ralph:prd-active"}]}'
          exit 0
        fi
        if [ "$want_tb" = "1" ]; then
          printf '{"title":"Draft issue","body":"Body"}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      edit)
        exit 0
        ;;
      comment)
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
  label)
    if [ "$2" = "create" ]; then
      exit 0
    fi
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/repo/pull/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo)
    case "$2" in
      clone)
        target_dir="$4"
        mkdir -p "$target_dir"
        git init "$target_dir" --quiet 2>/dev/null
        git -C "$target_dir" config user.email "mock@test"
        git -C "$target_dir" config user.name "MockClone"
        touch "$target_dir/.gitkeep"
        git -C "$target_dir" add .gitkeep
        git -C "$target_dir" commit -m "initial" --quiet 2>/dev/null
        exit 0
        ;;
      view)
        printf 'acme/widgets\n'
        exit 0
        ;;
    esac
    ;;
esac

exit 1
"#;

        let gh_path = write_mock_gh(&dh, gh_script).expect("write gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                ["daemon", "start", "--repo", "acme/widgets", "--single-iteration"],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let logs_dir = dh
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/22/logs");
        let entries = load_logs(&logs_dir);
        let labels: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry["label"].as_str().map(str::to_owned))
            .collect();

        assert!(labels.contains(&"draft-attempt-1".to_owned()));
        assert!(labels.contains(&"draft-review-attempt-1-of-3".to_owned()));
    })
}

fn state_file_path_unchanged(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let mut state = InteractivePrdState::new("acme", "widgets", 99);
        state.state = PrdWorkflowState::AwaitingAnswers;
        state.question_revision = 1;
        state.questions_posted_at = Some(Utc::now());
        state.save(h.data_dir()).expect("save state");

        let logger = PrdDebugLogger::new(h.data_dir(), "acme", "widgets", 99);
        logger.log_attempt(
            "claude(opus)",
            "question-gen-a",
            "prompt",
            Some("output".to_owned()),
            None,
            ValidationResult::NotChecked,
        );

        let state_path = h
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/99.json");
        let old_style_state_path = h
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/99/state.json");
        let logs_dir = h
            .data_dir()
            .join("acme/widgets/.ralph/interactive-prd/99/logs");

        assert!(state_path.exists(), "state file path regressed: {}", state_path.display());
        assert!(
            !old_style_state_path.exists(),
            "unexpected alternate state path exists: {}",
            old_style_state_path.display()
        );
        assert!(logs_dir.exists(), "logs dir should exist: {}", logs_dir.display());
    })
}

fn write_mock_gh(h: &RalphHarness, body: &str) -> crate::Result<String> {
    let script = h.write_mock_script("gh", body)?;
    let base = script
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let existing = std::env::var("PATH").unwrap_or_default();
    Ok(format!("{base}:{existing}"))
}

fn write_daemon_mock_ralph(h: &RalphHarness) -> crate::Result<String> {
    let script = h.write_mock_script("mock_ralph", &mock_scripts::daemon_mock_ralph_script())?;
    Ok(script.to_string_lossy().into_owned())
}

fn load_logs(logs_dir: &Path) -> Vec<Value> {
    let mut files: Vec<_> = fs::read_dir(logs_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", logs_dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    files.sort();

    files
        .into_iter()
        .map(|path| {
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            serde_json::from_str::<Value>(&raw)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
        })
        .collect()
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(super::panic_message(e)),
    }
}
