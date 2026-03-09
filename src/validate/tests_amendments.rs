use super::*;

use std::fs;

use crate::validate::assertions::{assert_exit_code, assert_file_exists};
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

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
