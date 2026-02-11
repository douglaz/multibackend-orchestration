//! Integration tests for git operations.

use std::fs;
use std::process::Command;

use ralph::git::commit::{
    changed_paths_excluding_prefixes, commit_feature_loop, has_uncommitted_changes, ref_exists,
    reset_and_clean_working_tree, reset_hard, rev_parse, stage_implementation_changes, staged_diff,
    unstaged_diff, working_tree_diff, working_tree_diff_excluding_orchestration_state,
};
use ralph::git::{has_conflicts, is_git_repo};
use tempfile::TempDir;

fn init_test_repo() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Create initial commit
    fs::write(temp_dir.path().join("README.md"), "# Test").unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    temp_dir
}

fn init_unborn_repo() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    temp_dir
}

#[test]
fn test_is_git_repo_true() {
    let temp_dir = init_test_repo();
    assert!(is_git_repo(temp_dir.path()));
}

#[test]
fn test_is_git_repo_false() {
    let temp_dir = TempDir::new().unwrap();
    assert!(!is_git_repo(temp_dir.path()));
}

#[test]
fn test_working_tree_diff_includes_staged_and_unstaged() {
    let temp_dir = init_test_repo();

    // Modify existing file and stage it
    fs::write(temp_dir.path().join("README.md"), "# Staged changes").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Create a new file and stage it, then modify it (now has both staged and unstaged)
    fs::write(temp_dir.path().join("both.txt"), "initial staged").unwrap();
    Command::new("git")
        .args(["add", "both.txt"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    fs::write(temp_dir.path().join("both.txt"), "modified unstaged").unwrap();

    let diff = working_tree_diff(temp_dir.path()).unwrap();

    // diff HEAD should show both staged and unstaged changes
    assert!(diff.contains("README.md"), "Should include staged file");
    assert!(
        diff.contains("both.txt"),
        "Should include file with unstaged changes"
    );
}

#[test]
fn test_working_tree_diff_without_head_commit() {
    let temp_dir = init_unborn_repo();

    fs::write(temp_dir.path().join("both.txt"), "staged content").unwrap();
    Command::new("git")
        .args(["add", "both.txt"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Modify after staging so the same tracked path has unstaged changes too.
    fs::write(temp_dir.path().join("both.txt"), "unstaged content").unwrap();

    let diff = working_tree_diff(temp_dir.path()).unwrap();
    assert!(diff.contains("both.txt"), "Should include tracked file");
    assert!(
        diff.contains("staged content"),
        "Should include staged content"
    );
    assert!(
        diff.contains("unstaged content"),
        "Should include unstaged content"
    );
}

#[test]
fn test_working_tree_diff_excludes_orchestration_state() {
    let temp_dir = init_test_repo();
    fs::create_dir_all(temp_dir.path().join(".ralph")).unwrap();

    fs::write(temp_dir.path().join(".ralph/index.json"), "{}\n").unwrap();
    fs::write(temp_dir.path().join("README.md"), "# Updated").unwrap();

    let diff = working_tree_diff_excluding_orchestration_state(temp_dir.path()).unwrap();
    assert!(
        diff.contains("README.md"),
        "Diff should include non-orchestration file changes"
    );
    assert!(
        !diff.contains(".ralph/index.json"),
        "Diff should exclude orchestration runtime artifacts"
    );
}

#[test]
fn test_stage_implementation_changes_includes_new_files() {
    let temp_dir = init_test_repo();
    fs::create_dir_all(temp_dir.path().join(".ralph")).unwrap();

    fs::write(temp_dir.path().join("new_module.rs"), "pub fn demo() {}\n").unwrap();
    fs::write(
        temp_dir.path().join(".ralph/runtime.json"),
        "{\"loop\":1}\n",
    )
    .unwrap();

    stage_implementation_changes(temp_dir.path()).unwrap();

    let diff = working_tree_diff_excluding_orchestration_state(temp_dir.path()).unwrap();
    assert!(
        diff.contains("new_module.rs"),
        "Reviewer diff should include newly created non-orchestration files"
    );

    let staged = staged_diff(temp_dir.path()).unwrap();
    assert!(
        !staged.contains(".ralph/runtime.json"),
        "Staging helper should not stage orchestration runtime files"
    );
}

#[test]
fn test_reset_and_clean_working_tree_preserves_ralph() {
    let temp_dir = init_test_repo();
    fs::create_dir_all(temp_dir.path().join(".ralph/runtime")).unwrap();

    fs::write(temp_dir.path().join("README.md"), "# Modified\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("scratch.txt"), "temporary\n").unwrap();
    fs::write(
        temp_dir.path().join(".ralph/runtime/session.json"),
        "{\"active\":true}\n",
    )
    .unwrap();

    reset_and_clean_working_tree(temp_dir.path()).unwrap();

    assert_eq!(
        fs::read_to_string(temp_dir.path().join("README.md")).unwrap(),
        "# Test",
        "Tracked non-orchestration file should be restored to HEAD"
    );
    assert!(
        !temp_dir.path().join("scratch.txt").exists(),
        "Untracked non-orchestration file should be removed"
    );
    assert!(
        temp_dir.path().join(".ralph/runtime/session.json").exists(),
        "Orchestration runtime state should be preserved"
    );

    let changed =
        changed_paths_excluding_prefixes(temp_dir.path(), &[".ralph/"]).expect("status parse");
    assert!(
        changed.is_empty(),
        "No non-orchestration changes should remain after cleanup"
    );
}

#[test]
fn test_staged_diff_only_staged() {
    let temp_dir = init_test_repo();

    // Create staged change
    fs::write(temp_dir.path().join("staged.txt"), "staged content").unwrap();
    Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Create unstaged change
    fs::write(temp_dir.path().join("unstaged.txt"), "unstaged content").unwrap();

    let diff = staged_diff(temp_dir.path()).unwrap();

    assert!(diff.contains("staged.txt"), "Should include staged file");
    assert!(
        !diff.contains("unstaged.txt"),
        "Should not include unstaged file"
    );
}

#[test]
fn test_unstaged_diff_only_unstaged() {
    let temp_dir = init_test_repo();

    // Create staged change
    fs::write(temp_dir.path().join("staged.txt"), "staged content").unwrap();
    Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Create unstaged change (modify existing file)
    fs::write(temp_dir.path().join("README.md"), "# Modified").unwrap();

    let diff = unstaged_diff(temp_dir.path()).unwrap();

    assert!(
        !diff.contains("staged.txt"),
        "Should not include staged file"
    );
    assert!(diff.contains("README.md"), "Should include unstaged file");
}

#[test]
fn test_commit_feature_loop() {
    let temp_dir = init_test_repo();

    fs::write(temp_dir.path().join("feature.txt"), "new feature").unwrap();

    let commit_hash = commit_feature_loop(
        temp_dir.path(),
        "feat: add new feature",
        Some("ralph/test/loop-1"),
        false,
    )
    .unwrap();

    assert!(!commit_hash.is_empty());
    assert!(ref_exists(temp_dir.path(), "ralph/test/loop-1").unwrap());
}

#[test]
fn test_commit_feature_loop_without_tag() {
    let temp_dir = init_test_repo();

    fs::write(temp_dir.path().join("feature.txt"), "new feature").unwrap();

    let commit_hash =
        commit_feature_loop(temp_dir.path(), "feat: add new feature", None, false).unwrap();

    assert!(!commit_hash.is_empty());
}

#[test]
fn test_commit_feature_loop_includes_orchestration_state_files() {
    let temp_dir = init_test_repo();
    fs::create_dir_all(temp_dir.path().join(".ralph")).unwrap();

    fs::write(temp_dir.path().join(".ralph/index.json"), "{}\n").unwrap();
    fs::write(temp_dir.path().join("feature.txt"), "new feature").unwrap();

    commit_feature_loop(temp_dir.path(), "feat: add new feature", None, false).unwrap();

    let output = Command::new("git")
        .args(["show", "--name-only", "--pretty=format:", "HEAD"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "git show should succeed");
    let files = String::from_utf8_lossy(&output.stdout);

    assert!(
        files.lines().any(|line| line.trim() == "feature.txt"),
        "Feature file should be committed"
    );
    assert!(
        files.lines().any(|line| line.trim() == ".ralph/index.json"),
        "Orchestration runtime files should remain committable for project history"
    );
}

#[test]
fn test_changed_paths_excludes_prefixes() {
    let temp_dir = init_test_repo();
    fs::create_dir_all(temp_dir.path().join(".ralph")).unwrap();

    fs::write(temp_dir.path().join(".ralph/index.json"), "{}\n").unwrap();
    fs::write(temp_dir.path().join("notes.md"), "hello\n").unwrap();

    let changed =
        changed_paths_excluding_prefixes(temp_dir.path(), &[".ralph/"]).expect("status parse");
    assert!(
        changed.iter().any(|path| path == "notes.md"),
        "Non-excluded files should remain visible"
    );
    assert!(
        !changed.iter().any(|path| path.starts_with(".ralph/")),
        "Excluded prefix files should be filtered out"
    );
}

#[test]
fn test_has_uncommitted_changes() {
    let temp_dir = init_test_repo();

    // No uncommitted changes initially
    assert!(!has_uncommitted_changes(temp_dir.path()).unwrap());

    // Create a change
    fs::write(temp_dir.path().join("README.md"), "# Modified").unwrap();

    assert!(has_uncommitted_changes(temp_dir.path()).unwrap());
}

#[test]
fn test_rev_parse_head() {
    let temp_dir = init_test_repo();

    let head = rev_parse(temp_dir.path(), "HEAD").unwrap();
    assert!(!head.is_empty());
    // SHA-1 hash is 40 characters
    assert_eq!(head.len(), 40);
}

#[test]
fn test_ref_exists() {
    let temp_dir = init_test_repo();

    assert!(ref_exists(temp_dir.path(), "HEAD").unwrap());
    assert!(!ref_exists(temp_dir.path(), "nonexistent-ref").unwrap());
}

#[test]
fn test_reset_hard() {
    let temp_dir = init_test_repo();

    // Save initial HEAD
    let initial_head = rev_parse(temp_dir.path(), "HEAD").unwrap();

    // Create new commit
    fs::write(temp_dir.path().join("new.txt"), "new").unwrap();
    commit_feature_loop(temp_dir.path(), "new commit", None, false).unwrap();

    let new_head = rev_parse(temp_dir.path(), "HEAD").unwrap();
    assert_ne!(initial_head, new_head);

    // Reset to initial
    reset_hard(temp_dir.path(), &initial_head).unwrap();

    let current_head = rev_parse(temp_dir.path(), "HEAD").unwrap();
    assert_eq!(current_head, initial_head);
}

#[test]
fn test_has_conflicts_no_conflicts() {
    let temp_dir = init_test_repo();

    // Clean repo should have no conflicts
    assert!(!has_conflicts(temp_dir.path()).unwrap());
}

#[test]
fn test_has_conflicts_with_conflict() {
    let temp_dir = init_test_repo();

    // Create a branch and make conflicting changes
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("README.md"), "# Feature branch").unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Feature change"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("README.md"), "# Master branch").unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Master change"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Attempt merge that will conflict
    let output = Command::new("git")
        .args(["merge", "feature"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Only run conflict check if merge actually created a conflict
    if !output.status.success() {
        assert!(has_conflicts(temp_dir.path()).unwrap());
    }
}

#[test]
fn test_commit_feature_loop_detects_conflict() {
    let temp_dir = init_test_repo();

    // Create a branch and make conflicting changes
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("README.md"), "# Feature branch").unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Feature change"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    fs::write(temp_dir.path().join("README.md"), "# Master branch").unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Master change"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Attempt merge that will conflict
    let output = Command::new("git")
        .args(["merge", "feature"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    // Only test if merge created a conflict
    if !output.status.success() {
        let result = commit_feature_loop(
            temp_dir.path(),
            "This should fail due to conflicts",
            None,
            false,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_str = err.to_string();
        assert!(
            err_str.contains("conflict"),
            "Error should mention conflict: {err_str}"
        );
    }
}
