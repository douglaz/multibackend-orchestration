use super::*;

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
    ]
}

fn creates_workspace_structure(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let ralph_dir = h.repo_root.join(".ralph");
        assert_dir_exists(&ralph_dir);
        assert_file_exists(&ralph_dir.join("ralph.toml"));
        assert_dir_exists(&ralph_dir.join("projects"));
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn creates_minimal_config(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.init_workspace().expect("ralph init should succeed");

        let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
        let raw_toml = std::fs::read_to_string(&toml_path).expect("read ralph.toml");
        let parsed: GlobalConfig = toml::from_str(&raw_toml).expect("ralph.toml should parse");
        assert_eq!(parsed, GlobalConfig::default());
        assert!(!h.repo_root.join(".ralph").join("templates").exists());
    })) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}

fn default_config(h: &RalphHarness) -> TestResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
dry-run: write-config .ralph/ralph.toml
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
