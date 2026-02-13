use super::*;

use crate::validate::assertions::{
    assert_dir_exists, assert_file_exists, assert_file_not_empty, assert_json_array_len,
    assert_json_field, assert_toml_field, load_toml,
};
use crate::validate::harness::RalphHarness;
use serde_json::json;
use toml::Value as TomlValue;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "init::creates_workspace_structure",
            func: creates_workspace_structure,
        },
        ConformanceTest {
            name: "init::creates_template_files",
            func: creates_template_files,
        },
        ConformanceTest {
            name: "init::default_config",
            func: default_config,
        },
        ConformanceTest {
            name: "init::default_index",
            func: default_index,
        },
        ConformanceTest {
            name: "init::rejects_nonempty_dir",
            func: rejects_nonempty_dir,
        },
    ]
}

fn creates_workspace_structure(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let ralph_dir = h.repo_root.join(".ralph");
        assert_dir_exists(&ralph_dir);
        assert_file_exists(&ralph_dir.join("ralph.toml"));
        assert_file_exists(&ralph_dir.join("index.json"));
        assert_dir_exists(&ralph_dir.join("projects"));
        assert_dir_exists(&ralph_dir.join("templates"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn creates_template_files(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let templates = h.repo_root.join(".ralph").join("templates");
        for name in &[
            "spec.md",
            "implementation.md",
            "review.md",
            "prompt_reviewer.md",
            "completion.md",
            "qa.md",
        ] {
            assert_file_not_empty(&templates.join(name));
        }
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn default_config(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let config = load_toml(&toml_path);

        // Workspace section
        assert_toml_field(
            &config,
            "workspace.version",
            &TomlValue::String("1.0".to_owned()),
        );
        assert_toml_field(
            &config,
            "workspace.default_backend",
            &TomlValue::String("claude".to_owned()),
        );

        // Backend commands exist
        assert_toml_field(
            &config,
            "backends.claude.command",
            &TomlValue::String("claude".to_owned()),
        );
        assert_toml_field(
            &config,
            "backends.codex.command",
            &TomlValue::String("codex".to_owned()),
        );

        // Template paths
        assert_toml_field(
            &config,
            "templates.planner",
            &TomlValue::String("templates/spec.md".to_owned()),
        );
        assert_toml_field(
            &config,
            "templates.implementer",
            &TomlValue::String("templates/implementation.md".to_owned()),
        );
        assert_toml_field(
            &config,
            "templates.reviewer",
            &TomlValue::String("templates/review.md".to_owned()),
        );
        assert_toml_field(
            &config,
            "templates.completer",
            &TomlValue::String("templates/completion.md".to_owned()),
        );
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn default_index(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let index = h.load_index().expect("failed to load index.json");

        assert_json_field(&index, "workspace_version", &json!("1.0"));
        assert_json_array_len(&index, "projects", 0);
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn rejects_nonempty_dir(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("first ralph init should succeed");

        // Second init should fail with exit code 2
        h.ralph_exit(["init"], 2)
            .expect("ralph command should execute");
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
