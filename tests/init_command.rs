//! Integration tests for the `ralph init` command.

use std::fs;

use ralph::workspace::Workspace;
use tempfile::TempDir;

#[test]
fn test_init_creates_workspace_structure() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    let workspace = Workspace::init(&workspace_root).expect("init should succeed");

    // Verify directory structure
    assert!(workspace_root.exists(), ".ralph directory should exist");
    assert!(
        workspace_root.join("projects").exists(),
        "projects directory should exist"
    );
    assert!(
        workspace_root.join("templates").exists(),
        "templates directory should exist"
    );
    assert!(
        workspace_root.join("ralph.toml").exists(),
        "ralph.toml should exist"
    );
    assert!(
        !workspace_root.join("index.json").exists(),
        "index.json should NOT be created"
    );

    // Verify workspace root is correct
    assert_eq!(workspace.root, workspace_root);
}

#[test]
fn test_init_generates_valid_config() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    let workspace = Workspace::init(&workspace_root).expect("init should succeed");

    // Verify required config sections exist with expected defaults
    assert_eq!(workspace.config.workspace.version, "1.0");
    assert_eq!(workspace.config.workspace.default_backend, "claude");
    assert!(!workspace.config.workspace.tmux);
    assert_eq!(workspace.config.workspace.tmux_session, "ralph");

    // Verify backends section
    assert_eq!(workspace.config.backends.claude.command, "claude");
    assert_eq!(workspace.config.backends.codex.command, "codex");
    assert_eq!(workspace.config.backends.claude.timeout_seconds, 7200);
    assert_eq!(workspace.config.backends.codex.timeout_seconds, 7200);

    // Verify workflow section
    assert_eq!(workspace.config.workflow.max_review_iterations, 30);
    assert!(workspace.config.workflow.auto_commit);

    // Verify templates section (canonical names)
    assert_eq!(workspace.config.templates.planner, "templates/spec.md");
    assert_eq!(
        workspace.config.templates.implementer,
        "templates/implementation.md"
    );
    assert_eq!(workspace.config.templates.reviewer, "templates/review.md");
    assert_eq!(
        workspace.config.templates.completer,
        "templates/completion.md"
    );

    // Verify git section
    assert!(workspace.config.git.auto_branch);
    assert_eq!(workspace.config.git.branch_format, "ralph/{project_id}");
    assert_eq!(workspace.config.git.base_branch, "master");
}

#[test]
fn test_init_does_not_create_index_json() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    Workspace::init(&workspace_root).expect("init should succeed");

    // index.json should NOT be created
    assert!(
        !workspace_root.join("index.json").exists(),
        "index.json should not exist after init"
    );
}

#[test]
fn test_init_with_custom_directory_relative() {
    let temp = TempDir::new().expect("temp dir");
    let custom_dir = temp.path().join("custom-workspace");

    let workspace = Workspace::init(&custom_dir).expect("init should succeed");

    assert!(custom_dir.exists(), "custom directory should exist");
    assert!(custom_dir.join("ralph.toml").exists());
    assert!(!custom_dir.join("index.json").exists());
    assert!(custom_dir.join("projects").exists());
    assert!(custom_dir.join("templates").exists());
    assert_eq!(workspace.root, custom_dir);
}

#[test]
fn test_init_with_nested_custom_directory() {
    let temp = TempDir::new().expect("temp dir");
    let nested_dir = temp.path().join("path").join("to").join("workspace");

    let workspace = Workspace::init(&nested_dir).expect("init should succeed");

    assert!(nested_dir.exists(), "nested directory should be created");
    assert!(nested_dir.join("ralph.toml").exists());
    assert_eq!(workspace.root, nested_dir);
}

#[test]
fn test_init_fails_on_existing_non_empty_workspace() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    // First init should succeed
    let args = ralph::cli::InitArgs {
        dir: workspace_root.clone(),
        dry_run: false,
    };
    ralph::cli::init::execute(args).expect("first init should succeed");

    // Second init should fail
    let args = ralph::cli::InitArgs {
        dir: workspace_root.clone(),
        dry_run: false,
    };
    let result = ralph::cli::init::execute(args);
    assert!(result.is_err(), "reinit should fail");

    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("already exists") && err_msg.contains("not empty"),
        "error should mention directory already exists and is not empty: {}",
        err_msg
    );
}

#[test]
fn test_init_does_not_partially_overwrite_on_failure() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    // First init
    let args = ralph::cli::InitArgs {
        dir: workspace_root.clone(),
        dry_run: false,
    };
    ralph::cli::init::execute(args).expect("first init should succeed");
    let original_config_content =
        fs::read_to_string(workspace_root.join("ralph.toml")).expect("read original config");

    // Attempt second init (should fail)
    let args = ralph::cli::InitArgs {
        dir: workspace_root.clone(),
        dry_run: false,
    };
    let _ = ralph::cli::init::execute(args);

    // Verify original files are unchanged
    let current_config_content = fs::read_to_string(workspace_root.join("ralph.toml"))
        .expect("read config after failed reinit");

    assert_eq!(
        original_config_content, current_config_content,
        "config should be unchanged after failed reinit"
    );
}

#[test]
fn test_init_workspace_can_be_loaded_after_creation() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    // Create workspace
    Workspace::init(&workspace_root).expect("init should succeed");

    // Load it back (no index.json required)
    let loaded = Workspace::load(workspace_root.clone()).expect("load should succeed");

    assert_eq!(loaded.root, workspace_root);
    assert_eq!(loaded.config.workspace.version, "1.0");
}

#[test]
fn test_init_creates_template_files_with_cli_execute() {
    // This test verifies the full CLI flow including template file creation
    // by calling the real cli::init::execute function
    use ralph::cli::init::execute;
    use ralph::cli::InitArgs;

    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    let args = InitArgs {
        dir: workspace_root.clone(),
        dry_run: false,
    };

    // Execute the actual CLI init flow
    execute(args).expect("cli init execute should succeed");

    // Verify canonical template files exist
    assert!(
        workspace_root.join("templates/spec.md").exists(),
        "spec template should exist"
    );
    assert!(
        workspace_root.join("templates/implementation.md").exists(),
        "implementation template should exist"
    );
    assert!(
        workspace_root.join("templates/review.md").exists(),
        "review template should exist"
    );
    assert!(
        workspace_root.join("templates/completion.md").exists(),
        "completion template should exist"
    );
    assert!(
        workspace_root.join("templates/qa.md").exists(),
        "qa template should exist"
    );

    // Verify legacy symlinks exist for backward compatibility
    assert!(
        workspace_root.join("templates/planner.md").exists(),
        "legacy planner.md symlink should exist"
    );
    assert!(
        workspace_root.join("templates/implementer.md").exists(),
        "legacy implementer.md symlink should exist"
    );
    assert!(
        workspace_root.join("templates/reviewer.md").exists(),
        "legacy reviewer.md symlink should exist"
    );
    assert!(
        workspace_root.join("templates/completer.md").exists(),
        "legacy completer.md symlink should exist"
    );

    // Verify template content is correct (read via canonical names)
    let planner_content =
        fs::read_to_string(workspace_root.join("templates/spec.md")).expect("read spec");
    assert!(planner_content.contains("software architect"));
    assert!(planner_content.contains("# Feature:"));

    let implementer_content =
        fs::read_to_string(workspace_root.join("templates/implementation.md")).expect("read impl");
    assert!(implementer_content.contains("software developer"));
    assert!(implementer_content.contains("# Implementation Notes"));

    let reviewer_content =
        fs::read_to_string(workspace_root.join("templates/review.md")).expect("read reviewer");
    assert!(reviewer_content.contains("code reviewer"));
    assert!(reviewer_content.contains("# Review: APPROVED"));

    let completer_content =
        fs::read_to_string(workspace_root.join("templates/completion.md")).expect("read completer");
    assert!(completer_content.contains("project completion validator"));
    assert!(completer_content.contains("# Verdict:"));

    let qa_content =
        fs::read_to_string(workspace_root.join("templates/qa.md")).expect("read qa template");
    assert!(qa_content.contains("QA engineer"));
    assert!(qa_content.contains("# QA: PASS"));
    assert!(qa_content.contains("# QA: FAIL"));
    assert!(qa_content.contains("Do NOT edit any source files"));
}

#[test]
fn test_init_allows_empty_existing_directory() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    // Create the directory but leave it empty
    fs::create_dir_all(&workspace_root).expect("create empty dir");

    // Init should succeed on empty directory
    let _workspace = Workspace::init(&workspace_root).expect("init on empty dir should succeed");

    assert!(workspace_root.join("ralph.toml").exists());
    assert!(!workspace_root.join("index.json").exists());
}

#[test]
fn test_init_cli_with_absolute_path() {
    // Tests --dir with an absolute path through the real CLI execute flow
    use ralph::cli::init::execute;
    use ralph::cli::InitArgs;

    let temp = TempDir::new().expect("temp dir");
    // temp.path() returns an absolute path
    let absolute_path = temp.path().join("absolute-workspace");

    // Verify it's an absolute path
    assert!(
        absolute_path.is_absolute(),
        "test path should be absolute: {:?}",
        absolute_path
    );

    let args = InitArgs {
        dir: absolute_path.clone(),
        dry_run: false,
    };

    execute(args).expect("cli init with absolute path should succeed");

    // Verify workspace was created at the absolute path
    assert!(
        absolute_path.join("ralph.toml").exists(),
        "ralph.toml should exist at absolute path"
    );
    assert!(
        !absolute_path.join("index.json").exists(),
        "index.json should NOT exist at absolute path"
    );
    assert!(
        absolute_path.join("projects").exists(),
        "projects directory should exist at absolute path"
    );
    assert!(
        absolute_path.join("templates").exists(),
        "templates directory should exist at absolute path"
    );
    assert!(
        absolute_path.join("templates/spec.md").exists(),
        "template files should exist at absolute path"
    );
}

#[test]
fn test_init_cli_with_relative_path() {
    // Tests --dir with a relative path through the real CLI execute flow
    use ralph::cli::init::execute;
    use ralph::cli::InitArgs;
    use std::env;
    use std::path::PathBuf;

    let temp = TempDir::new().expect("temp dir");
    let original_dir = env::current_dir().expect("get current dir");

    // Change to temp directory so we can use a relative path
    env::set_current_dir(temp.path()).expect("change to temp dir");

    // Use a relative path
    let relative_path = PathBuf::from("my-relative-workspace");

    // Verify it's a relative path
    assert!(
        !relative_path.is_absolute(),
        "test path should be relative: {:?}",
        relative_path
    );

    let args = InitArgs {
        dir: relative_path.clone(),
        dry_run: false,
    };

    let result = execute(args);

    // Restore original directory before assertions
    env::set_current_dir(&original_dir).expect("restore original dir");

    result.expect("cli init with relative path should succeed");

    // Verify workspace was created at the relative path (resolved from temp dir)
    let resolved_path = temp.path().join(&relative_path);
    assert!(
        resolved_path.join("ralph.toml").exists(),
        "ralph.toml should exist at relative path"
    );
    assert!(
        !resolved_path.join("index.json").exists(),
        "index.json should NOT exist at relative path"
    );
    assert!(
        resolved_path.join("projects").exists(),
        "projects directory should exist at relative path"
    );
    assert!(
        resolved_path.join("templates").exists(),
        "templates directory should exist at relative path"
    );
    assert!(
        resolved_path.join("templates/spec.md").exists(),
        "template files should exist at relative path"
    );
}
