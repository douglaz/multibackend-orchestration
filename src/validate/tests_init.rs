use super::*;

use crate::validate::assertions::{
    assert_dir_exists, assert_exit_code, assert_file_exists, assert_file_not_empty,
    assert_path_not_exists, assert_stdout_eq, assert_toml_field, load_toml,
};
use crate::validate::harness::RalphHarness;
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
            name: "init::no_index_json",
            func: no_index_json,
        },
        ConformanceTest {
            name: "init::rejects_nonempty_dir",
            func: rejects_nonempty_dir,
        },
        ConformanceTest {
            name: "init::dry_run_prints_actions",
            func: dry_run_prints_actions,
        },
        ConformanceTest {
            name: "init::dry_run_short_flag",
            func: dry_run_short_flag,
        },
        ConformanceTest {
            name: "init::dry_run_rejects_nonempty_dir",
            func: dry_run_rejects_nonempty_dir,
        },
        ConformanceTest {
            name: "init::dry_run_rejects_file_target",
            func: dry_run_rejects_file_target,
        },
        ConformanceTest {
            name: "init::dry_run_rejects_unreadable_target",
            func: dry_run_rejects_unreadable_target,
        },
    ]
}

fn creates_workspace_structure(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let ralph_dir = h.repo_root.join(".ralph");
        assert_dir_exists(&ralph_dir);
        assert_file_exists(&ralph_dir.join("ralph.toml"));
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
            "prompt_review_validator.md",
            "completion.md",
            "qa.md",
            "final_reviewer.md",
            "planner_position.md",
            "vote.md",
            "arbiter.md",
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

fn no_index_json(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");
        h.assert_no_index_json();
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

fn dry_run_prints_actions(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let output = h
            .ralph(["init", "--dry-run"])
            .expect("ralph init --dry-run should execute");
        assert_exit_code(&output, 0);

        let expected = r#"
dry-run: create-dir .ralph/projects
dry-run: create-dir .ralph/templates
dry-run: write-config .ralph/ralph.toml
dry-run: write-template .ralph/templates/spec.md
dry-run: write-template .ralph/templates/implementation.md
dry-run: write-template .ralph/templates/review.md
dry-run: write-template .ralph/templates/prompt_reviewer.md
dry-run: write-template .ralph/templates/prompt_review_validator.md
dry-run: write-template .ralph/templates/completion.md
dry-run: write-template .ralph/templates/qa.md
dry-run: write-template .ralph/templates/final_reviewer.md
dry-run: write-template .ralph/templates/planner_position.md
dry-run: write-template .ralph/templates/vote.md
dry-run: write-template .ralph/templates/arbiter.md
"#;
        assert_stdout_eq(&output, expected);

        assert_path_not_exists(&h.repo_root.join(".ralph"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn dry_run_short_flag(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let long_output = h
            .ralph(["init", "--dry-run"])
            .expect("ralph init --dry-run should execute");
        let short_output = h
            .ralph(["init", "-n"])
            .expect("ralph init -n should execute");
        assert_exit_code(&long_output, 0);
        assert_exit_code(&short_output, 0);

        assert_eq!(long_output.stdout, short_output.stdout);
        assert_eq!(long_output.stderr, short_output.stderr);
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn dry_run_rejects_nonempty_dir(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("first ralph init should succeed");

        let output = h
            .ralph(["init", "--dry-run"])
            .expect("ralph init --dry-run should execute");
        assert_exit_code(&output, 2);
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn dry_run_rejects_file_target(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let target = h.repo_root.join("file-target");
        std::fs::write(&target, "not-a-directory").expect("write target file");

        let output = h
            .ralph(vec![
                "init".to_owned(),
                "--dir".to_owned(),
                target.to_string_lossy().into_owned(),
                "--dry-run".to_owned(),
            ])
            .expect("ralph init --dry-run should execute");
        assert_exit_code(&output, 1);
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn dry_run_rejects_unreadable_target(h: &RalphHarness) -> TestResult {
    #[cfg(not(unix))]
    {
        let _ = h;
        TestResult::Pass
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let target = h.repo_root.join("unreadable-target");
            std::fs::create_dir_all(&target).expect("create unreadable target");

            let original_mode = std::fs::metadata(&target)
                .expect("stat target")
                .permissions()
                .mode();
            let mut locked = std::fs::metadata(&target)
                .expect("stat target")
                .permissions();
            locked.set_mode(0o000);
            std::fs::set_permissions(&target, locked).expect("set unreadable permissions");

            let unreadable = std::fs::read_dir(&target).is_err();

            let restore = || {
                let mut perms = std::fs::metadata(&target)
                    .expect("stat target for permission restore")
                    .permissions();
                perms.set_mode(original_mode);
                std::fs::set_permissions(&target, perms).expect("restore permissions");
            };

            if !unreadable {
                restore();
                return;
            }

            let output = h
                .ralph(vec![
                    "init".to_owned(),
                    "--dir".to_owned(),
                    target.to_string_lossy().into_owned(),
                    "--dry-run".to_owned(),
                ])
                .expect("ralph init --dry-run should execute");
            restore();

            assert_exit_code(&output, 1);
        })) {
            Ok(()) => TestResult::Pass,
            Err(e) => TestResult::Fail(panic_message(e)),
        }
    }
}
