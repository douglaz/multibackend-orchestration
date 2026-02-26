use super::*;

use crate::config::GlobalConfig;
use crate::validate::assertions::{
    assert_dir_exists, assert_exit_code, assert_file_exists, assert_file_not_empty,
    assert_path_not_exists, assert_stdout_eq, assert_toml_field, load_toml,
};
use crate::validate::harness::RalphHarness;
use toml::Value as TomlValue;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "init::creates_minimal_workspace_structure",
            func: creates_minimal_workspace_structure,
        },
        ConformanceTest {
            name: "init::default_minimal_config_equivalent_to_defaults",
            func: default_minimal_config_equivalent_to_defaults,
        },
        ConformanceTest {
            name: "init::copy_files_creates_template_files",
            func: copy_files_creates_template_files,
        },
        ConformanceTest {
            name: "init::copy_files_writes_full_config",
            func: copy_files_writes_full_config,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_existing_workspace",
            func: copy_files_overlay_existing_workspace,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_rejects_non_workspace_nonempty_dir",
            func: copy_files_overlay_rejects_non_workspace_nonempty_dir,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_invalid_config_fails_without_partial_writes",
            func: copy_files_overlay_invalid_config_fails_without_partial_writes,
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
            name: "init::dry_run_prints_minimal_actions",
            func: dry_run_prints_minimal_actions,
        },
        ConformanceTest {
            name: "init::dry_run_copy_files_prints_full_actions",
            func: dry_run_copy_files_prints_full_actions,
        },
        ConformanceTest {
            name: "init::dry_run_copy_files_overlay_prints_merge_and_skip_existing",
            func: dry_run_copy_files_overlay_prints_merge_and_skip_existing,
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

fn creates_minimal_workspace_structure(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("ralph init should succeed");

        let ralph_dir = h.repo_root.join(".ralph");
        assert_dir_exists(&ralph_dir);
        assert_file_exists(&ralph_dir.join("ralph.toml"));
        assert_dir_exists(&ralph_dir.join("projects"));
        assert!(
            !ralph_dir.join("templates").exists(),
            "minimal init should not create templates/"
        );
    })
}

fn default_minimal_config_equivalent_to_defaults(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("ralph init should succeed");

        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let raw = std::fs::read_to_string(&toml_path).expect("read minimal config");
        let parsed: GlobalConfig = toml::from_str(&raw).expect("parse minimal config");
        assert_eq!(parsed, GlobalConfig::default());

        let config = load_toml(&toml_path);
        assert_toml_field(
            &config,
            "workspace.version",
            &TomlValue::String(GlobalConfig::default().workspace.version),
        );
    })
}

fn copy_files_creates_template_files(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.ralph_ok(["init", "--copy-files"])
            .expect("ralph init --copy-files should succeed");

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
    })
}

fn copy_files_writes_full_config(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.ralph_ok(["init", "--copy-files"])
            .expect("ralph init --copy-files should succeed");

        let config = load_toml(&h.repo_root.join(".ralph").join("ralph.toml"));
        assert_toml_field(
            &config,
            "workspace.default_backend",
            &TomlValue::String("claude".to_owned()),
        );
        assert_toml_field(
            &config,
            "backends.claude.command",
            &TomlValue::String("claude".to_owned()),
        );
        assert_toml_field(
            &config,
            "templates.planner",
            &TomlValue::String("templates/spec.md".to_owned()),
        );
    })
}

fn copy_files_overlay_existing_workspace(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("minimal init should succeed");

        let workspace_root = h.repo_root.join(".ralph");
        std::fs::write(
            workspace_root.join("ralph.toml"),
            r#"[workspace]
version = "1.0"
default_backend = "codex"
"#,
        )
        .expect("write overlay config");

        let templates_dir = workspace_root.join("templates");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(templates_dir.join("spec.md"), "custom spec template")
            .expect("write custom spec template");

        h.ralph_ok(["init", "--copy-files"])
            .expect("overlay init --copy-files should succeed");

        let config = load_toml(&workspace_root.join("ralph.toml"));
        assert_toml_field(
            &config,
            "workspace.default_backend",
            &TomlValue::String("codex".to_owned()),
        );
        assert_toml_field(
            &config,
            "backends.codex.command",
            &TomlValue::String("codex".to_owned()),
        );

        let custom_spec =
            std::fs::read_to_string(templates_dir.join("spec.md")).expect("read custom spec");
        assert_eq!(custom_spec, "custom spec template");
        assert_file_not_empty(&templates_dir.join("implementation.md"));
    })
}

fn copy_files_overlay_rejects_non_workspace_nonempty_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let workspace_root = h.repo_root.join(".ralph");
        std::fs::create_dir_all(&workspace_root).expect("create .ralph");
        std::fs::write(workspace_root.join("existing.txt"), "x").expect("write sentinel");

        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 2);
    })
}

fn copy_files_overlay_invalid_config_fails_without_partial_writes(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("minimal init should succeed");
        let workspace_root = h.repo_root.join(".ralph");
        std::fs::write(workspace_root.join("ralph.toml"), "not = [valid")
            .expect("write invalid config");

        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 1);
        assert!(
            !workspace_root.join("templates").exists(),
            "invalid overlay config should fail before writing templates"
        );
    })
}

fn no_index_json(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("ralph init should succeed");
        h.assert_no_index_json();
    })
}

fn rejects_nonempty_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("first ralph init should succeed");
        h.ralph_exit(["init"], 2)
            .expect("second ralph init should fail");
    })
}

fn dry_run_prints_minimal_actions(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let output = h
            .ralph(["init", "--dry-run"])
            .expect("ralph init --dry-run should execute");
        assert_exit_code(&output, 0);

        let expected = r#"
dry-run: create-dir .ralph/projects
dry-run: write-config .ralph/ralph.toml
"#;
        assert_stdout_eq(&output, expected);
        assert_path_not_exists(&h.repo_root.join(".ralph"));
    })
}

fn dry_run_copy_files_prints_full_actions(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let output = h
            .ralph(["init", "--copy-files", "--dry-run"])
            .expect("ralph init --copy-files --dry-run should execute");
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
    })
}

fn dry_run_copy_files_overlay_prints_merge_and_skip_existing(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("minimal init should succeed");
        let workspace_root = h.repo_root.join(".ralph");
        let templates_dir = workspace_root.join("templates");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(templates_dir.join("spec.md"), "custom spec").expect("write spec");

        let output = h
            .ralph(["init", "--copy-files", "--dry-run"])
            .expect("ralph init --copy-files --dry-run should execute");
        assert_exit_code(&output, 0);

        let expected = r#"
dry-run: create-dir .ralph/projects
dry-run: create-dir .ralph/templates
dry-run: merge-config .ralph/ralph.toml
dry-run: skip-existing .ralph/templates/spec.md
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
    })
}

fn dry_run_short_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
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
    })
}

fn dry_run_rejects_nonempty_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("first ralph init should succeed");
        let output = h
            .ralph(["init", "--dry-run"])
            .expect("ralph init --dry-run should execute");
        assert_exit_code(&output, 2);
    })
}

fn dry_run_rejects_file_target(h: &RalphHarness) -> TestResult {
    run_case(|| {
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
    })
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

        run_case(|| {
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
        })
    }
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
