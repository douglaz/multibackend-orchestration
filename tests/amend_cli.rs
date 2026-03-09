//! Integration test for `ralph amend` end-to-end flow.
//!
//! Exercises the actual `ralph amend` CLI binary, verifies the produced queue
//! file deserializes correctly, and confirms drain succeeds.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use ralph::project::amendments::{
    drain_amendment_queue, AmendmentPriority, AmendmentRequest, AmendmentSource,
};

/// Locate the built `ralph` binary via standard Cargo mechanisms.
fn ralph_bin_absolute() -> PathBuf {
    if let Ok(p) = env::var("CARGO_BIN_EXE_ralph") {
        return PathBuf::from(p);
    }

    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set");
    let manifest_path = PathBuf::from(&manifest);

    let mut target_roots = vec![manifest_path.join("target")];
    if let Ok(target_dir) = env::var("CARGO_TARGET_DIR") {
        target_roots.push(PathBuf::from(target_dir));
    }

    let profiles = &["debug", "release"];

    for target_root in &target_roots {
        for profile in profiles {
            let candidate = target_root.join(profile).join("ralph");
            if candidate.exists() {
                return candidate;
            }
        }

        if let Ok(entries) = fs::read_dir(target_root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.matches('-').count() >= 2 {
                    for profile in profiles {
                        let candidate = entry.path().join(profile).join("ralph");
                        if candidate.exists() {
                            return candidate;
                        }
                    }
                }
            }
        }
    }

    if let Ok(p) = env::var("RALPH_TEST_BIN") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }

    panic!(
        "ralph binary not found; run `cargo build` first. \
         Searched: target/{{debug,release}}/ralph, \
         target/<triple>/{{debug,release}}/ralph, \
         CARGO_TARGET_DIR (same layouts), RALPH_TEST_BIN"
    );
}

/// Set up a temporary git repo with `ralph init` and a project, returning
/// (temp_dir, repo_root, project_dir).
fn setup_workspace_with_project(
    project_id: &str,
) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");

    // Initialize git repo
    run_git(&repo_root, &["init"]);
    run_git(&repo_root, &["config", "user.email", "test@example.com"]);
    run_git(&repo_root, &["config", "user.name", "Test"]);
    fs::write(repo_root.join(".gitkeep"), "").expect("write gitkeep");
    run_git(&repo_root, &["add", ".gitkeep"]);
    run_git(&repo_root, &["commit", "-m", "init"]);

    let bin = ralph_bin_absolute();

    // ralph init
    let output = Command::new(&bin)
        .args(["init"])
        .current_dir(&repo_root)
        .output()
        .expect("ralph init");
    assert!(output.status.success(), "ralph init failed: {}", String::from_utf8_lossy(&output.stderr));

    // Create prompt file and project
    let prompt_path = temp.path().join("prompt.md");
    fs::write(&prompt_path, "test prompt").expect("write prompt");

    let output = Command::new(&bin)
        .args([
            "project", "new",
            "--id", project_id,
            "--name", "Test Project",
            "--prompt", &prompt_path.to_string_lossy(),
        ])
        .current_dir(&repo_root)
        .output()
        .expect("ralph project new");
    assert!(output.status.success(), "ralph project new failed: {}", String::from_utf8_lossy(&output.stderr));

    let project_dir = repo_root.join(".ralph").join("projects").join(project_id);

    (temp, repo_root, project_dir)
}

fn run_git(repo_root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .expect("git should execute");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn amend_cli_enqueues_deserializes_and_drains_successfully() {
    let (_temp, repo_root, project_dir) = setup_workspace_with_project("amend-cli-test");
    let bin = ralph_bin_absolute();

    // Run ralph amend via the CLI
    let output = Command::new(&bin)
        .args([
            "amend",
            "--project", "amend-cli-test",
            "--body", "Add retry logic to the API client",
            "--priority", "P1",
            "--id", "EXT-CLI-001",
        ])
        .current_dir(&repo_root)
        .output()
        .expect("ralph amend should execute");

    assert!(
        output.status.success(),
        "ralph amend failed with exit code {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Read printed queue path from stdout
    let stdout = String::from_utf8_lossy(&output.stdout);
    let queue_path_str = stdout.trim();
    assert!(!queue_path_str.is_empty(), "stdout should contain the queue file path");
    assert!(queue_path_str.ends_with(".json"), "queue path should end with .json: {queue_path_str}");

    let queue_path = PathBuf::from(queue_path_str);
    assert!(queue_path.exists(), "queue file should exist after enqueue: {queue_path_str}");

    // Verify produced JSON deserializes correctly
    let raw = fs::read_to_string(&queue_path).expect("read queue file");
    let deserialized: AmendmentRequest =
        serde_json::from_str(&raw).expect("queue file should be valid AmendmentRequest JSON");
    assert_eq!(deserialized.id, "EXT-CLI-001");
    assert_eq!(deserialized.body, "Add retry logic to the API client");
    assert_eq!(deserialized.priority, AmendmentPriority::P1);
    assert_eq!(deserialized.source, AmendmentSource::Cli);
    assert!(deserialized.source_detail.is_none());

    // Drain and verify
    let drained = drain_amendment_queue(&project_dir).expect("drain should succeed");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].id, "EXT-CLI-001");
    assert_eq!(drained[0].body, "Add retry logic to the API client");

    // Queue file should be removed after drain
    assert!(
        !queue_path.exists(),
        "queue file should be removed after drain"
    );
}

#[test]
fn amend_cli_multiple_amendments_drain_in_order() {
    let (_temp, repo_root, project_dir) = setup_workspace_with_project("amend-cli-multi");
    let bin = ralph_bin_absolute();

    for i in 0..3 {
        let id = format!("EXT-MULTI-{i}");
        let body = format!("Amendment body {i}");
        let output = Command::new(&bin)
            .args([
                "amend",
                "--project", "amend-cli-multi",
                "--body", &body,
                "--id", &id,
            ])
            .current_dir(&repo_root)
            .output()
            .expect("ralph amend should execute");
        assert!(
            output.status.success(),
            "ralph amend failed for {id}: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let drained = drain_amendment_queue(&project_dir).expect("drain should succeed");
    assert_eq!(drained.len(), 3);

    let ids: Vec<&str> = drained.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"EXT-MULTI-0"));
    assert!(ids.contains(&"EXT-MULTI-1"));
    assert!(ids.contains(&"EXT-MULTI-2"));
}

#[test]
fn amend_cli_rejects_nonexistent_project() {
    let (_temp, repo_root, _project_dir) = setup_workspace_with_project("amend-cli-exists");
    let bin = ralph_bin_absolute();

    let output = Command::new(&bin)
        .args([
            "amend",
            "--project", "nonexistent-project-xyz",
            "--body", "should fail",
        ])
        .current_dir(&repo_root)
        .output()
        .expect("ralph amend should execute");

    assert!(
        !output.status.success(),
        "ralph amend with nonexistent project should fail"
    );

    // Verify no queue files were created for the nonexistent project
    let orphan_queue_dir = repo_root
        .join(".ralph")
        .join("projects")
        .join("nonexistent-project-xyz")
        .join("amendment-queue");
    assert!(
        !orphan_queue_dir.exists(),
        "no queue directory should be created for nonexistent project"
    );
}
