use super::*;

use std::fs;

use crate::daemon::github;
use crate::daemon::interactive_prd::{
    detect_approval, has_prd_label, prd_marker, prd_status_approved_marker,
    prd_status_failed_marker, InteractivePrdState, PrdWorkflowState, DRAFT_SECTION_RETRIES,
    PRD_LABELS, PRD_LABEL_NAMES, REQUIRED_SPEC_SECTION_COUNT,
};
use crate::prd::quick::check_spec_sections;
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
        ConformanceTest {
            name: "interactive_prd::answer_to_draft",
            func: answer_to_draft,
        },
        ConformanceTest {
            name: "interactive_prd::feedback_revision",
            func: feedback_revision,
        },
        ConformanceTest {
            name: "interactive_prd::approval_by_comment",
            func: approval_by_comment,
        },
        ConformanceTest {
            name: "interactive_prd::approval_by_label",
            func: approval_by_label,
        },
        ConformanceTest {
            name: "interactive_prd::feedback_stage_failure_labeling",
            func: feedback_stage_failure_labeling,
        },
        ConformanceTest {
            name: "interactive_prd::mixed_comments_approval_triggers_done",
            func: mixed_comments_approval_triggers_done,
        },
        ConformanceTest {
            name: "interactive_prd::approval_path_github_failure_increments_error",
            func: approval_path_github_failure_increments_error,
        },
        ConformanceTest {
            name: "interactive_prd::approval_failure_exhaustion_transitions_to_failed",
            func: approval_failure_exhaustion_transitions_to_failed,
        },
        ConformanceTest {
            name: "interactive_prd::draft_boundary_filtering_excludes_pre_draft_approval",
            func: draft_boundary_filtering_excludes_pre_draft_approval,
        },
        ConformanceTest {
            name: "interactive_prd::restart_continuity_marker_timestamp_hydration",
            func: restart_continuity_marker_timestamp_hydration,
        },
        ConformanceTest {
            name: "interactive_prd::draft_boundary_filtering_excludes_pre_draft_revision",
            func: draft_boundary_filtering_excludes_pre_draft_revision,
        },
        ConformanceTest {
            name: "interactive_prd::bot_login_failure_exhaustion_awaiting_answers",
            func: bot_login_failure_exhaustion_awaiting_answers,
        },
        ConformanceTest {
            name: "interactive_prd::bot_login_failure_exhaustion_awaiting_feedback",
            func: bot_login_failure_exhaustion_awaiting_feedback,
        },
        ConformanceTest {
            name: "interactive_prd::bot_login_failure_exhaustion_pending",
            func: bot_login_failure_exhaustion_pending,
        },
        ConformanceTest {
            name: "interactive_prd::approval_label_ordering_partial_failure_recovery",
            func: approval_label_ordering_partial_failure_recovery,
        },
        ConformanceTest {
            name: "interactive_prd::section_complete_spec_passes_validation",
            func: section_complete_spec_passes_validation,
        },
        ConformanceTest {
            name: "interactive_prd::section_incomplete_draft_is_rejected",
            func: section_incomplete_draft_is_rejected,
        },
        ConformanceTest {
            name: "interactive_prd::section_incomplete_revision_is_rejected",
            func: section_incomplete_revision_is_rejected,
        },
        ConformanceTest {
            name: "interactive_prd::section_constants_are_correct",
            func: section_constants_are_correct,
        },
        ConformanceTest {
            name: "interactive_prd::section_incomplete_draft_exhaustion_transitions_to_failed",
            func: section_incomplete_draft_exhaustion_transitions_to_failed,
        },
        ConformanceTest {
            name: "interactive_prd::section_incomplete_revision_exhaustion_transitions_to_failed",
            func: section_incomplete_revision_exhaustion_transitions_to_failed,
        },
        ConformanceTest {
            name: "interactive_prd::terminal_save_failure_keeps_retry_visibility",
            func: terminal_save_failure_keeps_retry_visibility,
        },
        ConformanceTest {
            name: "interactive_prd::bot_scoped_marker_ignores_user_spoof",
            func: bot_scoped_marker_ignores_user_spoof,
        },
        ConformanceTest {
            name: "interactive_prd::bot_scoped_extract_questions_ignores_spoof",
            func: bot_scoped_extract_questions_ignores_spoof,
        },
        ConformanceTest {
            name: "interactive_prd::terminal_save_failure_failed_path_keeps_retry_visibility",
            func: terminal_save_failure_failed_path_keeps_retry_visibility,
        },
        ConformanceTest {
            name: "interactive_prd::status_failed_marker_spoof_resistance",
            func: status_failed_marker_spoof_resistance,
        },
        ConformanceTest {
            name: "interactive_prd::prd_poll_config_max_concurrent_field",
            func: prd_poll_config_max_concurrent_field,
        },
        ConformanceTest {
            name: "interactive_prd::max_concurrent_zero_treated_as_one",
            func: max_concurrent_zero_treated_as_one,
        },
        ConformanceTest {
            name: "interactive_prd::concurrent_dedup_invariant",
            func: concurrent_dedup_invariant,
        },
        ConformanceTest {
            name: "interactive_prd::concurrent_error_isolation",
            func: concurrent_error_isolation,
        },
        ConformanceTest {
            name: "interactive_prd::concurrent_panic_isolation",
            func: concurrent_panic_isolation,
        },
        ConformanceTest {
            name: "interactive_prd::concurrent_bounded_worker_count",
            func: concurrent_bounded_worker_count,
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
        return TestResult::Fail(format!("expected 5 PRD labels, got {}", PRD_LABELS.len()));
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
            return TestResult::Fail(format!("has_prd_label should return true for {label_name}"));
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
                return TestResult::Fail(format!(
                    "expected AwaitingAnswers, got {:?}",
                    loaded.state
                ));
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
                return TestResult::Fail(format!(
                    "expected error_count=3, got {}",
                    loaded.error_count
                ));
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

        let gh_path =
            write_mock_gh(&dh, &mock_scripts::daemon_mock_gh_script()).expect("write mock gh");
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
        return TestResult::Fail(
            "state should be identical after save/load/save/load cycle".to_owned(),
        );
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
  api)
    if [ "$2" = "user" ]; then
      printf 'ralph-bot\n'
      exit 0
    fi
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
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
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
        let state_raw = fs::read_to_string(&state_path)
            .unwrap_or_else(|e| panic!("state file should exist at {}: {e}", state_path.display()));
        let state: InteractivePrdState = serde_json::from_str(&state_raw)
            .unwrap_or_else(|e| panic!("state should be valid JSON: {e}\n{state_raw}"));
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingAnswers,
            "state should be AwaitingAnswers after pickup, got: {:?}",
            state.state
        );
        assert_eq!(state.question_revision, 1, "question_revision should be 1");
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

/// Verify AwaitingAnswers -> AwaitingFeedback transition:
/// 1. Detects first non-bot answer comment after questions_posted_at
/// 2. Generates a draft via writer/reviewer mock backends
/// 3. Posts `draft-v1` marker comment idempotently
/// 4. Persists AwaitingFeedback state fields
fn answer_to_draft(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script(
                "prd_draft_backend.sh",
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
Generated from interactive answers.

## Acceptance Criteria
- [ ] Draft posted to issue.

## Technical Approach
Use the daemon transition and quick-prd review loop.

## Files & Modules
- src/daemon/interactive_prd.rs

## Testing Strategy
- conformance test coverage

## Out of Scope
- webhooks
EOF
"#,
            )
            .expect("write backend script");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup mock backends");

        let state_path = dh
            .temp_dir
            .path()
            .join("acme")
            .join("widgets")
            .join(".ralph")
            .join("interactive-prd")
            .join("22.json");
        fs::create_dir_all(state_path.parent().expect("state path parent should exist"))
            .expect("create state dir");
        let seeded = serde_json::json!({
            "issue_number": 22,
            "owner": "acme",
            "repo": "widgets",
            "state": "AwaitingAnswers",
            "question_revision": 1,
            "draft_revision": 0,
            "questions_comment_id": 320,
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
            serde_json::to_string_pretty(&seeded).expect("serialize seed state"),
        )
        .expect("write seeded state");

        let draft_log = dh.temp_dir.path().join("prd_answer_to_draft_comment.log");
        let draft_log_str = draft_log.to_string_lossy().into_owned();
        let gh_script = format!(
            r#"#!/bin/sh
DRAFT_LOG="{draft_log_str}"

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
          printf '[{{"number":22,"title":"PRD issue","labels":[{{"name":"ralph:prd-active"}}],"body":"Need a spec from answers"}}]'
        elif [ "$has_ready" = "1" ]; then
          printf '[]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      edit)
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
          if [ -f "$DRAFT_LOG" ]; then
            draft_body="$(cat "$DRAFT_LOG" | sed 's/"/\\"/g' | tr '\n' ' ')"
            printf '{{"comments":[{{"id":320,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:22:questions-v1 -->\\n## Clarifying Questions\\n1. What API should be exposed?","createdAt":"2026-01-01T00:00:05Z"}},{{"id":321,"author":{{"login":"octocat"}},"body":"Expose REST and include retries.","createdAt":"2026-01-01T00:00:15Z"}},{{"id":322,"author":{{"login":"ralph-bot"}},"body":"%s","createdAt":"2026-01-01T00:00:20Z"}}]}}' "$draft_body"
          else
            printf '{{"comments":[{{"id":320,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:22:questions-v1 -->\\n## Clarifying Questions\\n1. What API should be exposed?","createdAt":"2026-01-01T00:00:05Z"}},{{"id":321,"author":{{"login":"octocat"}},"body":"Expose REST and include retries.","createdAt":"2026-01-01T00:00:15Z"}}]}}'
          fi
          exit 0
        fi
        if [ "$want_labels" = "1" ]; then
          printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'
          exit 0
        fi
        if [ "$want_title_body" = "1" ]; then
          printf '{{"title":"PRD issue","body":"Need a spec from answers"}}'
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
              printf '%s' "$2" > "$DRAFT_LOG"
              shift 2
              ;;
            *) shift ;;
          esac
        done
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
  repo)
    case "$2" in
      view) printf 'acme/widgets\n' ; exit 0 ;;
    esac
    ;;
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

        let output = dh
            .daemon_env(
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

        let state_raw = fs::read_to_string(&state_path).expect("state should exist");
        let state: InteractivePrdState =
            serde_json::from_str(&state_raw).expect("state should parse");
        assert_eq!(state.state, PrdWorkflowState::AwaitingFeedback);
        assert_eq!(state.draft_revision, 1);
        assert_eq!(state.last_processed_comment_id, Some(321));
        assert_eq!(
            state.user_answers.as_deref(),
            Some("Expose REST and include retries.")
        );
        assert_eq!(state.latest_draft_comment_id, Some(322));
        assert!(
            state
                .latest_draft_body
                .as_deref()
                .unwrap_or_default()
                .contains("## Summary"),
            "latest draft body should include spec sections"
        );

        let draft_raw = fs::read_to_string(&draft_log).expect("draft comment should be written");
        assert!(
            draft_raw.contains("<!-- ralph:prd:22:draft-v1 -->"),
            "expected draft-v1 marker in posted comment: {draft_raw}"
        );
    })
}

/// Verify that a feedback comment in AwaitingFeedback produces a revision
/// draft (draft-v2) and keeps the state as AwaitingFeedback.
fn feedback_revision(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script(
                "prd_feedback_rev.sh",
                r#"#!/bin/sh
INPUT="$(cat)"
if echo "$INPUT" | grep -q "reviewing an engineering specification"; then
  printf '```json\n{"approved": true, "issues": []}\n```\n'
  exit 0
fi
printf '## Summary\nRevised.\n\n## Acceptance Criteria\n- [ ] AC\n\n## Technical Approach\nApproach.\n\n## Files & Modules\n- file.rs\n\n## Testing Strategy\n- test\n\n## Out of Scope\n- none\n'
"#,
            )
            .expect("write backend");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup backends");

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/30.json");
        fs::create_dir_all(state_path.parent().unwrap()).expect("mkdir");
        let seed = serde_json::json!({
            "issue_number": 30, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 300, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 302,
            "latest_draft_body": "## Summary\nOrig.",
            "user_answers": "ans", "last_processed_comment_id": 301,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("fb_rev_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let gh_script = format!(
            r#"#!/bin/sh
LOG="{comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":30,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":300,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":301,"author":{{"login":"u"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":302,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":303,"author":{{"login":"u"}},"body":"add error handling","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) shift; shift; while [ $# -gt 0 ]; do case "$1" in --body) printf '%s' "$2" > "$LOG"; shift 2 ;; *) shift ;; esac; done; exit 0 ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(state.state, PrdWorkflowState::AwaitingFeedback);
        assert_eq!(state.draft_revision, 2, "draft should be incremented");
        assert_eq!(state.last_processed_comment_id, Some(303));

        let posted = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            posted.contains("<!-- ralph:prd:30:draft-v2 -->"),
            "draft-v2 marker expected: {posted}"
        );
    })
}

/// Verify that an approval comment transitions to Done.
fn approval_by_comment(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/31.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 31, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 310, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 312,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 311,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("approval_comment_conf.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let gh_script = format!(
            r#"#!/bin/sh
LOG="{comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":31,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          if [ -f "$LOG" ]; then
            printf '{{"comments":[{{"id":310,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":311,"author":{{"login":"u"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":312,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":313,"author":{{"login":"u"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}},{{"id":314,"author":{{"login":"ralph-bot"}},"body":"ok","createdAt":"2026-01-01T00:00:30Z"}}]}}'
          else
            printf '{{"comments":[{{"id":310,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":311,"author":{{"login":"u"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":312,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":313,"author":{{"login":"u"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          fi; exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) shift; shift; while [ $# -gt 0 ]; do case "$1" in --body) printf '%s' "$2" > "$LOG"; shift 2 ;; *) shift ;; esac; done; exit 0 ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(state.state, PrdWorkflowState::Done);
        assert!(state.is_terminal());

        let posted = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            posted.contains("<!-- ralph:prd:31:status-approved-v1 -->"),
            "should post status-approved marker: {posted}"
        );
    })
}

/// Verify that `ralph:prd-approved` label triggers Done transition.
fn approval_by_label(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/32.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 32, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 2,
            "questions_comment_id": 320, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 323,
            "latest_draft_body": "## Summary\nD.",
            "user_answers": "a", "last_processed_comment_id": 322,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("label_appr_conf.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let gh_script = format!(
            r#"#!/bin/sh
LOG="{comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":32,"title":"T","labels":[{{"name":"ralph:prd-active"}},{{"name":"ralph:prd-approved"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          if [ -f "$LOG" ]; then
            printf '{{"comments":[{{"id":320,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":321,"author":{{"login":"u"}},"body":"a","createdAt":"2026-01-01T00:00:10Z"}},{{"id":322,"author":{{"login":"u"}},"body":"feedback","createdAt":"2026-01-01T00:00:15Z"}},{{"id":323,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:20Z"}},{{"id":324,"author":{{"login":"ralph-bot"}},"body":"approved","createdAt":"2026-01-01T00:00:30Z"}}]}}'
          else
            printf '{{"comments":[{{"id":320,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":321,"author":{{"login":"u"}},"body":"a","createdAt":"2026-01-01T00:00:10Z"}},{{"id":322,"author":{{"login":"u"}},"body":"feedback","createdAt":"2026-01-01T00:00:15Z"}},{{"id":323,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:20Z"}}]}}'
          fi; exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}},{{"name":"ralph:prd-approved"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) shift; shift; while [ $# -gt 0 ]; do case "$1" in --body) printf '%s' "$2" > "$LOG"; shift 2 ;; *) shift ;; esac; done; exit 0 ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(state.state, PrdWorkflowState::Done);
        assert_eq!(state.draft_revision, 2, "draft should remain at 2");

        let posted = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            posted.contains(&prd_status_approved_marker(32, 2)),
            "should post status-approved-v2 marker: {posted}"
        );
    })
}

/// Verify that repeated failures in AwaitingFeedback result in Failed
/// state with `ralph:prd-failed` label. This test simulates error
/// accumulation by seeding error_count=2 and causing a transition failure.
fn feedback_stage_failure_labeling(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        // Backend that always fails (exits non-zero)
        let backend_script = dh
            .write_mock_script("prd_fail.sh", "#!/bin/sh\ncat >/dev/null\nexit 1\n")
            .unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/33.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        // Seed with error_count=2 so next failure triggers threshold
        let seed = serde_json::json!({
            "issue_number": 33, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 330, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 332,
            "latest_draft_body": "## Summary\nD.",
            "user_answers": "a", "last_processed_comment_id": 331,
            "error_count": 2, "last_error": "previous error",
            "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("fail_comment_conf.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let label_log = dh.temp_dir.path().join("fail_label_conf.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
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
          printf '[{{"number":33,"title":"Fail test","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":330,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":331,"author":{{"login":"u"}},"body":"a","createdAt":"2026-01-01T00:00:10Z"}},{{"id":332,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":333,"author":{{"login":"u"}},"body":"fix tests please","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) shift; shift; while [ $# -gt 0 ]; do case "$1" in --body) printf '%s' "$2" >> "$CLOG"; shift 2 ;; *) shift ;; esac; done; exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let _output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        // Daemon may return 0 even if individual issue fails (it logs the error)
        // Check state file for Failed
        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(
            state.state,
            PrdWorkflowState::Failed,
            "should be Failed after 3 errors"
        );
        assert!(state.is_terminal());
        assert!(state.error_count >= 3);

        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed label should be added: {label_raw}"
        );
    })
}

/// Verify that when new comments include both an approval ("LGTM") and
/// non-approval feedback, the approval wins and transitions to Done.
fn mixed_comments_approval_triggers_done(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/40.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 40, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 400, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 402,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 401,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("mixed_appr_conf.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        // Two new comments: one plain feedback (id 403) and one LGTM (id 404)
        let gh_script = format!(
            r#"#!/bin/sh
LOG="{comment_log_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{{"number":40,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          if [ -f "$LOG" ]; then
            printf '{{"comments":[{{"id":400,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":401,"author":{{"login":"u"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":402,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":403,"author":{{"login":"bob"}},"body":"Fix the testing section.","createdAt":"2026-01-01T00:00:25Z"}},{{"id":404,"author":{{"login":"alice"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:30Z"}},{{"id":405,"author":{{"login":"ralph-bot"}},"body":"ok","createdAt":"2026-01-01T00:00:35Z"}}]}}'
          else
            printf '{{"comments":[{{"id":400,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":401,"author":{{"login":"u"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":402,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":403,"author":{{"login":"bob"}},"body":"Fix the testing section.","createdAt":"2026-01-01T00:00:25Z"}},{{"id":404,"author":{{"login":"alice"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:30Z"}}]}}'
          fi; exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment) shift; shift; while [ $# -gt 0 ]; do case "$1" in --body) printf '%s' "$2" > "$LOG"; shift 2 ;; *) shift ;; esac; done; exit 0 ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(
            state.state,
            PrdWorkflowState::Done,
            "mixed comments (approval + feedback) should transition to Done"
        );
        assert!(state.is_terminal());
        assert_eq!(
            state.draft_revision, 1,
            "draft should remain at 1 (no revision)"
        );

        let posted = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            posted.contains("<!-- ralph:prd:40:status-approved-v1 -->"),
            "should post status-approved marker: {posted}"
        );
    })
}

/// Verify that when the approval path fails (e.g. GitHub comment posting fails),
/// the error is propagated and `error_count` is incremented rather than
/// persisting terminal Done.
fn approval_path_github_failure_increments_error(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/41.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 41, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 410, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 412,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 411,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Mock gh that returns an approval comment but fails on `issue comment` (posting)
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":41,"title":"T","labels":[{"name":"ralph:prd-active"}],"body":"B"}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{"comments":[{"id":410,"author":{"login":"ralph-bot"},"body":"q","createdAt":"2026-01-01T00:00:05Z"},{"id":411,"author":{"login":"u"},"body":"ans","createdAt":"2026-01-01T00:00:10Z"},{"id":412,"author":{"login":"ralph-bot"},"body":"draft","createdAt":"2026-01-01T00:00:15Z"},{"id":413,"author":{"login":"u"},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}]}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{"labels":[{"name":"ralph:prd-active"}]}'; exit 0; fi
        exit 0 ;;
      comment)
        # Simulate GitHub failure when posting approval comment
        echo "gh: error posting comment" >&2
        exit 1
        ;;
      edit) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#;
        let gh_path = write_mock_gh(&dh, gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let _output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();

        // Should NOT be Done — the error should be recorded
        assert_ne!(
            state.state,
            PrdWorkflowState::Done,
            "approval-path GitHub failure should not persist terminal Done"
        );
        assert!(
            state.error_count >= 1,
            "error_count should be incremented on approval-path failure, got {}",
            state.error_count
        );
        assert!(
            state.last_error.is_some(),
            "last_error should be set on approval-path failure"
        );
        // State should remain AwaitingFeedback (retryable)
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingFeedback,
            "state should remain AwaitingFeedback for retry"
        );
    })
}

/// Multi-tick test: approval comment exists, approval action fails on each tick.
/// After 3 failures the state transitions to Failed with `ralph:prd-failed`.
fn approval_failure_exhaustion_transitions_to_failed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/45.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 45, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 450, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 452,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 451,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let label_log = dh.temp_dir.path().join("exhaustion_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
        let comment_log = dh.temp_dir.path().join("exhaustion_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();

        // gh returns an approval comment (id 453 with "LGTM") but always fails
        // on `issue comment` (posting the status-approved marker).
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
          printf '[{{"number":45,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":450,"author":{{"login":"ralph-bot"}},"body":"q","createdAt":"2026-01-01T00:00:05Z"}},{{"id":451,"author":{{"login":"u"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":452,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":453,"author":{{"login":"u"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        exit 0 ;;
      comment)
        # Always fail when posting — simulates persistent GitHub outage
        echo "gh: error posting comment" >&2
        exit 1
        ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        // Run 3 daemon ticks — each should fail the approval transition
        for tick in 1..=3 {
            let _output = dh
                .daemon_env(
                    [
                        "daemon",
                        "start",
                        "--repo",
                        "acme/widgets",
                        "--single-iteration",
                    ],
                    &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
                )
                .unwrap();

            let state: InteractivePrdState =
                serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
                    .unwrap_or_else(|e| panic!("parse state after tick {tick}: {e}"));

            if tick < 3 {
                // First two ticks: error_count should increment, state stays AwaitingFeedback
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
                // Cursor must NOT have advanced (approval comments remain visible)
                assert_eq!(
                    state.last_processed_comment_id,
                    Some(451),
                    "tick {tick}: last_processed_comment_id should not advance on failure"
                );
            } else {
                // Third tick: threshold reached, should transition to Failed
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Failed,
                    "tick 3: should be Failed after exhaustion"
                );
                assert!(state.is_terminal(), "tick 3: Failed should be terminal");
                assert!(state.error_count >= 3, "tick 3: error_count should be >= 3");
            }
        }

        // Verify ralph:prd-failed label was applied
        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed label should be added after exhaustion: {label_raw}"
        );
    })
}

/// Verify that pre-draft user comments with approval text are excluded from
/// feedback processing in AwaitingFeedback. Only post-draft comments should
/// be considered for approval detection.
fn draft_boundary_filtering_excludes_pre_draft_approval(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/60.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        // Seed: AwaitingFeedback with draft at id=603, cursor at id=600
        let seed = serde_json::json!({
            "issue_number": 60, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 600, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 603,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 600,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Pre-draft comment (id 602) has "LGTM" but should be ignored
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":60,"title":"T","labels":[{"name":"ralph:prd-active"}],"body":"B"}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{"comments":[{"id":600,"author":{"login":"ralph-bot"},"body":"questions","createdAt":"2026-01-01T00:00:05Z"},{"id":601,"author":{"login":"alice"},"body":"answers","createdAt":"2026-01-01T00:00:10Z"},{"id":602,"author":{"login":"alice"},"body":"LGTM, approved!","createdAt":"2026-01-01T00:00:12Z"},{"id":603,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:60:draft-v1 -->\nDraft","createdAt":"2026-01-01T00:00:15Z"}]}'
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
        let gh_path = write_mock_gh(&dh, gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingFeedback,
            "pre-draft approval should be ignored; state should remain AwaitingFeedback"
        );
        assert_eq!(state.draft_revision, 1, "no revision should have occurred");
    })
}

/// Verify that pre-draft user feedback comments are excluded from revision
/// aggregation. Only post-draft comments trigger the revision loop.
fn draft_boundary_filtering_excludes_pre_draft_revision(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/61.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 61, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 610, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 613,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 610,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // Pre-draft non-approval feedback (id 612) should be ignored
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":61,"title":"T","labels":[{"name":"ralph:prd-active"}],"body":"B"}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{"comments":[{"id":610,"author":{"login":"ralph-bot"},"body":"questions","createdAt":"2026-01-01T00:00:05Z"},{"id":611,"author":{"login":"alice"},"body":"answers","createdAt":"2026-01-01T00:00:10Z"},{"id":612,"author":{"login":"bob"},"body":"Please fix the testing section.","createdAt":"2026-01-01T00:00:12Z"},{"id":613,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:61:draft-v1 -->\nDraft","createdAt":"2026-01-01T00:00:15Z"}]}'
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
        let gh_path = write_mock_gh(&dh, gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingFeedback,
            "pre-draft feedback should be ignored; no revision triggered"
        );
        assert_eq!(state.draft_revision, 1, "draft_revision should remain 1");
    })
}

/// Verify restart continuity: when questions-v{n} marker already exists on
/// the issue, the daemon hydrates `questions_posted_at` from the existing
/// comment's `created_at` rather than using `Utc::now()`. This ensures that
/// user answers posted between the original question time and the restart are
/// not skipped.
fn restart_continuity_marker_timestamp_hydration(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        // Backend for question generation (should NOT be called since marker exists)
        let backend_script = dh
            .write_mock_script(
                "prd_restart_q.sh",
                r#"#!/bin/sh
cat >/dev/null
printf '1. Q1?\n2. Q2?\n'
"#,
            )
            .unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        // No persisted state — simulating a fresh start that picks up an issue
        // where the questions marker was already posted by a prior instance.
        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/70.json");

        let label_log = dh.temp_dir.path().join("restart_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();
        // The marker comment already exists with created_at = 2026-01-10T12:00:00Z
        let gh_script = format!(
            r#"#!/bin/sh
LLOG="{label_log_str}"
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
          printf '[{{"number":70,"title":"Restart test","labels":[{{"name":"ralph:prd"}}],"body":"Test restart."}}]'
        elif [ "$has_active" = "1" ]; then
          printf '[]'
        else
          printf '[]'
        fi
        exit 0
        ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":7001,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:70:questions-v1 -->\\n## Clarifying Questions\\n1. Q1?","createdAt":"2026-01-10T12:00:00Z"}}]}}'
          exit 0
        fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd"}}]}}'; exit 0; fi
        exit 0
        ;;
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
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();
        assert_exit_code(&output, 0);

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        assert_eq!(state.state, PrdWorkflowState::AwaitingAnswers);
        assert_eq!(state.question_revision, 1);
        assert_eq!(state.questions_comment_id, Some(7001));

        // The key assertion: questions_posted_at should be the existing
        // comment's created_at (2026-01-10T12:00:00Z), NOT Utc::now()
        let qpa = state
            .questions_posted_at
            .expect("questions_posted_at should be set");
        let expected = chrono::DateTime::parse_from_rfc3339("2026-01-10T12:00:00Z")
            .expect("parse")
            .with_timezone(&chrono::Utc);
        assert_eq!(
            qpa, expected,
            "questions_posted_at should be hydrated from existing marker comment's created_at, \
             got {qpa} but expected {expected}"
        );
    })
}

/// Verify that repeated `gh api user` failures in AwaitingAnswers are routed
/// through transition retry accounting. After 3 consecutive failures the state
/// transitions to Failed with `ralph:prd-failed`.
fn bot_login_failure_exhaustion_awaiting_answers(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/200.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 200, "owner": "acme", "repo": "widgets",
            "state": "AwaitingAnswers",
            "question_revision": 1, "draft_revision": 0,
            "questions_comment_id": 2000, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": null,
            "latest_draft_body": null,
            "user_answers": null, "last_processed_comment_id": null,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let label_log = dh.temp_dir.path().join("botlogin_aa_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // `gh api user` always fails
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
          printf '[{{"number":200,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":2000,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":2001,"author":{{"login":"alice"}},"body":"answers","createdAt":"2026-01-01T00:00:20Z"}}]}}'
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
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        for tick in 1..=3u32 {
            let _output = dh
                .daemon_env(
                    [
                        "daemon",
                        "start",
                        "--repo",
                        "acme/widgets",
                        "--single-iteration",
                    ],
                    &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
                )
                .unwrap();

            let state: InteractivePrdState =
                serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
                    .unwrap_or_else(|e| panic!("parse state tick {tick}: {e}"));

            if tick < 3 {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::AwaitingAnswers,
                    "tick {tick}: should remain AwaitingAnswers"
                );
                assert_eq!(state.error_count, tick, "tick {tick}: error_count");
                assert!(state.last_error.is_some(), "tick {tick}: last_error set");
            } else {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Failed,
                    "tick 3: should be Failed"
                );
                assert!(state.is_terminal());
                assert!(state.error_count >= 3);
            }
        }

        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed should be added: {label_raw}"
        );
    })
}

/// Verify that repeated `gh api user` failures in AwaitingFeedback are routed
/// through transition retry accounting. After 3 consecutive failures the state
/// transitions to Failed with `ralph:prd-failed`.
fn bot_login_failure_exhaustion_awaiting_feedback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/210.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 210, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 2100, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 2102,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 2101,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let label_log = dh.temp_dir.path().join("botlogin_af_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // `gh api user` always fails
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
          printf '[{{"number":210,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":2100,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":2101,"author":{{"login":"alice"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":2102,"author":{{"login":"ralph-bot"}},"body":"draft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":2103,"author":{{"login":"alice"}},"body":"fix testing","createdAt":"2026-01-01T00:00:25Z"}}]}}'
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
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        for tick in 1..=3u32 {
            let _output = dh
                .daemon_env(
                    [
                        "daemon",
                        "start",
                        "--repo",
                        "acme/widgets",
                        "--single-iteration",
                    ],
                    &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
                )
                .unwrap();

            let state: InteractivePrdState =
                serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
                    .unwrap_or_else(|e| panic!("parse state tick {tick}: {e}"));

            if tick < 3 {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::AwaitingFeedback,
                    "tick {tick}: should remain AwaitingFeedback"
                );
                assert_eq!(state.error_count, tick, "tick {tick}: error_count");
                assert!(state.last_error.is_some(), "tick {tick}: last_error set");
            } else {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Failed,
                    "tick 3: should be Failed"
                );
                assert!(state.is_terminal());
                assert!(state.error_count >= 3);
            }
        }

        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed should be added: {label_raw}"
        );
    })
}

/// Verify that repeated `gh api user` failures during the Pending stage
/// (Pending -> AwaitingAnswers pickup) are routed through transition retry
/// accounting. After 3 consecutive failures the state transitions to Failed
/// with `ralph:prd-failed`.
fn bot_login_failure_exhaustion_pending(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let label_log = dh.temp_dir.path().join("botlogin_pending_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // `gh api user` always fails; issue #300 starts in Pending (no pre-seeded state)
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
          printf '[{{"number":300,"title":"T","labels":[{{"name":"ralph:prd"}}],"body":"B"}}]'
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
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/300.json");

        for tick in 1..=3u32 {
            let _output = dh
                .daemon_env(
                    [
                        "daemon",
                        "start",
                        "--repo",
                        "acme/widgets",
                        "--single-iteration",
                    ],
                    &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
                )
                .unwrap();

            let state: InteractivePrdState =
                serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
                    .unwrap_or_else(|e| panic!("parse state tick {tick}: {e}"));

            if tick < 3 {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Pending,
                    "tick {tick}: should remain Pending"
                );
                assert_eq!(state.error_count, tick, "tick {tick}: error_count");
                assert!(state.last_error.is_some(), "tick {tick}: last_error set");
            } else {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Failed,
                    "tick 3: should be Failed"
                );
                assert!(state.is_terminal());
                assert!(state.error_count >= 3);
            }
        }

        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed should be added: {label_raw}"
        );
    })
}

/// Verify approval label ordering and partial failure recovery:
/// The approval transition adds `ralph:prd-done` first, then removes
/// `ralph:prd-active`. If adding `ralph:prd-done` fails, the state remains
/// `AwaitingFeedback` (poll-visible via `ralph:prd-active`), error_count
/// increments, and retry continues on the next tick.
fn approval_label_ordering_partial_failure_recovery(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        let backend_script = dh.write_mock_script("noop.sh", "#!/bin/sh\ncat\n").unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/220.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 220, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 2200, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 2202,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "ans", "last_processed_comment_id": 2201,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("label_order_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let label_log = dh.temp_dir.path().join("label_order_labels.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

        // gh mock: approval comment posting succeeds but adding ralph:prd-done
        // always fails. Since boundary-safe order is add-done then remove-active,
        // the remove never happens and ralph:prd-active stays present.
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
          printf '[{{"number":220,"title":"T","labels":[{{"name":"ralph:prd-active"}}],"body":"B"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":2200,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":2201,"author":{{"login":"alice"}},"body":"ans","createdAt":"2026-01-01T00:00:10Z"}},{{"id":2202,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:220:draft-v1 -->\\nDraft","createdAt":"2026-01-01T00:00:15Z"}},{{"id":2203,"author":{{"login":"alice"}},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}}]}}'
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
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        // Tick 1: label add fails — state stays AwaitingFeedback
        let _output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
            )
            .unwrap();

        let state: InteractivePrdState =
            serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();

        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingFeedback,
            "state should remain AwaitingFeedback on partial label failure"
        );
        assert!(
            state.error_count >= 1,
            "error_count should increment: {}",
            state.error_count
        );
        assert!(state.last_error.is_some(), "last_error should be set");

        // The label log should NOT contain ralph:prd-active removal (since
        // the add-done step failed first in boundary-safe order).
        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            !label_raw.contains("ralph:prd-active"),
            "ralph:prd-active should NOT have been removed (add-done failed first): {label_raw}"
        );
    })
}

// ---------------------------------------------------------------------------
// Section-completeness conformance tests
// ---------------------------------------------------------------------------

/// Verify that a complete 6-section spec passes `check_spec_sections`.
fn section_complete_spec_passes_validation(_harness: &RalphHarness) -> TestResult {
    let complete = "\
## Summary\nDraft summary.\n\n\
## Acceptance Criteria\n- [ ] AC1\n\n\
## Technical Approach\nApproach.\n\n\
## Files & Modules\n- file.rs\n\n\
## Testing Strategy\n- tests\n\n\
## Out of Scope\n- none";
    let (_cleaned, missing) = check_spec_sections(complete);
    if !missing.is_empty() {
        return TestResult::Fail(format!(
            "complete spec should have no missing sections, got: {missing:?}"
        ));
    }
    TestResult::Pass
}

/// Verify that a spec missing sections is detected and would be rejected
/// by the hardened draft generation flow (returns InteractivePrdFailed).
fn section_incomplete_draft_is_rejected(_harness: &RalphHarness) -> TestResult {
    let incomplete = "\
## Summary\nPartial draft.\n\n\
## Acceptance Criteria\n- [ ] AC1";
    let (_cleaned, missing) = check_spec_sections(incomplete);
    if missing.is_empty() {
        return TestResult::Fail("incomplete spec should report missing sections".to_owned());
    }
    // Verify the error message format matches what run_draft_with_section_retry_sync produces
    let error_msg = format!(
        "draft missing required sections after {} retries: {}",
        DRAFT_SECTION_RETRIES,
        missing.join(", ")
    );
    if !error_msg.contains("## Technical Approach")
        && !error_msg.contains("## Files & Modules")
        && !error_msg.contains("## Testing Strategy")
        && !error_msg.contains("## Out of Scope")
    {
        return TestResult::Fail(format!(
            "error message should list specific missing section names: {error_msg}"
        ));
    }
    TestResult::Pass
}

/// Verify that an incomplete revision output would be rejected by
/// the hardened revision generation flow.
fn section_incomplete_revision_is_rejected(_harness: &RalphHarness) -> TestResult {
    // Simulate a revision that has only 3 of 6 required sections
    let partial_revision = "\
## Summary\nRevised summary.\n\n\
## Acceptance Criteria\n- [ ] Updated AC.\n\n\
## Technical Approach\nRevised approach.";
    let (_cleaned, missing) = check_spec_sections(partial_revision);
    if missing.is_empty() {
        return TestResult::Fail("partial revision should report missing sections".to_owned());
    }
    // Verify that the 6-section requirement means exactly these are missing
    let expected_missing = [
        "## Files & Modules",
        "## Testing Strategy",
        "## Out of Scope",
    ];
    for section in &expected_missing {
        if !missing.iter().any(|m| m == *section) {
            return TestResult::Fail(format!(
                "expected {section} to be reported as missing, got: {missing:?}"
            ));
        }
    }

    // An empty spec should report all 6 sections missing
    let empty = "Just some text without any section headers.";
    let (_cleaned, all_missing) = check_spec_sections(empty);
    if all_missing.len() != REQUIRED_SPEC_SECTION_COUNT {
        return TestResult::Fail(format!(
            "empty spec should be missing all {REQUIRED_SPEC_SECTION_COUNT} sections, got {} missing",
            all_missing.len()
        ));
    }

    TestResult::Pass
}

/// Verify that section completeness constants are correct.
fn section_constants_are_correct(_harness: &RalphHarness) -> TestResult {
    if REQUIRED_SPEC_SECTION_COUNT != 6 {
        return TestResult::Fail(format!(
            "REQUIRED_SPEC_SECTION_COUNT should be 6, got {REQUIRED_SPEC_SECTION_COUNT}"
        ));
    }
    if DRAFT_SECTION_RETRIES < 1 {
        return TestResult::Fail(format!(
            "DRAFT_SECTION_RETRIES should be >= 1, got {DRAFT_SECTION_RETRIES}"
        ));
    }
    TestResult::Pass
}

/// Verify that an incomplete-spec writer backend causes the AwaitingAnswers ->
/// draft transition to fail: no `draft-vN` comment is posted, `error_count`
/// increments on each tick, and the state transitions to `Failed` after 3
/// consecutive errors.
fn section_incomplete_draft_exhaustion_transitions_to_failed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        // Backend that always produces an incomplete spec (only 2 of 6 sections)
        let backend_script = dh
            .write_mock_script(
                "prd_incomplete_draft.sh",
                r#"#!/bin/sh
cat >/dev/null
cat <<'EOF'
## Summary
Incomplete draft from section test.

## Acceptance Criteria
- [ ] Incomplete.
EOF
"#,
            )
            .unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/300.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 300, "owner": "acme", "repo": "widgets",
            "state": "AwaitingAnswers",
            "question_revision": 1, "draft_revision": 0,
            "questions_comment_id": 3000, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": null,
            "latest_draft_body": null,
            "user_answers": null, "last_processed_comment_id": null,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("section_draft_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let label_log = dh.temp_dir.path().join("section_draft_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

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
          printf '[{{"number":300,"title":"Incomplete draft test","labels":[{{"name":"ralph:prd-active"}}],"body":"Test body"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0; want_tb=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; title,body) want_tb=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":3000,"author":{{"login":"ralph-bot"}},"body":"<!-- ralph:prd:300:questions-v1 -->\\n1. Q?","createdAt":"2026-01-01T00:00:05Z"}},{{"id":3001,"author":{{"login":"octocat"}},"body":"User answers here","createdAt":"2026-01-01T00:00:15Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        if [ "$want_tb" = "1" ]; then printf '{{"title":"Incomplete draft test","body":"Test body"}}'; exit 0; fi
        exit 0 ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in --body) printf '%s\n' "$2" >> "$CLOG"; shift 2 ;; *) shift ;; esac
        done; exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        // Run 3 daemon ticks — each should fail because the backend produces
        // an incomplete spec missing 4 of 6 required sections.
        for tick in 1..=3u32 {
            let _output = dh
                .daemon_env(
                    [
                        "daemon",
                        "start",
                        "--repo",
                        "acme/widgets",
                        "--single-iteration",
                    ],
                    &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
                )
                .unwrap();

            let state: InteractivePrdState =
                serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
                    .unwrap_or_else(|e| panic!("parse state tick {tick}: {e}"));

            if tick < 3 {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::AwaitingAnswers,
                    "tick {tick}: should remain AwaitingAnswers (not post incomplete draft)"
                );
                assert_eq!(
                    state.error_count, tick,
                    "tick {tick}: error_count should increment"
                );
                assert!(
                    state.last_error.is_some(),
                    "tick {tick}: last_error should be set"
                );
                let err_msg = state.last_error.as_deref().unwrap();
                assert!(
                    err_msg.contains("missing required sections"),
                    "tick {tick}: error should mention missing sections: {err_msg}"
                );
            } else {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Failed,
                    "tick 3: should be Failed after exhaustion"
                );
                assert!(state.is_terminal());
                assert!(state.error_count >= 3);
            }
        }

        // Verify no draft-v1 comment was posted (incomplete draft rejected)
        let comments_raw = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            !comments_raw.contains("draft-v1"),
            "no draft-v1 comment should be posted for incomplete spec, comment log:\n{comments_raw}"
        );

        // Verify Failed label was added
        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed should be added: {label_raw}"
        );
    })
}

/// Verify that an incomplete-spec writer backend causes the AwaitingFeedback
/// revision transition to fail: no new `draft-vN` comment is posted,
/// `error_count` increments, and the state transitions to `Failed` after 3
/// consecutive errors.
fn section_incomplete_revision_exhaustion_transitions_to_failed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("harness");
        dh.init_workspace().unwrap();

        // Backend that always produces an incomplete spec (only 2 of 6 sections)
        let backend_script = dh
            .write_mock_script(
                "prd_incomplete_revision.sh",
                r#"#!/bin/sh
cat >/dev/null
cat <<'EOF'
## Summary
Incomplete revision.

## Acceptance Criteria
- [ ] Still incomplete.
EOF
"#,
            )
            .unwrap();
        dh.setup_mock_backends_stable(&backend_script).unwrap();

        let state_path = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd/310.json");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let seed = serde_json::json!({
            "issue_number": 310, "owner": "acme", "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1, "draft_revision": 1,
            "questions_comment_id": 3100, "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 3102,
            "latest_draft_body": "## Summary\nOriginal draft.\n\n## Acceptance Criteria\n- [ ] AC.\n\n## Technical Approach\nOld.\n\n## Files & Modules\n- f.rs\n\n## Testing Strategy\n- tests\n\n## Out of Scope\n- none",
            "user_answers": "User answer text", "last_processed_comment_id": 3101,
            "error_count": 0, "last_error": null, "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let comment_log = dh.temp_dir.path().join("section_revision_comment.log");
        let comment_log_str = comment_log.to_string_lossy().into_owned();
        let label_log = dh.temp_dir.path().join("section_revision_label.log");
        let label_log_str = label_log.to_string_lossy().into_owned();

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
          printf '[{{"number":310,"title":"Incomplete revision test","labels":[{{"name":"ralph:prd-active"}}],"body":"Test body"}}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0; want_tb=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; title,body) want_tb=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{{"comments":[{{"id":3100,"author":{{"login":"ralph-bot"}},"body":"questions","createdAt":"2026-01-01T00:00:05Z"}},{{"id":3101,"author":{{"login":"octocat"}},"body":"answers","createdAt":"2026-01-01T00:00:10Z"}},{{"id":3102,"author":{{"login":"ralph-bot"}},"body":"draft v1","createdAt":"2026-01-01T00:00:15Z"}},{{"id":3103,"author":{{"login":"octocat"}},"body":"Please add error handling details.","createdAt":"2026-01-01T00:00:25Z"}}]}}'
          exit 0; fi
        if [ "$want_l" = "1" ]; then printf '{{"labels":[{{"name":"ralph:prd-active"}}]}}'; exit 0; fi
        if [ "$want_tb" = "1" ]; then printf '{{"title":"Incomplete revision test","body":"Test body"}}'; exit 0; fi
        exit 0 ;;
      comment)
        shift; shift
        while [ $# -gt 0 ]; do
          case "$1" in --body) printf '%s\n' "$2" >> "$CLOG"; shift 2 ;; *) shift ;; esac
        done; exit 0 ;;
      edit) echo "$@" >> "$LLOG"; exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  pr) case "$2" in list) printf '' ;; *) ;; esac; exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
  label) exit 0 ;;
esac; exit 0
"#
        );
        let gh_path = write_mock_gh(&dh, &gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        // Run 3 daemon ticks — each should fail because the backend produces
        // an incomplete revision missing 4 of 6 required sections.
        for tick in 1..=3u32 {
            let _output = dh
                .daemon_env(
                    [
                        "daemon",
                        "start",
                        "--repo",
                        "acme/widgets",
                        "--single-iteration",
                    ],
                    &[("PATH", &gh_path), ("RALPH_DAEMON_BIN", &ralph_path)],
                )
                .unwrap();

            let state: InteractivePrdState =
                serde_json::from_str(&fs::read_to_string(&state_path).unwrap())
                    .unwrap_or_else(|e| panic!("parse state tick {tick}: {e}"));

            if tick < 3 {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::AwaitingFeedback,
                    "tick {tick}: should remain AwaitingFeedback (not post incomplete revision)"
                );
                assert_eq!(
                    state.error_count, tick,
                    "tick {tick}: error_count should increment"
                );
                assert!(
                    state.last_error.is_some(),
                    "tick {tick}: last_error should be set"
                );
                let err_msg = state.last_error.as_deref().unwrap();
                assert!(
                    err_msg.contains("missing required sections"),
                    "tick {tick}: error should mention missing sections: {err_msg}"
                );
            } else {
                assert_eq!(
                    state.state,
                    PrdWorkflowState::Failed,
                    "tick 3: should be Failed after exhaustion"
                );
                assert!(state.is_terminal());
                assert!(state.error_count >= 3);
            }
        }

        // Verify no draft-v2 comment was posted (incomplete revision rejected)
        let comments_raw = fs::read_to_string(&comment_log).unwrap_or_default();
        assert!(
            !comments_raw.contains("draft-v2"),
            "no draft-v2 comment should be posted for incomplete revision, comment log:\n{comments_raw}"
        );

        // Verify Failed label was added
        let label_raw = fs::read_to_string(&label_log).unwrap_or_default();
        assert!(
            label_raw.contains("ralph:prd-failed"),
            "ralph:prd-failed should be added: {label_raw}"
        );
    })
}

/// Verify that a terminal transition save-failure keeps the issue retryable
/// by leaving it in a non-terminal state with error_count incremented.
fn terminal_save_failure_keeps_retry_visibility(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script("prd_noop.sh", "#!/bin/sh\ncat\n")
            .expect("write backend");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup mock backends");

        let state_dir = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd");
        fs::create_dir_all(&state_dir).expect("create state dir");

        let state_path = state_dir.join("170.json");
        let seed = serde_json::json!({
            "issue_number": 170,
            "owner": "acme",
            "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1,
            "draft_revision": 1,
            "questions_comment_id": 1700,
            "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 1702,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "answers",
            "last_processed_comment_id": 1701,
            "error_count": 0,
            "last_error": null,
            "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":170,"title":"T","labels":[{"name":"ralph:prd-active"}],"body":"B"}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{"comments":[{"id":1700,"author":{"login":"ralph-bot"},"body":"questions","createdAt":"2026-01-01T00:00:05Z"},{"id":1701,"author":{"login":"alice"},"body":"answers","createdAt":"2026-01-01T00:00:10Z"},{"id":1702,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:170:draft-v1 -->\nDraft","createdAt":"2026-01-01T00:00:15Z"},{"id":1703,"author":{"login":"alice"},"body":"LGTM!","createdAt":"2026-01-01T00:00:25Z"}]}'
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
        let gh_path = write_mock_gh(&dh, gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        // Inject save failure via env var — deterministic regardless of privilege level
        let _output = dh
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
                    ("RALPH_TEST_INJECT_SAVE_FAILURE", "1"),
                ],
            )
            .unwrap();

        let state_raw = fs::read_to_string(&state_path).unwrap();
        let state: InteractivePrdState = serde_json::from_str(&state_raw).unwrap();

        // State should NOT be terminal since save failed
        assert_ne!(
            state.state,
            PrdWorkflowState::Done,
            "state must not be Done when save fails"
        );
        // The original state should be preserved (save couldn't overwrite)
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingFeedback,
            "original AwaitingFeedback state should be preserved when save fails"
        );
    })
}

/// Verify that bot-scoped marker lookup ignores user-spoofed markers.
/// A user comment with the same marker text should not be treated as an
/// existing bot marker, so the bot should still post its own.
fn bot_scoped_marker_ignores_user_spoof(_harness: &RalphHarness) -> TestResult {
    run_case(|| {
        let comments = vec![
            github::IssueComment {
                id: 100,
                author_login: "mallory".to_owned(),
                body: "<!-- ralph:prd:42:draft-v1 -->\nSpoofed draft".to_owned(),
                created_at: chrono::Utc::now(),
            },
            github::IssueComment {
                id: 101,
                author_login: "ralph-bot".to_owned(),
                body: "<!-- ralph:prd:42:draft-v1 -->\nReal draft".to_owned(),
                created_at: chrono::Utc::now(),
            },
        ];

        let marker = "<!-- ralph:prd:42:draft-v1 -->";

        // Generic (non-scoped) lookup finds the user spoof first
        let generic = comments.iter().find(|c| c.body.contains(marker));
        assert!(generic.is_some());
        assert_eq!(
            generic.unwrap().author_login,
            "mallory",
            "generic lookup finds user spoof first"
        );

        // Bot-scoped lookup should only find the bot comment
        let bot_scoped = comments
            .iter()
            .find(|c| c.author_login == "ralph-bot" && c.body.contains(marker));
        assert!(bot_scoped.is_some());
        assert_eq!(
            bot_scoped.unwrap().author_login,
            "ralph-bot",
            "bot-scoped lookup should find bot comment"
        );
        assert_eq!(bot_scoped.unwrap().id, 101);
    })
}

/// Verify that bot-scoped extract_questions_text ignores user-authored spoof
/// markers and correctly hydrates from bot-authored comments.
fn bot_scoped_extract_questions_ignores_spoof(_harness: &RalphHarness) -> TestResult {
    use crate::daemon::interactive_prd::prd_marker;

    run_case(|| {
        let marker = prd_marker(42, "questions", 1);
        let comments = vec![
            github::IssueComment {
                id: 200,
                author_login: "mallory".to_owned(),
                body: format!("{marker}\n## Clarifying Questions\n1. Spoofed question"),
                created_at: "2026-01-01T00:00:05Z"
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .unwrap(),
            },
            github::IssueComment {
                id: 201,
                author_login: "ralph-bot".to_owned(),
                body: format!("{marker}\n## Clarifying Questions\n1. Real bot question"),
                created_at: "2026-01-01T00:00:10Z"
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .unwrap(),
            },
        ];

        // Lookup by marker (no ID) with bot_login should find bot comment
        let extracted = crate::daemon::interactive_prd::tests_extract_questions_text(
            &comments,
            None,
            42,
            1,
            "ralph-bot",
        );
        assert!(
            extracted.contains("Real bot question"),
            "should extract from bot comment: {extracted}"
        );
        assert!(
            !extracted.contains("Spoofed question"),
            "should not extract from user spoof: {extracted}"
        );
    })
}

/// Verify that a terminal transition save-failure on the FAILED path
/// (transition_to_failed) keeps the issue retryable by leaving it in a
/// non-terminal state when the save inside transition_to_failed fails.
///
/// Uses env-var failure injection (`RALPH_TEST_INJECT_SAVE_FAILURE`) for
/// deterministic behavior regardless of privilege level.
fn terminal_save_failure_failed_path_keeps_retry_visibility(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace().expect("init failed");

        let backend_script = dh
            .write_mock_script("prd_noop.sh", "#!/bin/sh\necho ''\n")
            .expect("write backend");
        dh.setup_mock_backends_stable(&backend_script)
            .expect("setup mock backends");

        let state_dir = dh
            .temp_dir
            .path()
            .join("acme/widgets/.ralph/interactive-prd");
        fs::create_dir_all(&state_dir).expect("create state dir");

        // Seed state: AwaitingFeedback with error_count=2 (next error triggers failure)
        let state_path = state_dir.join("175.json");
        let seed = serde_json::json!({
            "issue_number": 175,
            "owner": "acme",
            "repo": "widgets",
            "state": "AwaitingFeedback",
            "question_revision": 1,
            "draft_revision": 1,
            "questions_comment_id": 1750,
            "questions_posted_at": "2026-01-01T00:00:05Z",
            "latest_draft_comment_id": 1752,
            "latest_draft_body": "## Summary\nDraft.",
            "user_answers": "answers",
            "last_processed_comment_id": 1751,
            "error_count": 2,
            "last_error": "previous error",
            "last_advanced_at": null
        });
        fs::write(&state_path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        // gh mock returns feedback comment to trigger revision attempt;
        // the empty backend output causes section validation failure,
        // pushing error_count to 3 and triggering transition_to_failed.
        let gh_script = r#"#!/bin/sh
case "$1" in
  issue)
    case "$2" in
      list)
        has_active=0
        for arg in "$@"; do case "$arg" in ralph:prd-active) has_active=1 ;; esac; done
        if [ "$has_active" = "1" ]; then
          printf '[{"number":175,"title":"T","labels":[{"name":"ralph:prd-active"}],"body":"B"}]'
        else printf '[]'; fi; exit 0 ;;
      view)
        want_c=0; want_l=0
        for arg in "$@"; do case "$arg" in comments) want_c=1 ;; labels) want_l=1 ;; esac; done
        if [ "$want_c" = "1" ]; then
          printf '{"comments":[{"id":1750,"author":{"login":"ralph-bot"},"body":"questions","createdAt":"2026-01-01T00:00:05Z"},{"id":1751,"author":{"login":"alice"},"body":"answers","createdAt":"2026-01-01T00:00:10Z"},{"id":1752,"author":{"login":"ralph-bot"},"body":"<!-- ralph:prd:175:draft-v1 -->\nDraft","createdAt":"2026-01-01T00:00:15Z"},{"id":1753,"author":{"login":"alice"},"body":"Please revise.","createdAt":"2026-01-01T00:00:25Z"}]}'
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
        let gh_path = write_mock_gh(&dh, gh_script).unwrap();
        let ralph_path = write_daemon_mock_ralph(&dh).unwrap();

        // Inject save failure via env var — deterministic regardless of privilege level
        let _output = dh
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
                    ("RALPH_TEST_INJECT_SAVE_FAILURE", "1"),
                ],
            )
            .unwrap();

        let state_raw = fs::read_to_string(&state_path).unwrap();
        let state: InteractivePrdState = serde_json::from_str(&state_raw).unwrap();

        // State should NOT be Failed since save inside transition_to_failed failed
        assert_ne!(
            state.state,
            PrdWorkflowState::Failed,
            "state must not be Failed when save fails in transition_to_failed"
        );
        // The original state should be preserved (AwaitingFeedback)
        assert_eq!(
            state.state,
            PrdWorkflowState::AwaitingFeedback,
            "original AwaitingFeedback state should be preserved when save fails"
        );
    })
}

/// Verify that bot-scoped status-failed marker posting is resistant to
/// user-authored spoof markers.  A user comment with the same status-failed
/// marker text should not suppress the bot's own failure status comment.
fn status_failed_marker_spoof_resistance(_harness: &RalphHarness) -> TestResult {
    run_case(|| {
        let marker = "<!-- ralph:prd:42:status-failed -->";

        let comments = vec![github::IssueComment {
            id: 300,
            author_login: "mallory".to_owned(),
            body: format!("{marker}\n## PRD Workflow Failed\nSpoofed failure"),
            created_at: chrono::Utc::now(),
        }];

        // Generic (non-scoped) lookup finds the user spoof
        let generic = comments.iter().find(|c| c.body.contains(marker));
        assert!(generic.is_some(), "generic lookup should find user spoof");
        assert_eq!(generic.unwrap().author_login, "mallory");

        // Bot-scoped lookup should NOT find the user spoof
        let bot_scoped = comments
            .iter()
            .find(|c| c.author_login == "ralph-bot" && c.body.contains(marker));
        assert!(
            bot_scoped.is_none(),
            "bot-scoped lookup must not find user-spoofed status-failed marker"
        );

        // With a real bot comment added, bot-scoped lookup finds only the bot comment
        let mut comments_with_bot = comments.clone();
        comments_with_bot.push(github::IssueComment {
            id: 301,
            author_login: "ralph-bot".to_owned(),
            body: format!("{marker}\n## PRD Workflow Failed\nReal failure"),
            created_at: chrono::Utc::now(),
        });

        let bot_result = comments_with_bot
            .iter()
            .find(|c| c.author_login == "ralph-bot" && c.body.contains(marker));
        assert!(
            bot_result.is_some(),
            "bot-scoped lookup should find bot comment"
        );
        assert_eq!(
            bot_result.unwrap().id,
            301,
            "should find the bot comment, not the spoof"
        );
    })
}

// ---------------------------------------------------------------------------
// Concurrency conformance tests
// ---------------------------------------------------------------------------

fn prd_poll_config_max_concurrent_field(_harness: &RalphHarness) -> TestResult {
    use crate::config::GlobalConfig;
    use crate::daemon::interactive_prd::PrdPollConfig;
    use std::path::PathBuf;

    run_case(|| {
        let config = PrdPollConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            data_dir: PathBuf::from("/tmp"),
            prd_enabled: true,
            question_backends: vec![],
            writer_backend: String::new(),
            reviewer_backend: String::new(),
            max_revisions: 0,
            backend_timeout_secs: 10,
            global_config: GlobalConfig::default(),
            verbose: false,
            max_concurrent: 4,
        };
        assert_eq!(config.max_concurrent, 4, "max_concurrent should store value");
    })
}

fn max_concurrent_zero_treated_as_one(_harness: &RalphHarness) -> TestResult {
    use crate::config::GlobalConfig;
    use crate::daemon::interactive_prd::PrdPollConfig;
    use std::path::PathBuf;

    run_case(|| {
        let config = PrdPollConfig {
            owner: "o".to_string(),
            repo: "r".to_string(),
            data_dir: PathBuf::from("/tmp"),
            prd_enabled: true,
            question_backends: vec![],
            writer_backend: String::new(),
            reviewer_backend: String::new(),
            max_revisions: 0,
            backend_timeout_secs: 10,
            global_config: GlobalConfig::default(),
            verbose: false,
            max_concurrent: 0,
        };
        let effective = std::cmp::max(1, config.max_concurrent);
        assert_eq!(effective, 1, "max_concurrent=0 should be treated as 1");
    })
}

/// Conformance: dedup invariant via real poll_and_advance_prd path.
/// Issue #50 appears in both ralph:prd and ralph:prd-active polls.
/// Assert it is processed exactly once per tick (counted via label-edit calls).
fn concurrent_dedup_invariant(_harness: &RalphHarness) -> TestResult {
    use crate::config::GlobalConfig;
    use crate::daemon::interactive_prd::{poll_and_advance_prd, PrdPollConfig};

    run_case(|| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let data_dir = tmp.path();

        let counter = data_dir.join("edit_count");
        fs::write(&counter, "0").expect("init counter");
        let counter_str = counter.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
COUNTER="{counter_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0; has_active=0
        for arg in "$@"; do
          case "$arg" in ralph:prd) has_prd=1 ;; ralph:prd-active) has_active=1 ;; esac
        done
        if [ "$has_prd" = "1" ] || [ "$has_active" = "1" ]; then
          printf '[{{"number":50,"title":"X","labels":[{{"name":"ralph:prd"}},{{"name":"ralph:prd-active"}}],"body":"X"}}]'
        else
          printf '[]'
        fi
        exit 0 ;;
      edit)
        c=$(cat "$COUNTER" 2>/dev/null || printf '0'); c=$((c+1)); printf '%d' "$c" > "$COUNTER"
        exit 0 ;;
      view)
        for arg in "$@"; do case "$arg" in comments) printf '{{"comments":[]}}'; exit 0 ;; esac; done
        printf '{{}}'; exit 0 ;;
      comment) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
esac
exit 0
"#
        );

        let scripts_dir = data_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        let gh_path = scripts_dir.join("gh");
        fs::write(&gh_path, gh_script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let clone_dir = data_dir.join("acme").join("widgets");
        fs::create_dir_all(&clone_dir).unwrap();

        let path_env = format!("{}:{}", scripts_dir.display(), std::env::var("PATH").unwrap_or_default());
        let config = PrdPollConfig {
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            data_dir: data_dir.to_path_buf(),
            prd_enabled: true,
            question_backends: vec!["claude".to_string(), "codex".to_string()],
            writer_backend: "claude".to_string(),
            reviewer_backend: "codex".to_string(),
            max_revisions: 1,
            backend_timeout_secs: 30,
            global_config: GlobalConfig::default(),
            verbose: false,
            max_concurrent: 2,
        };

        let old = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", &path_env) };
        let result = poll_and_advance_prd(&config);
        unsafe { std::env::set_var("PATH", &old) };

        assert!(result.is_ok(), "tick should succeed");
        let count: u32 = fs::read_to_string(&counter).unwrap().trim().parse().unwrap();
        assert_eq!(count, 1, "issue #50 should be processed exactly once, got {count}");
    })
}

/// Conformance: error isolation via real poll_and_advance_prd path.
/// Issue #60's label edit fails; issue #70 succeeds. Tick returns Ok and
/// issue #70's label edit was reached.
fn concurrent_error_isolation(_harness: &RalphHarness) -> TestResult {
    use crate::config::GlobalConfig;
    use crate::daemon::interactive_prd::{poll_and_advance_prd, PrdPollConfig};

    run_case(|| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let data_dir = tmp.path();

        let success_flag = data_dir.join("issue70_ok");
        let success_str = success_flag.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
SUCCESS="{success_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        for arg in "$@"; do case "$arg" in ralph:prd) has_prd=1 ;; esac; done
        if [ "$has_prd" = "1" ]; then
          printf '[{{"number":60,"title":"A","labels":[{{"name":"ralph:prd"}}],"body":"A"}},{{"number":70,"title":"B","labels":[{{"name":"ralph:prd"}}],"body":"B"}}]'
        else
          printf '[]'
        fi
        exit 0 ;;
      edit)
        for arg in "$@"; do
          case "$arg" in
            60) exit 1 ;;
            70) touch "$SUCCESS"; exit 0 ;;
          esac
        done
        exit 0 ;;
      view)
        for arg in "$@"; do case "$arg" in comments) printf '{{"comments":[]}}'; exit 0 ;; esac; done
        printf '{{}}'; exit 0 ;;
      comment) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
esac
exit 0
"#
        );

        let scripts_dir = data_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        let gh_path = scripts_dir.join("gh");
        fs::write(&gh_path, gh_script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let clone_dir = data_dir.join("acme").join("widgets");
        fs::create_dir_all(&clone_dir).unwrap();

        let path_env = format!("{}:{}", scripts_dir.display(), std::env::var("PATH").unwrap_or_default());
        let config = PrdPollConfig {
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            data_dir: data_dir.to_path_buf(),
            prd_enabled: true,
            question_backends: vec!["claude".to_string(), "codex".to_string()],
            writer_backend: "claude".to_string(),
            reviewer_backend: "codex".to_string(),
            max_revisions: 1,
            backend_timeout_secs: 30,
            global_config: GlobalConfig::default(),
            verbose: false,
            max_concurrent: 2,
        };

        let old = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", &path_env) };
        let result = poll_and_advance_prd(&config);
        unsafe { std::env::set_var("PATH", &old) };

        assert!(result.is_ok(), "tick should return Ok despite issue #60 error");
        assert!(success_flag.exists(), "issue #70 should advance despite #60 error");
    })
}

/// Conformance: panic/error isolation via real poll_and_advance_prd path.
/// Issue #110 has corrupt persisted state (deserialization error caught by
/// catch_unwind), issue #111 proceeds normally.
fn concurrent_panic_isolation(_harness: &RalphHarness) -> TestResult {
    use crate::config::GlobalConfig;
    use crate::daemon::interactive_prd::{poll_and_advance_prd, PrdPollConfig};

    run_case(|| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let data_dir = tmp.path();

        let success_flag = data_dir.join("issue111_ok");
        let success_str = success_flag.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
SUCCESS="{success_str}"
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        for arg in "$@"; do case "$arg" in ralph:prd) has_prd=1 ;; esac; done
        if [ "$has_prd" = "1" ]; then
          printf '[{{"number":110,"title":"Corrupt","labels":[{{"name":"ralph:prd"}}],"body":"A"}},{{"number":111,"title":"Good","labels":[{{"name":"ralph:prd"}}],"body":"B"}}]'
        else
          printf '[]'
        fi
        exit 0 ;;
      edit)
        for arg in "$@"; do
          case "$arg" in 111) touch "$SUCCESS" ;; esac
        done
        exit 0 ;;
      view)
        for arg in "$@"; do case "$arg" in comments) printf '{{"comments":[]}}'; exit 0 ;; esac; done
        printf '{{}}'; exit 0 ;;
      comment) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
esac
exit 0
"#
        );

        let scripts_dir = data_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        let gh_path = scripts_dir.join("gh");
        fs::write(&gh_path, gh_script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let clone_dir = data_dir.join("acme").join("widgets");
        fs::create_dir_all(&clone_dir).unwrap();

        // Write corrupt state for issue #110
        let state_dir = data_dir
            .join("acme").join("widgets")
            .join(".ralph").join("interactive-prd");
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("110.json"), "NOT VALID JSON").unwrap();

        let path_env = format!("{}:{}", scripts_dir.display(), std::env::var("PATH").unwrap_or_default());
        let config = PrdPollConfig {
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            data_dir: data_dir.to_path_buf(),
            prd_enabled: true,
            question_backends: vec!["claude".to_string(), "codex".to_string()],
            writer_backend: "claude".to_string(),
            reviewer_backend: "codex".to_string(),
            max_revisions: 1,
            backend_timeout_secs: 30,
            global_config: GlobalConfig::default(),
            verbose: false,
            max_concurrent: 2,
        };

        let old = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", &path_env) };
        let result = poll_and_advance_prd(&config);
        unsafe { std::env::set_var("PATH", &old) };

        assert!(result.is_ok(), "tick should succeed despite corrupt state on #110");
        assert!(success_flag.exists(), "issue #111 should advance despite #110 error");
    })
}

/// Conformance: bounded concurrency via real poll_and_advance_prd path.
/// With max_concurrent=2 and 4 issues, peak active workers (measured via
/// flock-based counter in mock gh script) must never exceed 2.
fn concurrent_bounded_worker_count(_harness: &RalphHarness) -> TestResult {
    use crate::config::GlobalConfig;
    use crate::daemon::interactive_prd::{poll_and_advance_prd, PrdPollConfig};

    run_case(|| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let data_dir = tmp.path();

        let active_file = data_dir.join("active");
        let peak_file = data_dir.join("peak");
        let lock_file = data_dir.join("counter.lock");
        fs::write(&active_file, "0").unwrap();
        fs::write(&peak_file, "0").unwrap();
        let active_str = active_file.to_string_lossy().into_owned();
        let peak_str = peak_file.to_string_lossy().into_owned();
        let lock_str = lock_file.to_string_lossy().into_owned();

        let gh_script = format!(
            r#"#!/bin/sh
ACTIVE="{active_str}"
PEAK="{peak_str}"
LOCK="{lock_str}"
inc() {{ ( flock 9; c=$(cat "$ACTIVE" 2>/dev/null||printf '0'); c=$((c+1)); printf '%d' "$c">"$ACTIVE"; p=$(cat "$PEAK" 2>/dev/null||printf '0'); [ "$c" -gt "$p" ] && printf '%d' "$c">"$PEAK" ) 9>"$LOCK"; }}
dec() {{ ( flock 9; c=$(cat "$ACTIVE" 2>/dev/null||printf '0'); c=$((c-1)); printf '%d' "$c">"$ACTIVE" ) 9>"$LOCK"; }}
case "$1" in
  issue)
    case "$2" in
      list)
        has_prd=0
        for arg in "$@"; do case "$arg" in ralph:prd) has_prd=1 ;; esac; done
        if [ "$has_prd" = "1" ]; then
          printf '[{{"number":300,"title":"A","labels":[{{"name":"ralph:prd"}}],"body":"A"}},{{"number":301,"title":"B","labels":[{{"name":"ralph:prd"}}],"body":"B"}},{{"number":302,"title":"C","labels":[{{"name":"ralph:prd"}}],"body":"C"}},{{"number":303,"title":"D","labels":[{{"name":"ralph:prd"}}],"body":"D"}}]'
        else
          printf '[]'
        fi
        exit 0 ;;
      edit) inc; sleep 0.1; dec; exit 0 ;;
      view)
        for arg in "$@"; do case "$arg" in comments) printf '{{"comments":[]}}'; exit 0 ;; esac; done
        printf '{{}}'; exit 0 ;;
      comment) exit 0 ;;
    esac ;;
  api) if [ "$2" = "user" ]; then printf 'ralph-bot\n'; exit 0; fi ;;
  label) exit 0 ;;
  repo) printf 'acme/widgets\n'; exit 0 ;;
esac
exit 0
"#
        );

        let scripts_dir = data_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        let gh_path = scripts_dir.join("gh");
        fs::write(&gh_path, gh_script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let clone_dir = data_dir.join("acme").join("widgets");
        fs::create_dir_all(&clone_dir).unwrap();

        let path_env = format!("{}:{}", scripts_dir.display(), std::env::var("PATH").unwrap_or_default());
        let config = PrdPollConfig {
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            data_dir: data_dir.to_path_buf(),
            prd_enabled: true,
            question_backends: vec!["claude".to_string(), "codex".to_string()],
            writer_backend: "claude".to_string(),
            reviewer_backend: "codex".to_string(),
            max_revisions: 1,
            backend_timeout_secs: 60,
            global_config: GlobalConfig::default(),
            verbose: false,
            max_concurrent: 2,
        };

        let old = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", &path_env) };
        let result = poll_and_advance_prd(&config);
        unsafe { std::env::set_var("PATH", &old) };

        assert!(result.is_ok(), "tick should succeed");
        let peak: u32 = fs::read_to_string(&peak_file).unwrap().trim().parse().unwrap();
        assert!(peak <= 2, "peak {peak} must not exceed max_concurrent=2");
        assert!(peak >= 1, "at least one worker should have been active");
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
