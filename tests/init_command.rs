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
        workspace_root.join("index.json").exists(),
        "index.json should exist"
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
    assert_eq!(workspace.config.backends.claude.timeout_seconds, 600);
    assert_eq!(workspace.config.backends.codex.timeout_seconds, 600);

    // Verify workflow section
    assert_eq!(workspace.config.workflow.max_review_iterations, 5);
    assert!(workspace.config.workflow.auto_commit);

    // Verify templates section
    assert_eq!(workspace.config.templates.planner, "templates/planner.md");
    assert_eq!(
        workspace.config.templates.implementer,
        "templates/implementer.md"
    );
    assert_eq!(workspace.config.templates.reviewer, "templates/reviewer.md");
    assert_eq!(
        workspace.config.templates.completer,
        "templates/completer.md"
    );

    // Verify git section
    assert!(workspace.config.git.auto_branch);
    assert_eq!(workspace.config.git.branch_format, "ralph/{project_id}");
    assert_eq!(workspace.config.git.base_branch, "master");
}

#[test]
fn test_init_generates_valid_index() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    let workspace = Workspace::init(&workspace_root).expect("init should succeed");

    // Verify index contents
    assert_eq!(workspace.index.workspace_version, "1.0");
    assert!(workspace.index.active_project.is_none());
    assert!(workspace.index.projects.is_empty());
}

#[test]
fn test_init_with_custom_directory_relative() {
    let temp = TempDir::new().expect("temp dir");
    let custom_dir = temp.path().join("custom-workspace");

    let workspace = Workspace::init(&custom_dir).expect("init should succeed");

    assert!(custom_dir.exists(), "custom directory should exist");
    assert!(custom_dir.join("ralph.toml").exists());
    assert!(custom_dir.join("index.json").exists());
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
    Workspace::init(&workspace_root).expect("first init should succeed");

    // Second init should fail
    let result = Workspace::init(&workspace_root);
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
    let _original_workspace = Workspace::init(&workspace_root).expect("first init should succeed");
    let original_config_content =
        fs::read_to_string(workspace_root.join("ralph.toml")).expect("read original config");
    let original_index_content =
        fs::read_to_string(workspace_root.join("index.json")).expect("read original index");

    // Attempt second init (should fail)
    let _ = Workspace::init(&workspace_root);

    // Verify original files are unchanged
    let current_config_content = fs::read_to_string(workspace_root.join("ralph.toml"))
        .expect("read config after failed reinit");
    let current_index_content = fs::read_to_string(workspace_root.join("index.json"))
        .expect("read index after failed reinit");

    assert_eq!(
        original_config_content, current_config_content,
        "config should be unchanged after failed reinit"
    );
    assert_eq!(
        original_index_content, current_index_content,
        "index should be unchanged after failed reinit"
    );
}

#[test]
fn test_init_workspace_can_be_loaded_after_creation() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    // Create workspace
    Workspace::init(&workspace_root).expect("init should succeed");

    // Load it back
    let loaded = Workspace::load(workspace_root.clone()).expect("load should succeed");

    assert_eq!(loaded.root, workspace_root);
    assert_eq!(loaded.config.workspace.version, "1.0");
    assert!(loaded.index.projects.is_empty());
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
    };

    // Execute the actual CLI init flow
    execute(args).expect("cli init execute should succeed");

    // Verify template files exist (created by the real execute function)
    assert!(
        workspace_root.join("templates/planner.md").exists(),
        "planner template should exist"
    );
    assert!(
        workspace_root.join("templates/implementer.md").exists(),
        "implementer template should exist"
    );
    assert!(
        workspace_root.join("templates/reviewer.md").exists(),
        "reviewer template should exist"
    );
    assert!(
        workspace_root.join("templates/completer.md").exists(),
        "completer template should exist"
    );

    // Verify template content is correct
    let planner_content =
        fs::read_to_string(workspace_root.join("templates/planner.md")).expect("read planner");
    assert!(planner_content.contains("software architect"));
    assert!(planner_content.contains("# Feature:"));

    let implementer_content =
        fs::read_to_string(workspace_root.join("templates/implementer.md")).expect("read impl");
    assert!(implementer_content.contains("software developer"));
    assert!(implementer_content.contains("# Implementation Notes"));

    let reviewer_content =
        fs::read_to_string(workspace_root.join("templates/reviewer.md")).expect("read reviewer");
    assert!(reviewer_content.contains("code reviewer"));
    assert!(reviewer_content.contains("# Review: APPROVED"));

    let completer_content =
        fs::read_to_string(workspace_root.join("templates/completer.md")).expect("read completer");
    assert!(completer_content.contains("project completion validator"));
    assert!(completer_content.contains("# Verdict:"));
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
    assert!(workspace_root.join("index.json").exists());
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
    };

    execute(args).expect("cli init with absolute path should succeed");

    // Verify workspace was created at the absolute path
    assert!(
        absolute_path.join("ralph.toml").exists(),
        "ralph.toml should exist at absolute path"
    );
    assert!(
        absolute_path.join("index.json").exists(),
        "index.json should exist at absolute path"
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
        absolute_path.join("templates/planner.md").exists(),
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
        resolved_path.join("index.json").exists(),
        "index.json should exist at relative path"
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
        resolved_path.join("templates/planner.md").exists(),
        "template files should exist at relative path"
    );
}
