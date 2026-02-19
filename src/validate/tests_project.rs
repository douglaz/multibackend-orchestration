use super::*;

use crate::validate::assertions::{
    assert_file_exists, assert_git_branch_exists, assert_json_field, assert_path_not_exists,
    assert_stderr_contains, assert_stdout_contains,
};
use crate::validate::harness::RalphHarness;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "project::new_creates_state",
            func: new_creates_state,
        },
        ConformanceTest {
            name: "project::new_copies_prompt",
            func: new_copies_prompt,
        },
        ConformanceTest {
            name: "project::new_activates_project",
            func: new_activates_project,
        },
        ConformanceTest {
            name: "project::new_creates_branch",
            func: new_creates_branch,
        },
        ConformanceTest {
            name: "project::new_rejects_duplicate",
            func: new_rejects_duplicate,
        },
        ConformanceTest {
            name: "project::list_shows_project",
            func: list_shows_project,
        },
        ConformanceTest {
            name: "project::use_switches_active",
            func: use_switches_active,
        },
        ConformanceTest {
            name: "project::delete_removes_directory",
            func: delete_removes_directory,
        },
        ConformanceTest {
            name: "project::delete_refuses_active",
            func: delete_refuses_active,
        },
        ConformanceTest {
            name: "project::delete_nonexistent_fails",
            func: delete_nonexistent_fails,
        },
        ConformanceTest {
            name: "project::delete_no_active_project",
            func: delete_no_active_project,
        },
        ConformanceTest {
            name: "project::show_displays_info",
            func: show_displays_info,
        },
        ConformanceTest {
            name: "project::show_json",
            func: show_json,
        },
        ConformanceTest {
            name: "project::no_index_json_after_create",
            func: no_index_json_after_create,
        },
        ConformanceTest {
            name: "project::migration_from_legacy_index",
            func: migration_from_legacy_index,
        },
        ConformanceTest {
            name: "project::stale_active_project",
            func: stale_active_project,
        },
        ConformanceTest {
            name: "project::corrupt_active_project",
            func: corrupt_active_project,
        },
    ]
}

fn new_creates_state(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("test-proj", "Test Project", "Test prompt content")
            .expect("create_project failed");

        let state = h.load_state("test-proj").expect("load_state failed");

        assert_json_field(&state, "current_loop", &json!(0));
        assert_json_field(&state, "current_phase", &json!("planning"));
        assert_json_field(&state, "status", &json!("pending"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn new_copies_prompt(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");

        let prompt_content = "This is the project prompt for conformance testing.";
        h.create_project("copy-test", "Copy Test", prompt_content)
            .expect("create_project failed");

        let prompt_path = h
            .repo_root
            .join(".ralph")
            .join("projects")
            .join("copy-test")
            .join("prompt.md");
        assert_file_exists(&prompt_path);

        let actual = std::fs::read_to_string(&prompt_path).expect("failed to read prompt.md");
        assert_eq!(
            actual.trim(),
            prompt_content.trim(),
            "prompt.md content mismatch"
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn new_activates_project(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("act-test", "Active Test", "prompt")
            .expect("create_project failed");

        // First project should be auto-activated via worktree-local storage
        let active = h.load_active_project().expect("load_active_project failed");
        assert_eq!(
            active.as_deref(),
            Some("act-test"),
            "first project should be auto-activated"
        );

        // Derived state should contain created_at
        let state = h.load_state("act-test").expect("load_state failed");
        assert!(
            state.get("created_at").is_some(),
            "derived state should contain created_at"
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn new_creates_branch(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("branch-test", "Branch Test", "prompt")
            .expect("create_project failed");

        assert_git_branch_exists(&h.repo_root, "ralph/branch-test");
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn new_rejects_duplicate(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("dup-test", "First", "prompt")
            .expect("first create_project failed");

        // Write a second prompt file and attempt duplicate creation
        let prompt_path = h.temp_dir.path().join("dup-prompt.md");
        std::fs::write(&prompt_path, "duplicate prompt").expect("write prompt");

        let prompt_str = prompt_path.to_string_lossy().into_owned();
        h.ralph_exit(
            vec![
                "project".to_owned(),
                "new".to_owned(),
                "--id".to_owned(),
                "dup-test".to_owned(),
                "--name".to_owned(),
                "Second".to_owned(),
                "--prompt".to_owned(),
                prompt_str,
            ],
            2,
        )
        .expect("ralph command should execute");
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn list_shows_project(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("list-test", "List Test Project", "prompt")
            .expect("create_project failed");

        let output = h
            .ralph(["project", "list"])
            .expect("ralph project list failed");
        assert_stdout_contains(&output, "list-test");
        assert_stdout_contains(&output, "List Test Project");
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn use_switches_active(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("proj-a", "Project A", "prompt a")
            .expect("create first project failed");
        h.create_project("proj-b", "Project B", "prompt b")
            .expect("create second project failed");

        // After creating two projects, the first should be active
        let active = h.load_active_project().expect("load_active_project failed");
        assert_eq!(
            active.as_deref(),
            Some("proj-a"),
            "first project should be active"
        );

        // Switch to proj-b
        h.ralph_ok(["project", "use", "proj-b"])
            .expect("ralph project use failed");

        let active = h
            .load_active_project()
            .expect("load_active_project after use failed");
        assert_eq!(
            active.as_deref(),
            Some("proj-b"),
            "active project should be proj-b after use"
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn delete_removes_directory(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("del-test", "Delete Test", "prompt")
            .expect("create first project failed");
        h.create_project("keep-test", "Keep Test", "prompt")
            .expect("create second project failed");
        h.ralph_ok(["project", "use", "keep-test"])
            .expect("project use should succeed");

        let stdout = h
            .ralph_ok(["project", "delete", "del-test"])
            .expect("project delete should succeed");

        assert!(
            stdout.contains("project 'del-test' deleted"),
            "expected delete confirmation, got:\n{}",
            stdout
        );
        assert_path_not_exists(&h.project_dir("del-test"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn delete_refuses_active(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("active-del", "Active Delete", "prompt")
            .expect("create project failed");

        let output = h
            .ralph_exit(["project", "delete", "active-del"], 2)
            .expect("project delete should execute");
        assert_stderr_contains(&output, "cannot delete the active project 'active-del'");
        assert_file_exists(&h.project_dir("active-del").join("prompt.md"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn delete_nonexistent_fails(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        let output = h
            .ralph_exit(["project", "delete", "no-such-proj"], 2)
            .expect("project delete should execute");
        assert_stderr_contains(&output, "project not found: no-such-proj");
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn delete_no_active_project(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("orphan-del", "Orphan Delete", "prompt")
            .expect("create project failed");

        let active_path = h.repo_root.join(".git").join("ralph-active-project");
        std::fs::write(&active_path, "\n").expect("clear active-project file");
        let active = h
            .load_active_project()
            .expect("load_active_project should succeed");
        assert_eq!(active, None, "active project should be cleared");

        let stdout = h
            .ralph_ok(["project", "delete", "orphan-del"])
            .expect("project delete should succeed");
        assert!(
            stdout.contains("project 'orphan-del' deleted"),
            "expected delete confirmation, got:\n{}",
            stdout
        );
        assert_path_not_exists(&h.project_dir("orphan-del"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn show_displays_info(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("show-test", "Show Test", "prompt")
            .expect("create_project failed");

        let output = h
            .ralph(["project", "show"])
            .expect("ralph project show failed");

        assert_stdout_contains(&output, "show-test");
        assert_stdout_contains(&output, "pending");
        assert_stdout_contains(&output, "planning");
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn show_json(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("json-test", "JSON Test", "prompt")
            .expect("create_project failed");

        let stdout = h
            .ralph_ok(["project", "show", "--json"])
            .expect("ralph project show --json failed");

        // Must be valid JSON
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("output should be valid JSON");

        // Should contain project and state sections with correct fields
        assert_json_field(&parsed, "project.id", &json!("json-test"));
        assert_json_field(&parsed, "state.current_loop", &json!(0));
        assert_json_field(&parsed, "state.current_phase", &json!("planning"));
        assert_json_field(&parsed, "state.status", &json!("pending"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn no_index_json_after_create(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("no-idx", "No Index", "prompt")
            .expect("create_project failed");

        // index.json should not exist after init + project creation
        h.assert_no_index_json();
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

/// Test one-time migration from legacy `index.json` active_project.
fn migration_from_legacy_index(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("mig-test", "Migration Test", "prompt")
            .expect("create_project failed");

        // Remove the worktree-local active-project file to simulate
        // a pre-migration state.
        let active_path = h.repo_root.join(".git").join("ralph-active-project");
        if active_path.exists() {
            std::fs::remove_file(&active_path).expect("remove active-project file");
        }

        // Write a legacy index.json with active_project pointing at mig-test
        let index_path = h.repo_root.join(".ralph").join("index.json");
        let legacy_index = json!({
            "workspace_version": "1.0",
            "active_project": "mig-test",
            "projects": []
        });
        std::fs::write(
            &index_path,
            serde_json::to_string_pretty(&legacy_index).unwrap(),
        )
        .expect("write legacy index.json");

        // Now run any command that triggers Workspace::load (e.g., project list)
        let output = h.ralph(["project", "list"]).expect("ralph project list");
        assert_stderr_contains(&output, "migrated active project");

        // The worktree-local active-project file should now be set
        let active = h.load_active_project().expect("load_active_project failed");
        assert_eq!(
            active.as_deref(),
            Some("mig-test"),
            "migration should seed active project from index.json"
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

/// Test that a stale active-project (pointing to a deleted project) produces
/// a descriptive error with a hint to run `ralph project use <id>`.
fn stale_active_project(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("stale-test", "Stale Test", "prompt")
            .expect("create_project failed");

        // Delete the project directory to make the active project stale
        let project_dir = h
            .repo_root
            .join(".ralph")
            .join("projects")
            .join("stale-test");
        std::fs::remove_dir_all(&project_dir).expect("remove project dir");

        // Running status without --project should fail with a hint
        let output = h.ralph(["status"]).expect("ralph status");
        assert!(
            !output.status.success(),
            "ralph status should fail with stale active project"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("no longer exists") || stderr.contains("ralph project use"),
            "error should mention that the active project no longer exists and hint at `ralph project use`; got: {}",
            stderr
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

/// Test that a corrupt active-project file (invalid characters) is treated
/// as no active project.
fn corrupt_active_project(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");

        // Write invalid content to the active-project file
        let active_path = h.repo_root.join(".git").join("ralph-active-project");
        std::fs::write(&active_path, "invalid project id!@#\n")
            .expect("write corrupt active-project");

        // Running status should fail with ActiveProjectNotSet, not a crash
        let output = h.ralph(["status"]).expect("ralph status");
        assert!(
            !output.status.success(),
            "ralph status should fail with corrupt active project"
        );

        // Should be treated as "no active project" (exit code 2)
        assert_eq!(
            output.status.code(),
            Some(2),
            "corrupt active project should result in exit code 2"
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
