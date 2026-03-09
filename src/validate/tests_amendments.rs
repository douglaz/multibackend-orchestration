use super::*;

use chrono::Utc;
use std::fs;

use crate::project::amendments::{
    enqueue_amendment, pending_amendment_count, AmendmentPriority, AmendmentRequest,
    AmendmentSource,
};
use crate::validate::assertions::{
    assert_exit_code, assert_file_exists, assert_path_not_exists, assert_stderr_contains,
    strip_ansi,
};
use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "amendments::amend_enqueues_valid_json",
            func: amend_enqueues_valid_json,
        },
        ConformanceTest {
            name: "amendments::amend_uses_default_priority",
            func: amend_uses_default_priority,
        },
        ConformanceTest {
            name: "amendments::amend_rejects_invalid_priority",
            func: amend_rejects_invalid_priority,
        },
        ConformanceTest {
            name: "amendments::amend_reads_body_from_file",
            func: amend_reads_body_from_file,
        },
        ConformanceTest {
            name: "amendments::amend_fails_without_project",
            func: amend_fails_without_project,
        },
        ConformanceTest {
            name: "amendments::amend_fails_for_missing_body_file",
            func: amend_fails_for_missing_body_file,
        },
        ConformanceTest {
            name: "amendments::amend_rejects_nonexistent_project",
            func: amend_rejects_nonexistent_project,
        },
        ConformanceTest {
            name: "amendments::amend_invalid_priority_creates_no_queue_files",
            func: amend_invalid_priority_creates_no_queue_files,
        },
        ConformanceTest {
            name: "amendments::amend_missing_body_file_creates_no_queue_files",
            func: amend_missing_body_file_creates_no_queue_files,
        },
        ConformanceTest {
            name: "amendments::standard_planner_drains_and_injects_amendments",
            func: standard_planner_drains_and_injects_amendments,
        },
        ConformanceTest {
            name: "amendments::quick_dev_drains_and_injects_amendments",
            func: quick_dev_drains_and_injects_amendments,
        },
        ConformanceTest {
            name: "amendments::completion_guard_rejects_with_pending_amendments",
            func: completion_guard_rejects_with_pending_amendments,
        },
        ConformanceTest {
            name: "amendments::unify_config_default_off",
            func: unify_config_default_off,
        },
        ConformanceTest {
            name: "amendments::unify_planner_dedupe_excludes_final_review",
            func: unify_planner_dedupe_excludes_final_review,
        },
        ConformanceTest {
            name: "amendments::unify_mirroring_enqueues_final_review_amendments",
            func: unify_mirroring_enqueues_final_review_amendments,
        },
    ]
}

fn amend_enqueues_valid_json(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-test", "Amend Test", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-test",
                "--body",
                "fix the authentication bug",
                "--priority",
                "P1",
                "--id",
                "EXT-001",
            ])
            .expect("ralph amend should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let queue_path = stdout.trim();
        assert!(
            !queue_path.is_empty(),
            "stdout should contain the queue file path"
        );
        assert!(
            queue_path.ends_with(".json"),
            "queue path should end with .json: {queue_path}"
        );
        assert_file_exists(std::path::Path::new(queue_path));

        // Verify the queued file is valid JSON with expected fields
        let raw = fs::read_to_string(queue_path).expect("read queue file");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse queue JSON");
        assert_eq!(value["id"], "EXT-001");
        assert_eq!(value["body"], "fix the authentication bug");
        assert_eq!(value["priority"], "P1");
        assert_eq!(value["source"], "cli");
    })
}

fn amend_uses_default_priority(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-defaults", "Amend Defaults", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-defaults",
                "--body",
                "some amendment",
            ])
            .expect("ralph amend should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let queue_path = stdout.trim();
        let raw = fs::read_to_string(queue_path).expect("read queue file");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse queue JSON");
        assert_eq!(value["priority"], "P2");

        // Verify auto-generated ID starts with EXT-
        let id = value["id"].as_str().expect("id should be a string");
        assert!(
            id.starts_with("EXT-"),
            "default id should start with EXT-: {id}"
        );
    })
}

fn amend_rejects_invalid_priority(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-invalid", "Amend Invalid", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-invalid",
                "--body",
                "some amendment",
                "--priority",
                "HIGH",
            ])
            .expect("ralph amend should execute");

        // Should fail with non-zero exit code
        assert!(
            !output.status.success(),
            "invalid priority should cause non-zero exit"
        );
    })
}

fn amend_reads_body_from_file(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-file", "Amend File", "test prompt")
            .expect("create project");

        let body_path = h.temp_dir.path().join("amendment-body.txt");
        fs::write(&body_path, "body loaded from file").expect("write body file");

        let body_arg = format!("@{}", body_path.display());
        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-file",
                "--body",
                &body_arg,
            ])
            .expect("ralph amend should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let queue_path = stdout.trim();
        let raw = fs::read_to_string(queue_path).expect("read queue file");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse queue JSON");
        assert_eq!(value["body"], "body loaded from file");
    })
}

fn amend_fails_without_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        // No project created and no active project set

        let output = h
            .ralph(vec!["amend", "--body", "some amendment"])
            .expect("ralph amend should execute");

        assert!(
            !output.status.success(),
            "amend without project should fail"
        );
    })
}

fn amend_fails_for_missing_body_file(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-missing-file", "Amend Missing File", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-missing-file",
                "--body",
                "@/nonexistent/path/to/body.txt",
            ])
            .expect("ralph amend should execute");

        assert!(
            !output.status.success(),
            "amend with missing body file should fail"
        );
    })
}

fn amend_rejects_nonexistent_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-exists", "Amend Exists", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "nonexistent-project-xyz",
                "--body",
                "should fail",
            ])
            .expect("ralph amend should execute");

        assert!(
            !output.status.success(),
            "amend with nonexistent project should fail with non-zero exit"
        );

        // No queue directory should be created for the nonexistent project
        let orphan_queue_dir = h
            .project_dir("nonexistent-project-xyz")
            .join("amendment-queue");
        assert_path_not_exists(&orphan_queue_dir);
    })
}

fn amend_invalid_priority_creates_no_queue_files(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-no-queue-pri", "Amend No Queue Pri", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-no-queue-pri",
                "--body",
                "should not be enqueued",
                "--priority",
                "INVALID",
            ])
            .expect("ralph amend should execute");

        assert!(
            !output.status.success(),
            "invalid priority should cause non-zero exit"
        );

        // No queue files should be created
        let queue_dir = h.project_dir("amend-no-queue-pri").join("amendment-queue");
        if queue_dir.exists() {
            let json_count = fs::read_dir(&queue_dir)
                .expect("read queue dir")
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "json")
                        && !e
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".tmp-")
                })
                .count();
            assert_eq!(
                json_count, 0,
                "no published .json queue files should exist after invalid priority"
            );
        }
    })
}

fn amend_missing_body_file_creates_no_queue_files(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("amend-no-queue-file", "Amend No Queue File", "test prompt")
            .expect("create project");

        let output = h
            .ralph(vec![
                "amend",
                "--project",
                "amend-no-queue-file",
                "--body",
                "@/nonexistent/path/to/body.txt",
            ])
            .expect("ralph amend should execute");

        assert!(
            !output.status.success(),
            "missing body file should cause non-zero exit"
        );

        // No queue files should be created
        let queue_dir = h
            .project_dir("amend-no-queue-file")
            .join("amendment-queue");
        if queue_dir.exists() {
            let json_count = fs::read_dir(&queue_dir)
                .expect("read queue dir")
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "json")
                        && !e
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".tmp-")
                })
                .count();
            assert_eq!(
                json_count, 0,
                "no published .json queue files should exist after missing body file"
            );
        }
    })
}

fn standard_planner_drains_and_injects_amendments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "amend-plan-inject";
        h.init_workspace().expect("init workspace");

        let mock = h
            .write_stable_mock_script(
                "amend-standard-injection.sh",
                &standard_planner_injection_mock_script(),
            )
            .expect("write planner injection mock");
        h.setup_mock_backends_stable(&mock)
            .expect("setup mock backends");

        h.create_project(
            project_id,
            "Planner Amendment Injection",
            "planner amendment injection prompt",
        )
        .expect("create project");
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable qa");

        enqueue_external_amendment(
            h,
            project_id,
            "EXT-PLAN-001",
            "planner external amendment body",
        );
        let output = h
            .ralph(["run", "--project", project_id, "--loops", "1", "--skip-commit"])
            .expect("ralph run should execute");
        assert_exit_code(&output, 0);

        let pending =
            pending_amendment_count(&h.project_dir(project_id)).expect("pending amendment count");
        assert_eq!(pending, 0, "queue should be empty after planner drain");
    })
}

fn quick_dev_drains_and_injects_amendments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "amend-qd-inject";
        h.init_workspace().expect("init workspace");

        let impl_mock = h
            .write_stable_mock_script(
                "amend-quick-dev-impl.sh",
                &quick_dev_injection_implementer_mock_script(),
            )
            .expect("write quick-dev implementer mock");
        let rev_mock = h
            .write_stable_mock_script(
                "amend-quick-dev-rev.sh",
                &quick_dev_injection_reviewer_mock_script(),
            )
            .expect("write quick-dev reviewer mock");
        h.setup_separate_mock_backends(&impl_mock, &rev_mock)
            .expect("setup separate mock backends");

        h.create_project(
            project_id,
            "Quick Dev Amendment Injection",
            "quick dev amendment injection prompt",
        )
        .expect("create project");

        enqueue_external_amendment(
            h,
            project_id,
            "EXT-QD-001",
            "quick dev external amendment body",
        );
        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");
        assert_exit_code(&output, 0);

        let pending =
            pending_amendment_count(&h.project_dir(project_id)).expect("pending amendment count");
        assert_eq!(pending, 0, "queue should be empty after quick-dev drain");
    })
}

fn completion_guard_rejects_with_pending_amendments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "amend-completion-guard";
        h.init_workspace().expect("init workspace");

        let mock = h
            .write_stable_mock_script(
                "amend-completion-guard.sh",
                &completion_guard_pending_mock_script(project_id),
            )
            .expect("write completion guard mock");
        h.setup_mock_backends_stable(&mock)
            .expect("setup mock backends");

        h.create_project(
            project_id,
            "Completion Guard Pending Amendments",
            "completion guard pending amendments prompt",
        )
        .expect("create project");
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");

        let output = h
            .ralph([
                "run",
                "--project",
                project_id,
                "--until-complete",
                "--skip-commit",
            ])
            .expect("ralph run should execute");
        assert_exit_code(&output, 1);
        assert_stderr_contains(
            &output,
            "planner requested completion but 1 amendment(s) are still pending in the queue",
        );

        let pending =
            pending_amendment_count(&h.project_dir(project_id)).expect("pending amendment count");
        assert!(
            pending > 0,
            "completion guard should not drain or mutate pending amendment queue"
        );
    })
}

fn enqueue_external_amendment(h: &RalphHarness, project_id: &str, id: &str, body: &str) {
    enqueue_amendment(
        &h.project_dir(project_id),
        &AmendmentRequest {
            id: id.to_owned(),
            body: body.to_owned(),
            priority: AmendmentPriority::P2,
            source: AmendmentSource::Cli,
            source_detail: Some("validate".to_owned()),
            created_at: Utc::now(),
        },
    )
    .expect("enqueue amendment");
}

fn standard_planner_injection_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  grep -q "## External Amendments" <<< "$INPUT" || { echo "planner prompt missing external amendments heading" >&2; exit 1; }
  grep -q -- "- id: EXT-PLAN-001" <<< "$INPUT" || { echo "planner prompt missing amendment id" >&2; exit 1; }
  grep -q "planner external amendment body" <<< "$INPUT" || { echo "planner prompt missing amendment body" >&2; exit 1; }
  cat <<'EOF'
# Feature: Amendment Prompt Injection

## Description
Validate planner prompt includes externally queued amendments.

## Acceptance Criteria
- [ ] Mock criteria

## Files to Modify/Create
- `mock_file.txt` - mock implementation output

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created mock implementation for planner amendment injection test.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  echo "implemented" > mock_file.txt
  git add mock_file.txt
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock criteria

## Notes
Looks good.

## Commit Message
feat: validate amendment planner injection
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn quick_dev_injection_implementer_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev plan-and-implement phase" <<< "$INPUT"; then
  grep -q "## External Amendments" <<< "$INPUT" || { echo "quick-dev prompt missing external amendments heading" >&2; exit 1; }
  grep -q -- "- id: EXT-QD-001" <<< "$INPUT" || { echo "quick-dev prompt missing amendment id" >&2; exit 1; }
  grep -q "quick dev external amendment body" <<< "$INPUT" || { echo "quick-dev prompt missing amendment body" >&2; exit 1; }
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created quick-dev implementation for amendment injection test.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  echo "quick-dev-implemented" > mock_file.txt
  git add mock_file.txt
elif grep -q "quick-dev apply-fixes phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Applied reviewer-requested fixes.

## Could Not Address
- None
EOF
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No further quick-dev amendments.
EOF
else
  echo "quick-dev implementer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn quick_dev_injection_reviewer_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Review: SATISFIED

Implementation is acceptable.
EOF
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No further quick-dev amendments.
EOF
else
  echo "quick-dev reviewer: unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn completion_guard_pending_mock_script(project_id: &str) -> String {
    let queue_dir = format!(".ralph/projects/{project_id}/amendment-queue");
    format!(
        r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  mkdir -p "{queue_dir}"
  cat > "{queue_dir}/99999999999999-injected-pending.json" <<'EOF'
{{"id":"EXT-PENDING-001","body":"pending amendment injected during planner call","priority":"P2","source":"cli","source_detail":"validate","created_at":"2026-03-09T00:00:00Z"}}
EOF
  cat <<'EOF'
# Project Completion Request

## Rationale
All work is complete.

## Summary of Work
- Completed all required behavior.

## Remaining Items
- None
EOF
else
  echo "completion guard mock received unexpected prompt" >&2
  exit 1
fi
"###
    )
}

/// Verify that `amendments.unify_final_review` defaults to false and the config
/// system can read/write the key at both global and project scopes.
fn unify_config_default_off(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("unify-default", "Unify Default", "test prompt")
            .expect("create project");

        // Default should be false
        let output = h
            .ralph(["config", "get", "amendments.unify_final_review", "--global"])
            .expect("config get global");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "false", "global default should be false");

        // Project scope should also resolve to false
        let output = h
            .ralph([
                "config",
                "get",
                "amendments.unify_final_review",
                "--project",
                "unify-default",
            ])
            .expect("config get project");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.trim(), "false", "project default should be false");

        // Set at project level
        let output = h
            .ralph([
                "config",
                "set",
                "amendments.unify_final_review",
                "true",
                "--project",
                "unify-default",
            ])
            .expect("config set project");
        assert_exit_code(&output, 0);

        // Now project should read true
        let output = h
            .ralph([
                "config",
                "get",
                "amendments.unify_final_review",
                "--project",
                "unify-default",
            ])
            .expect("config get project after set");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            "true",
            "project value should override to true"
        );

        // Global should still be false
        let output = h
            .ralph(["config", "get", "amendments.unify_final_review", "--global"])
            .expect("config get global after project set");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            "false",
            "global should still be false after project-level override"
        );
    })
}

/// Verify that when `unify_final_review` is enabled, drained amendments with
/// `source == final-review` are excluded from the planner's external amendments
/// text (dedupe behavior), while CLI-sourced amendments are still included.
fn unify_planner_dedupe_excludes_final_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "unify-dedupe";
        h.init_workspace().expect("init workspace");

        let mock = h
            .write_stable_mock_script(
                "unify-dedupe-planner.sh",
                &unify_dedupe_planner_mock_script(),
            )
            .expect("write mock");
        h.setup_mock_backends_stable(&mock)
            .expect("setup mock backends");

        h.create_project(
            project_id,
            "Unify Dedupe Test",
            "test prompt for unify dedupe",
        )
        .expect("create project");
        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable qa");
        h.ralph_ok([
            "config",
            "set",
            "amendments.unify_final_review",
            "true",
            "--project",
            project_id,
        ])
        .expect("enable unify_final_review");

        // Enqueue a CLI amendment (should appear in prompt)
        enqueue_external_amendment(h, project_id, "EXT-CLI-001", "cli amendment body");

        // Enqueue a final-review amendment (should be excluded from prompt)
        enqueue_amendment(
            &h.project_dir(project_id),
            &AmendmentRequest {
                id: "FR-MIRROR-001".to_owned(),
                body: "mirrored final review body".to_owned(),
                priority: AmendmentPriority::P2,
                source: AmendmentSource::FinalReview,
                source_detail: Some("claude(opus)".to_owned()),
                created_at: Utc::now(),
            },
        )
        .expect("enqueue final-review amendment");

        let output = h
            .ralph(["run", "--project", project_id, "--loops", "1", "--skip-commit"])
            .expect("ralph run");
        assert_exit_code(&output, 0);

        // Both amendments should be drained (queue empty)
        let pending =
            pending_amendment_count(&h.project_dir(project_id)).expect("pending count");
        assert_eq!(pending, 0, "queue should be empty after drain");
    })
}

/// Verify that when unify is enabled and a final-review round accepts
/// amendments, the orchestrator's `run_final_review_phase` enqueues them
/// as AmendmentRequests with source=final-review via the actual mirroring
/// code path (not just a manual enqueue/drain roundtrip).
fn unify_mirroring_enqueues_final_review_amendments(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "unify-mirror";
        h.init_workspace().expect("init workspace");

        let mock = h
            .write_stable_mock_script(
                "unify-mirror-final-review.sh",
                &unify_mirroring_mock_script(),
            )
            .expect("write mock");
        h.setup_mock_backends_stable(&mock)
            .expect("setup mock backends");

        h.create_project(
            project_id,
            "Unify Mirror Test",
            "test prompt for unify mirroring",
        )
        .expect("create project");

        h.ralph_ok(["config", "set", "workflow.prompt_review_enabled", "false"])
            .expect("disable prompt review");
        h.ralph_ok(["config", "set", "workflow.qa_enabled", "false"])
            .expect("disable qa");
        h.ralph_ok(["config", "set", "workflow.final_review_enabled", "true"])
            .expect("enable final review");
        h.ralph_ok([
            "config",
            "set",
            "workflow.completion_backends",
            "[\"claude\"]",
        ])
        .expect("set completion backends");
        h.ralph_ok(["config", "set", "workflow.completion_min_completers", "1"])
            .expect("set min completers");
        h.ralph_ok([
            "config",
            "set",
            "amendments.unify_final_review",
            "true",
            "--global",
        ])
        .expect("enable unify globally");

        // Run the full orchestration flow. The final reviewer returns
        // AMENDMENTS on its first call (with id=FR-MIRROR-001), triggering
        // the mirroring code path in run_final_review_phase.
        let output = h
            .ralph_with_log(["run", "--project", project_id, "--until-complete"], "info")
            .expect("ralph run");
        assert_exit_code(&output, 0);

        // Verify the orchestrator mirroring code path ran by checking
        // for the info-level log emitted by run_final_review_phase.
        let stderr = strip_ansi(&String::from_utf8_lossy(&output.stderr));
        let mirrored = stderr
            .lines()
            .any(|l| l.contains("mirrored accepted final-review amendment to queue"));
        assert!(
            mirrored,
            "expected orchestrator to log mirroring of accepted final-review amendment"
        );

        // The mirrored amendment should have been drained by the planner
        // in the restart loop, so the queue should be empty.
        let pending =
            pending_amendment_count(&h.project_dir(project_id)).expect("pending count");
        assert_eq!(pending, 0, "queue should be empty after planner drain");
    })
}

/// Mock script for unify mirroring test. Handles all orchestration phases:
/// - Planner: returns feature spec on odd calls, completion request on even
/// - Implementer: creates mock file
/// - Reviewer: approves
/// - Completer: marks complete
/// - Final reviewer: returns AMENDMENTS on first call, NO AMENDMENTS on second
fn unify_mirroring_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

PLANNER_COUNTER=".ralph-planner-counter"
FINAL_REVIEWER_COUNTER=".ralph-final-reviewer-counter"

if grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- none

## Refined Prompt
No changes.
EOF
elif grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  COUNT=0
  if [ -f "$PLANNER_COUNTER" ]; then
    COUNT="$(cat "$PLANNER_COUNTER")"
  fi
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$PLANNER_COUNTER"
  if [ "$COUNT" -eq 1 ] || [ "$COUNT" -eq 3 ]; then
    cat <<'EOF'
# Feature: Unify Mirror Validation

## Description
Validate that accepted final-review amendments are mirrored to the queue.

## Acceptance Criteria
- [ ] Mock implementation file exists

## Files to Modify/Create
- `mirror_test.txt` - mock output

## Dependencies
- Requires: none
- Blocks: none
EOF
  else
    cat <<'EOF'
# Project Completion Request

## Rationale
All required behavior is complete.

## Summary of Work
- Prior loops implemented and reviewed successfully.

## Remaining Items
- None
EOF
  fi
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Mock implementation for unify mirroring test.

## Spec Deviations
- None

## Testing
- Mock only
EOF
  echo "mirrored" > mirror_test.txt
  git add mirror_test.txt
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file exists

## Notes
Looks good.

## Commit Message
feat: validate unify mirroring
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  cat <<'EOF'
# Verdict: COMPLETE

The project satisfies all requirements:
- Mock requirement: satisfied
EOF
elif grep -q "You are a final reviewer auditing a completed project for correctness, safety, and robustness." <<< "$INPUT"; then
  COUNT=0
  if [ -f "$FINAL_REVIEWER_COUNTER" ]; then
    COUNT="$(cat "$FINAL_REVIEWER_COUNTER")"
  fi
  COUNT=$((COUNT + 1))
  echo "$COUNT" > "$FINAL_REVIEWER_COUNTER"
  if [ "$COUNT" -le 1 ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Amendment: FR-MIRROR-001

### Problem
mirror_test.txt needs a header comment.

### Proposed Change
Add a header comment to mirror_test.txt.

### Affected Files
- `mirror_test.txt`
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
No amendments required.
EOF
  fi
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn unify_dedupe_planner_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  # CLI amendment should be present
  grep -q -- "- id: EXT-CLI-001" <<< "$INPUT" || { echo "planner prompt missing CLI amendment id" >&2; exit 1; }
  grep -q "cli amendment body" <<< "$INPUT" || { echo "planner prompt missing CLI amendment body" >&2; exit 1; }
  # Final-review amendment should NOT be in External Amendments (dedupe)
  if grep -q -- "- id: FR-MIRROR-001" <<< "$INPUT"; then
    echo "planner prompt should NOT contain final-review amendment id when unify dedupe is active" >&2
    exit 1
  fi
  cat <<'EOF'
# Feature: Unify Dedupe Validation

## Description
Validate planner prompt excludes final-review source amendments when unify is enabled.

## Acceptance Criteria
- [ ] Mock criteria

## Files to Modify/Create
- `mock_file.txt` - mock output

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Mock implementation for unify dedupe test.

## Spec Deviations
- None

## Testing
- Mock only
EOF
  echo "dedupe-test" > mock_file.txt
  git add mock_file.txt
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock criteria

## Notes
Looks good.

## Commit Message
feat: validate unify dedupe
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
