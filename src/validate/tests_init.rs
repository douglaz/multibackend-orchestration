use super::*;

use toml_edit::DocumentMut;

use crate::validate::assertions::{
    assert_dir_exists, assert_exit_code, assert_file_exists,
    assert_path_not_exists, assert_stdout_eq,
};
use crate::validate::harness::RalphHarness;
use crate::config::GlobalConfig;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "init::creates_workspace_structure",
            func: creates_workspace_structure,
        },
        ConformanceTest {
            name: "init::creates_minimal_config",
            func: creates_minimal_config,
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
        ConformanceTest {
            name: "init::copy_files_full_scaffold_on_new_target",
            func: copy_files_full_scaffold_on_new_target,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_preserves_custom_values",
            func: copy_files_overlay_preserves_custom_values,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_fills_missing_keys",
            func: copy_files_overlay_fills_missing_keys,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_creates_missing_templates_only",
            func: copy_files_overlay_creates_missing_templates_only,
        },
        ConformanceTest {
            name: "init::copy_files_rejects_non_workspace_nonempty_dir",
            func: copy_files_rejects_non_workspace_nonempty_dir,
        },
        ConformanceTest {
            name: "init::copy_files_rejects_malformed_toml",
            func: copy_files_rejects_malformed_toml,
        },
        ConformanceTest {
            name: "init::copy_files_dry_run_full_scaffold",
            func: copy_files_dry_run_full_scaffold,
        },
        ConformanceTest {
            name: "init::copy_files_dry_run_overlay",
            func: copy_files_dry_run_overlay,
        },
        ConformanceTest {
            name: "init::copy_files_overlay_inline_table_merge",
            func: copy_files_overlay_inline_table_merge,
        },
    ]
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

fn creates_workspace_structure(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("ralph init should succeed");

        let ralph_dir = h.repo_root.join(".ralph");
        assert_dir_exists(&ralph_dir);
        assert_file_exists(&ralph_dir.join("ralph.toml"));
        assert_dir_exists(&ralph_dir.join("projects"));
    })
}

fn creates_minimal_config(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("ralph init should succeed");

        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let raw_toml = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        let parsed: GlobalConfig = toml::from_str(&raw_toml).expect("ralph.toml should parse");
        assert_eq!(parsed, GlobalConfig::default());
        assert!(!h.repo_root.join(".ralph").join("templates").exists());
    })
}

fn default_config(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("ralph init should succeed");

        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let config = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        let toml = crate::validate::assertions::load_toml(&toml_path);

        let workspace_table = toml
            .get("workspace")
            .expect("workspace section should exist")
            .as_table()
            .expect("workspace section should be a table");
        assert!(workspace_table.is_empty());

        let parsed: GlobalConfig = toml::from_str(&config).expect("parse minimal toml");
        assert_eq!(parsed, GlobalConfig::default());
        assert_eq!(toml.get("backends"), None);
        assert_eq!(toml.get("templates"), None);
        assert_eq!(toml.get("git"), None);
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

        // Second init should fail with exit code 2
        h.ralph_exit(["init"], 2)
            .expect("ralph command should execute");
    })
}

fn dry_run_prints_actions(h: &RalphHarness) -> TestResult {
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

// --- copy-files conformance tests ---

fn copy_files_full_scaffold_on_new_target(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 0);

        let ralph_dir = h.repo_root.join(".ralph");
        assert_dir_exists(&ralph_dir.join("projects"));
        assert_dir_exists(&ralph_dir.join("templates"));
        assert_file_exists(&ralph_dir.join("ralph.toml"));

        // Verify full config is written (not minimal).
        let config_str =
            std::fs::read_to_string(ralph_dir.join("ralph.toml")).expect("read ralph.toml");
        let parsed: GlobalConfig =
            toml::from_str(&config_str).expect("ralph.toml should parse");
        assert_eq!(parsed, GlobalConfig::default());

        // Verify all 11 template files exist.
        let templates_dir = ralph_dir.join("templates");
        for (name, _) in crate::cli::init::TEMPLATE_FILES {
            assert_file_exists(&templates_dir.join(name));
        }
    })
}

fn copy_files_overlay_preserves_custom_values(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // First, create a minimal workspace.
        h.init_workspace().expect("init should succeed");

        let ralph_dir = h.repo_root.join(".ralph");
        let toml_path = ralph_dir.join("ralph.toml");

        // Write a config with a custom value.
        std::fs::write(
            &toml_path,
            "[workspace]\ndefault_backend = \"codex\"\n",
        )
        .expect("write custom config");

        // Run --copy-files overlay.
        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 0);

        // Verify the custom value is preserved.
        let merged_str =
            std::fs::read_to_string(&toml_path).expect("read merged ralph.toml");
        let parsed: GlobalConfig =
            toml::from_str(&merged_str).expect("merged config should parse");
        assert_eq!(
            parsed.workspace.default_backend, "codex",
            "custom default_backend should be preserved"
        );
    })
}

fn copy_files_overlay_fills_missing_keys(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a minimal workspace.
        h.init_workspace().expect("init should succeed");

        // Run --copy-files overlay.
        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 0);

        let ralph_dir = h.repo_root.join(".ralph");
        let toml_path = ralph_dir.join("ralph.toml");
        let merged_str =
            std::fs::read_to_string(&toml_path).expect("read merged ralph.toml");
        let parsed: GlobalConfig =
            toml::from_str(&merged_str).expect("merged config should parse");

        // Effective config should match full defaults.
        assert_eq!(parsed, GlobalConfig::default());

        // Verify file-level key insertion (not just deserialized equality).
        let doc: toml_edit::DocumentMut = merged_str.parse().expect("parse as doc");
        let ws = doc
            .get("workspace")
            .expect("workspace key should exist")
            .as_table()
            .expect("workspace should be a table");
        assert!(
            ws.contains_key("version"),
            "version should be physically present in merged TOML file content"
        );
        assert!(
            ws.contains_key("default_backend"),
            "default_backend should be physically present in merged TOML file content"
        );
        assert!(
            doc.get("backends").is_some(),
            "backends section should be physically present in merged TOML"
        );

        // Templates should be created.
        let templates_dir = ralph_dir.join("templates");
        assert_dir_exists(&templates_dir);
        for (name, _) in crate::cli::init::TEMPLATE_FILES {
            assert_file_exists(&templates_dir.join(name));
        }
    })
}

fn copy_files_overlay_creates_missing_templates_only(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a workspace with one custom template already present.
        h.init_workspace().expect("init should succeed");
        let ralph_dir = h.repo_root.join(".ralph");
        let templates_dir = ralph_dir.join("templates");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(templates_dir.join("spec.md"), "my custom spec template")
            .expect("write custom spec.md");

        // Run --copy-files overlay.
        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 0);

        // Custom spec.md should be unchanged.
        let spec_content =
            std::fs::read_to_string(templates_dir.join("spec.md")).expect("read spec.md");
        assert_eq!(spec_content, "my custom spec template");

        // Other templates should now exist.
        for (name, _) in crate::cli::init::TEMPLATE_FILES {
            assert_file_exists(&templates_dir.join(name));
        }
    })
}

fn copy_files_rejects_non_workspace_nonempty_dir(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a non-empty directory without ralph.toml.
        let ralph_dir = h.repo_root.join(".ralph");
        std::fs::create_dir_all(&ralph_dir).expect("create dir");
        std::fs::write(ralph_dir.join("random_file.txt"), "stuff").expect("write file");

        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 2);

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("directory exists but is not a ralph workspace (no ralph.toml found)"),
            "expected exact non-workspace error message, got:\n{combined}"
        );

        // Verify no files were added.
        assert!(
            !ralph_dir.join("ralph.toml").exists(),
            "ralph.toml should not have been created"
        );
        assert!(
            !ralph_dir.join("templates").exists(),
            "templates/ should not have been created"
        );
    })
}

fn copy_files_rejects_malformed_toml(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a workspace with a malformed ralph.toml.
        let ralph_dir = h.repo_root.join(".ralph");
        std::fs::create_dir_all(&ralph_dir).expect("create dir");
        std::fs::write(
            ralph_dir.join("ralph.toml"),
            "[workspace]\nversion = [1, 2, 3]\n",
        )
        .expect("write malformed ralph.toml");

        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 1);

        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("failed to parse ralph.toml"),
            "expected 'failed to parse ralph.toml' in output, got:\n{combined}"
        );

        // Existing file should be unchanged.
        let raw = std::fs::read_to_string(ralph_dir.join("ralph.toml")).expect("read ralph.toml");
        assert!(
            raw.contains("version = [1, 2, 3]"),
            "malformed ralph.toml should be unchanged"
        );
        assert!(
            !ralph_dir.join("templates").exists(),
            "templates/ should not have been created on malformed toml"
        );
    })
}

fn copy_files_dry_run_full_scaffold(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let output = h
            .ralph(["init", "--copy-files", "--dry-run"])
            .expect("ralph init --copy-files --dry-run should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should include template actions.
        assert!(
            stdout.contains("write-template"),
            "dry-run --copy-files should show write-template actions, got:\n{stdout}"
        );
        assert!(
            stdout.contains("write-config"),
            "dry-run --copy-files should show write-config action, got:\n{stdout}"
        );

        // Should NOT have created any files.
        assert_path_not_exists(&h.repo_root.join(".ralph"));
    })
}

fn copy_files_dry_run_overlay(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a minimal workspace first.
        h.init_workspace().expect("init should succeed");

        let output = h
            .ralph(["init", "--copy-files", "--dry-run"])
            .expect("ralph init --copy-files --dry-run should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Should include overlay config action.
        assert!(
            stdout.contains("overlay-config"),
            "dry-run --copy-files on existing workspace should show overlay-config, got:\n{stdout}"
        );
        // Should include template actions (for missing templates).
        assert!(
            stdout.contains("write-template"),
            "dry-run --copy-files on existing workspace should show write-template actions, got:\n{stdout}"
        );

        // The workspace config should be unchanged (dry-run).
        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let raw = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        assert!(
            raw.contains("[workspace]"),
            "config should still contain [workspace]"
        );
        // Templates dir should NOT exist (dry-run).
        assert!(
            !h.repo_root.join(".ralph").join("templates").exists(),
            "templates/ should not be created during dry-run"
        );
    })
}

fn copy_files_overlay_inline_table_merge(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Create a workspace with inline-table syntax in ralph.toml.
        let ralph_dir = h.repo_root.join(".ralph");
        std::fs::create_dir_all(ralph_dir.join("projects")).expect("create projects dir");
        std::fs::write(
            ralph_dir.join("ralph.toml"),
            "workspace = { default_backend = \"codex\" }\n",
        )
        .expect("write inline-table config");

        // Run --copy-files overlay on existing workspace with inline tables.
        let output = h
            .ralph(["init", "--copy-files"])
            .expect("ralph init --copy-files should execute");
        assert_exit_code(&output, 0);

        let toml_path = ralph_dir.join("ralph.toml");
        let merged_str =
            std::fs::read_to_string(&toml_path).expect("read merged ralph.toml");
        let parsed: GlobalConfig =
            toml::from_str(&merged_str).expect("merged config should parse");

        // User value should be preserved.
        assert_eq!(
            parsed.workspace.default_backend, "codex",
            "inline-table custom value should be preserved"
        );
        // Missing default keys should be filled.
        assert_eq!(
            parsed.workspace.version,
            GlobalConfig::default().workspace.version,
            "missing version should be filled from defaults"
        );

        // Verify file-level key insertion.
        let doc: DocumentMut = merged_str.parse().expect("parse as doc");
        assert!(
            doc.get("backends").is_some(),
            "backends section should be physically present after inline-table overlay merge"
        );
    })
}
