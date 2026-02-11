use super::*;

use crate::validate::assertions::{
    assert_file_exists, assert_git_branch_exists, assert_json_field, assert_stdout_contains,
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
            name: "project::new_updates_index",
            func: new_updates_index,
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
            name: "project::show_displays_info",
            func: show_displays_info,
        },
        ConformanceTest {
            name: "project::show_json",
            func: show_json,
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

fn new_updates_index(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("init failed");
        h.create_project("idx-test", "Index Test", "prompt")
            .expect("create_project failed");

        let index = h.load_index().expect("load_index failed");

        // active_project should be set (first project becomes active)
        assert_json_field(&index, "active_project", &json!("idx-test"));

        // projects array should contain the new project
        let projects = index["projects"]
            .as_array()
            .expect("projects should be an array");
        assert_eq!(projects.len(), 1, "expected exactly 1 project entry");
        assert_json_field(&projects[0], "id", &json!("idx-test"));
        assert_json_field(&projects[0], "name", &json!("Index Test"));
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
        let index = h.load_index().expect("load_index failed");
        assert_json_field(&index, "active_project", &json!("proj-a"));

        // Switch to proj-b
        h.ralph_ok(["project", "use", "proj-b"])
            .expect("ralph project use failed");

        let index = h.load_index().expect("load_index after use failed");
        assert_json_field(&index, "active_project", &json!("proj-b"));
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
