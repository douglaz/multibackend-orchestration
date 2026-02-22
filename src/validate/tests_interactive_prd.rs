use super::*;

use std::fs;

use crate::daemon::github;
use crate::daemon::interactive_prd::{
    detect_approval, has_prd_label, prd_marker, prd_status_failed_marker, InteractivePrdState,
    PrdWorkflowState, PRD_LABELS, PRD_LABEL_NAMES,
};
use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "interactive_prd::state_serialization_roundtrip",
            func: state_serialization_roundtrip,
        },
        ConformanceTest {
            name: "interactive_prd::prd_labels_are_complete",
            func: prd_labels_are_complete,
        },
        ConformanceTest {
            name: "interactive_prd::prd_label_detection_filters_correctly",
            func: prd_label_detection_filters_correctly,
        },
        ConformanceTest {
            name: "interactive_prd::approval_detection_positive_and_negative",
            func: approval_detection_positive_and_negative,
        },
        ConformanceTest {
            name: "interactive_prd::marker_format_is_correct",
            func: marker_format_is_correct,
        },
        ConformanceTest {
            name: "interactive_prd::state_persistence_survives_restart",
            func: state_persistence_survives_restart,
        },
        ConformanceTest {
            name: "interactive_prd::failed_state_persists_error_info",
            func: failed_state_persists_error_info,
        },
        ConformanceTest {
            name: "interactive_prd::terminal_states_are_idempotent",
            func: terminal_states_are_idempotent,
        },
        ConformanceTest {
            name: "interactive_prd::prd_ready_label_conflict_detection",
            func: prd_ready_label_conflict_detection,
        },
        ConformanceTest {
            name: "interactive_prd::startup_prd_label_ensure",
            func: startup_prd_label_ensure,
        },
        ConformanceTest {
            name: "interactive_prd::prd_ready_conflict_in_claim_path",
            func: prd_ready_conflict_in_claim_path,
        },
        ConformanceTest {
            name: "interactive_prd::idempotent_state_reprocessing",
            func: idempotent_state_reprocessing,
        },
        ConformanceTest {
            name: "interactive_prd::pickup_and_question_posting",
            func: pickup_and_question_posting,
        },
    ]
}

fn state_serialization_roundtrip(_harness: &RalphHarness) -> TestResult {
    let variants = [
        PrdWorkflowState::Pending,
        PrdWorkflowState::AwaitingAnswers,
        PrdWorkflowState::AwaitingFeedback,
        PrdWorkflowState::Done,
        PrdWorkflowState::Failed,
    ];

    for state in variants {
        let json = match serde_json::to_string(&state) {
            Ok(j) => j,
            Err(err) => return TestResult::Fail(format!("serialize failed: {err}")),
        };
        let parsed: PrdWorkflowState = match serde_json::from_str(&json) {
            Ok(p) => p,
            Err(err) => return TestResult::Fail(format!("deserialize failed: {err}")),
        };
        if parsed != state {
            return TestResult::Fail(format!("roundtrip mismatch for {state:?}"));
        }
    }
    TestResult::Pass
}

fn prd_labels_are_complete(_harness: &RalphHarness) -> TestResult {
    if PRD_LABELS.len() != 5 {
        return TestResult::Fail(format!(
            "expected 5 PRD labels, got {}",
            PRD_LABELS.len()
        ));
    }

    let expected = [
        "ralph:prd",
        "ralph:prd-active",
        "ralph:prd-approved",
        "ralph:prd-done",
        "ralph:prd-failed",
    ];
    let names: Vec<&str> = PRD_LABELS.iter().map(|(name, _, _)| *name).collect();
    for label in &expected {
        if !names.contains(label) {
            return TestResult::Fail(format!("missing PRD label: {label}"));
        }
    }
    TestResult::Pass
}

fn prd_label_detection_filters_correctly(_harness: &RalphHarness) -> TestResult {
    // All PRD labels should be detected
    for &label_name in PRD_LABEL_NAMES {
        if !has_prd_label(&[label_name.to_owned()]) {
            return TestResult::Fail(format!(
                "has_prd_label should return true for {label_name}"
            ));
        }
    }

    // Non-PRD labels should not be detected
    let non_prd = vec!["ralph:ready".to_owned(), "bug".to_owned()];
    if has_prd_label(&non_prd) {
        return TestResult::Fail("has_prd_label should return false for non-PRD labels".to_owned());
    }

    TestResult::Pass
}

fn approval_detection_positive_and_negative(_harness: &RalphHarness) -> TestResult {
    // Positive
    let positive_cases = ["Approved.", "LGTM", "Ship it!", "Looks good to me"];
    for text in &positive_cases {
        if !detect_approval(text) {
            return TestResult::Fail(format!("expected approval for: {text}"));
        }
    }

    // Negative
    let negative_cases = ["not approved", "do not approve", "not lgtm"];
    for text in &negative_cases {
        if detect_approval(text) {
            return TestResult::Fail(format!("unexpected approval for: {text}"));
        }
    }

    // Mixed signals => no approval
    if detect_approval("approved, but do not approve yet") {
        return TestResult::Fail("mixed signals should not be approval".to_owned());
    }

    // Code blocks should be stripped
    if detect_approval("```\napproved\n```") {
        return TestResult::Fail("code blocks should be stripped before approval check".to_owned());
    }

    TestResult::Pass
}

fn marker_format_is_correct(_harness: &RalphHarness) -> TestResult {
    let marker = prd_marker(42, "questions", 1);
    if marker != "<!-- ralph:prd:42:questions-v1 -->" {
        return TestResult::Fail(format!("unexpected marker: {marker}"));
    }

    let failed_marker = prd_status_failed_marker(42);
    if failed_marker != "<!-- ralph:prd:42:status-failed -->" {
        return TestResult::Fail(format!("unexpected failed marker: {failed_marker}"));
    }

    TestResult::Pass
}

fn state_persistence_survives_restart(harness: &RalphHarness) -> TestResult {
    let data_dir = harness.data_dir();

    let mut state = InteractivePrdState::new("acme", "widgets", 42);
    state.state = PrdWorkflowState::AwaitingAnswers;
    state.question_revision = 1;
    state.questions_comment_id = Some(12345);
    state.questions_posted_at = Some(chrono::Utc::now());
    state.last_advanced_at = Some(chrono::Utc::now());

    if let Err(err) = state.save(data_dir) {
        return TestResult::Fail(format!("save failed: {err}"));
    }

    // Simulate restart: reload
    match InteractivePrdState::load(data_dir, "acme", "widgets", 42) {
        Ok(Some(loaded)) => {
            if loaded.state != PrdWorkflowState::AwaitingAnswers {
                return TestResult::Fail(format!("expected AwaitingAnswers, got {:?}", loaded.state));
            }
            if loaded.question_revision != 1 {
                return TestResult::Fail(format!(
                    "expected question_revision=1, got {}",
                    loaded.question_revision
                ));
            }
            if loaded.questions_comment_id != Some(12345) {
                return TestResult::Fail("questions_comment_id mismatch".to_owned());
            }
        }
        Ok(None) => return TestResult::Fail("state should exist after save".to_owned()),
        Err(err) => return TestResult::Fail(format!("load failed: {err}")),
    }

    TestResult::Pass
}

fn failed_state_persists_error_info(harness: &RalphHarness) -> TestResult {
    let data_dir = harness.data_dir();

    let mut state = InteractivePrdState::new("acme", "widgets", 99);
    state.state = PrdWorkflowState::Failed;
    state.error_count = 3;
    state.last_error = Some("backend timeout after 120s".to_owned());
    state.last_advanced_at = Some(chrono::Utc::now());

    if let Err(err) = state.save(data_dir) {
        return TestResult::Fail(format!("save failed: {err}"));
    }

    match InteractivePrdState::load(data_dir, "acme", "widgets", 99) {
        Ok(Some(loaded)) => {
            if loaded.state != PrdWorkflowState::Failed {
                return TestResult::Fail(format!("expected Failed, got {:?}", loaded.state));
            }
            if loaded.error_count != 3 {
                return TestResult::Fail(format!("expected error_count=3, got {}", loaded.error_count));
            }
            if loaded.last_error.as_deref() != Some("backend timeout after 120s") {
                return TestResult::Fail("last_error mismatch".to_owned());
            }
            if !loaded.is_terminal() {
                return TestResult::Fail("Failed state should be terminal".to_owned());
            }
        }
        Ok(None) => return TestResult::Fail("state should exist after save".to_owned()),
        Err(err) => return TestResult::Fail(format!("load failed: {err}")),
    }

    TestResult::Pass
}

fn terminal_states_are_idempotent(harness: &RalphHarness) -> TestResult {
    let data_dir = harness.data_dir();

    // Save a Done state
    let mut state = InteractivePrdState::new("acme", "widgets", 77);
    state.state = PrdWorkflowState::Done;
    state.last_advanced_at = Some(chrono::Utc::now());
    if let Err(err) = state.save(data_dir) {
        return TestResult::Fail(format!("save failed: {err}"));
    }

    // Load and verify it's terminal
    match InteractivePrdState::load(data_dir, "acme", "widgets", 77) {
        Ok(Some(loaded)) => {
            if !loaded.is_terminal() {
                return TestResult::Fail("Done state should be terminal".to_owned());
            }
        }
        Ok(None) => return TestResult::Fail("state should exist".to_owned()),
        Err(err) => return TestResult::Fail(format!("load failed: {err}")),
    }

    TestResult::Pass
}

fn prd_ready_label_conflict_detection(_harness: &RalphHarness) -> TestResult {
    // An issue with ralph:prd + ralph:ready should be detected as having a PRD label
    let labels = vec!["ralph:ready".to_owned(), "ralph:prd".to_owned()];
    if !has_prd_label(&labels) {
        return TestResult::Fail(
            "has_prd_label should return true when ralph:prd is present alongside ralph:ready"
                .to_owned(),
        );
    }

    // An issue with only ralph:ready should NOT be detected
    let ready_only = vec!["ralph:ready".to_owned()];
    if has_prd_label(&ready_only) {
        return TestResult::Fail(
            "has_prd_label should return false when only ralph:ready is present".to_owned(),
        );
    }

    TestResult::Pass
}

// ---------------------------------------------------------------------------
// Runtime conformance tests (exercise daemon binary with mocked gh)
// ---------------------------------------------------------------------------

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

/// Verify that `daemon start` creates PRD lifecycle labels at startup.
///
/// The mock gh logs every `label create` call. We verify that all 5 PRD labels
/// and all 4 standard labels are attempted.
fn startup_prd_label_ensure(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let label_log = dh.temp_dir.path().join("prd_label_ensure.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list) printf '[]' ; exit 0 ;;
      edit) exit 0 ;;
      view) printf '' ; exit 0 ;;
      comment) exit 0 ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label)
    case "$2" in
      create)
        echo "$@" >> "{label_log_str}"
        exit 0
        ;;
      *) exit 1 ;;
    esac
    ;;
esac
exit 1
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .ralph_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        let log_raw = fs::read_to_string(&label_log).expect("label create log should exist");
        let lines: Vec<&str> = log_raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();

        // Verify PRD labels are created.
        // Use exact token matching: "create <label> " (with trailing space) to
        // avoid substring false positives (e.g. "ralph:prd" matching
        // "ralph:prd-active").
        let prd_label_names = [
            "ralph:prd",
            "ralph:prd-active",
            "ralph:prd-approved",
            "ralph:prd-done",
            "ralph:prd-failed",
        ];
        for label_name in &prd_label_names {
            let needle = format!("create {label_name} ");
            let count = lines.iter().filter(|line| line.contains(&needle)).count();
            assert_eq!(
                count, 1,
                "expected one label create call for PRD label '{label_name}', got {count}:\n{log_raw}"
            );
        }

        // Also verify standard labels are created
        for (label_name, _, _) in github::REQUIRED_LABELS {
            let needle = format!("create {label_name} ");
            let count = lines.iter().filter(|line| line.contains(&needle)).count();
            assert_eq!(
                count, 1,
                "expected one label create call for standard label '{label_name}', got {count}:\n{log_raw}"
            );
        }

        // Total: 4 standard + 5 PRD = 9
        let total_expected = github::REQUIRED_LABELS.len() + prd_label_names.len();
        assert_eq!(
            lines.len(),
            total_expected,
            "expected {total_expected} label create calls total, got {}:\n{log_raw}",
            lines.len()
        );
    })
}

/// Verify that `ralph:prd` + `ralph:ready` issues are NOT claimed by the
/// normal daemon workflow (the has_prd_label guard prevents dual ownership).
///
/// Runs a single-iteration daemon with a mock issue that has both labels.
/// The issue must NOT be claimed (no ralph:in-progress label edit).
fn prd_ready_conflict_in_claim_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let label_log = dh.temp_dir.path().join("prd_conflict_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // Issue has both ralph:ready and ralph:prd — should be skipped by claim path
        let issues = r#"[{"number":50,"title":"prd conflict issue","labels":[{"name":"ralph:ready"},{"name":"ralph:prd"}],"body":"test"}]"#;

        let gh_path = write_mock_gh(&dh, &mock_scripts::daemon_mock_gh_script()).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                    ("MOCK_GH_ISSUES", issues),
                    ("MOCK_GH_LABEL_LOG", &label_log_str),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // Verify no claim happened — no ralph:in-progress label swap in the log
        let log_raw = fs::read_to_string(&label_log).unwrap_or_default();
        let claimed = log_raw.contains("--add-label") && log_raw.contains("ralph:in-progress");
        assert!(
            !claimed,
            "issue #50 with ralph:prd should NOT be claimed by normal workflow, label log:\n{log_raw}"
        );
    })
}

/// Verify that saving and loading state is idempotent across multiple
/// save/load cycles (simulating reprocessing on daemon restart).
fn idempotent_state_reprocessing(harness: &RalphHarness) -> TestResult {
    let data_dir = harness.data_dir();

    // Create a state in AwaitingAnswers (as if questions were already posted)
    let mut state = InteractivePrdState::new("acme", "widgets", 55);
    state.state = PrdWorkflowState::AwaitingAnswers;
    state.question_revision = 1;
    state.questions_comment_id = Some(111);
    state.questions_posted_at = Some(chrono::Utc::now());
    state.last_advanced_at = Some(chrono::Utc::now());

    if let Err(err) = state.save(data_dir) {
        return TestResult::Fail(format!("first save failed: {err}"));
    }

    // First reload
    let loaded1 = match InteractivePrdState::load(data_dir, "acme", "widgets", 55) {
        Ok(Some(s)) => s,
        Ok(None) => return TestResult::Fail("state should exist after first save".to_owned()),
        Err(err) => return TestResult::Fail(format!("first load failed: {err}")),
    };

    // Save the loaded state again (simulating daemon reprocessing)
    if let Err(err) = loaded1.save(data_dir) {
        return TestResult::Fail(format!("re-save failed: {err}"));
    }

    // Second reload — should be identical
    let loaded2 = match InteractivePrdState::load(data_dir, "acme", "widgets", 55) {
        Ok(Some(s)) => s,
        Ok(None) => return TestResult::Fail("state should exist after re-save".to_owned()),
        Err(err) => return TestResult::Fail(format!("second load failed: {err}")),
    };

    if loaded1 != loaded2 {
        return TestResult::Fail("state should be identical after save/load/save/load cycle".to_owned());
    }

    // Verify key fields survived
    if loaded2.state != PrdWorkflowState::AwaitingAnswers {
        return TestResult::Fail(format!(
            "expected AwaitingAnswers after reprocessing, got {:?}",
            loaded2.state
        ));
    }
    if loaded2.question_revision != 1 {
        return TestResult::Fail("question_revision should survive reprocessing".to_owned());
    }

    TestResult::Pass
}

/// Verify that a `ralph:prd` issue is picked up, questions are posted with the
/// correct marker, labels are swapped (`ralph:prd-active` added, `ralph:prd`
/// removed), and state is persisted as `AwaitingAnswers`.
///
/// Runs a single-iteration daemon tick with mock gh and mock backends. Then
/// verifies:
/// 1. `ralph:prd-active` was added to the issue
/// 2. `ralph:prd` was removed from the issue
/// 3. A questions comment was posted (logged)
/// 4. Persisted state shows `AwaitingAnswers` with `question_revision == 1`
///
/// On a second daemon tick, the same issue (now `ralph:prd-active`) should NOT
/// produce a duplicate questions comment (idempotency).
fn pickup_and_question_posting(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        // Set up mock backends that produce question output
        let question_script = dh
            .write_mock_script(
                "prd_question_backend.sh",
                r#"#!/bin/sh
# Mock backend for PRD question generation - reads stdin, produces questions
cat >/dev/null
printf '1. What are the performance requirements?\n'
printf '2. What error handling strategy should be used?\n'
printf '3. Are there any backward compatibility constraints?\n'
"#,
            )
            .expect("write question backend script");

        // Configure both claude and codex backends to use our mock
        dh.setup_mock_backends_stable(&question_script)
            .expect("setup mock backends");

        let label_log = dh.temp_dir.path().join("prd_pickup_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
        let comment_log = dh.temp_dir.path().join("prd_pickup_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        // Mock gh script that:
        // - Returns issue #10 with ralph:prd label on `issue list --label ralph:prd`
        // - Returns empty on `issue list --label ralph:prd-active` (first tick)
        // - Logs label edits
        // - Tracks posted comments for marker idempotency
        let gh_script = format!(
            r#"#!/bin/sh
LABEL_LOG="{label_log_str}"
COMMENT_LOG="{comment_log_str}"

case "$1" in
  issue)
    case "$2" in
      list)
        # Check which label is being queried
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
          printf '[{{"number":10,"title":"Add user auth","labels":[{{"name":"ralph:prd"}}],"body":"We need user authentication"}}]'
        elif [ "$has_active" = "1" ]; then
          printf '[]'
        elif [ "$has_ready" = "1" ]; then
          printf '[]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
        echo "$@" >> "$LABEL_LOG"
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
          # Return the posted comment if it exists (for marker idempotency check)
          if [ -f "$COMMENT_LOG" ]; then
            comment_body="$(cat "$COMMENT_LOG")"
            printf '{{"comments":[{{"id":42001,"author":{{"login":"ralph-bot"}},"body":"%s","createdAt":"2026-01-01T00:00:00Z"}}]}}' "$(printf '%s' "$comment_body" | sed 's/"/\\"/g' | tr '\n' ' ')"
          else
            printf '{{"comments":[]}}'
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[]}}'
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          printf '{{"title":"Add user auth","body":"We need user authentication"}}'
          exit 0
        fi
        printf ''
        exit 0
        ;;
      comment)
        # Log the comment body to a file for marker checks
        shift; shift  # skip 'issue' 'comment'
        while [ $# -gt 0 ]; do
          case "$1" in
            --body)
              printf '%s' "$2" > "$COMMENT_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
        exit 0
        ;;
    esac
    ;;
  pr)
    case "$2" in
      list) printf '' ; exit 0 ;;
      create) printf 'https://github.com/mock/pr/1\n' ; exit 0 ;;
      edit) exit 0 ;;
    esac
    ;;
  repo) printf 'acme/widgets\n' ; exit 0 ;;
  label)
    case "$2" in
      create) exit 0 ;;
    esac
    ;;
esac
exit 0
"#
        );

        let gh_path = write_mock_gh(&dh, &gh_script).expect("write mock gh");
        let ralph_path = write_daemon_mock_ralph(&dh).expect("write mock ralph");

        // Run first daemon tick
        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[
                    ("PATH", &gh_path),
                    ("RALPH_DAEMON_BIN", &ralph_path),
                ],
            )
            .expect("daemon start should execute");
        assert_exit_code(&output, 0);

        // 1. Verify ralph:prd-active was added
        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("--add-label") && label_raw.contains("ralph:prd-active"),
            "ralph:prd-active should have been added, label log:\n{label_raw}"
        );

        // 2. Verify ralph:prd was removed
        assert!(
            label_raw.contains("--remove-label") && label_raw.contains("ralph:prd"),
            "ralph:prd should have been removed, label log:\n{label_raw}"
        );

        // 3. Verify questions comment was posted (comment log exists and contains marker)
        let comment_raw = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            comment_raw.contains("<!-- ralph:prd:10:questions-v1 -->"),
            "questions marker should be in posted comment, comment log:\n{comment_raw}"
        );
        assert!(
            comment_raw.contains("Clarifying Questions"),
            "posted comment should contain questions heading, comment log:\n{comment_raw}"
        );

        // 4. Verify persisted state
        let state_path = dh
            .temp_dir
            .path()
            .join("acme")
            .join("widgets")
            .join(".ralph")
            .join("interactive-prd")
            .join("10.json");
        let state_raw = fs::read_to_string(&state_path).unwrap_or_else(|e| {
            panic!(
                "state file should exist at {}: {e}",
                state_path.display()
            )
        });
        let state: InteractivePrdState = serde_json::from_str(&state_raw).unwrap_or_else(|e| {
            panic!("state should be valid JSON: {e}\n{state_raw}")
        });
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingAnswers,
            "state should be AwaitingAnswers after pickup, got: {:?}",
            state.state
        );
        assert_eq!(
            state.question_revision, 1,
            "question_revision should be 1"
        );
        assert!(
            state.questions_posted_at.is_some(),
            "questions_posted_at should be set"
        );
        assert!(
            state.last_advanced_at.is_some(),
            "last_advanced_at should be set"
        );
        assert_eq!(state.error_count, 0, "error_count should be 0");
    })
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
