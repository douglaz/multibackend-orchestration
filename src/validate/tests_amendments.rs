use super::*;

use chrono::Utc;
use std::fs;

use crate::project::amendments::{
    enqueue_amendment, pending_amendment_count, AmendmentPriority, AmendmentRequest,
    AmendmentSource,
};
use crate::validate::assertions::{
    assert_exit_code, assert_file_exists, assert_path_not_exists, assert_stderr_contains,
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

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
