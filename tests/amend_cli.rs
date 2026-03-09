//! Integration test for `ralph amend` end-to-end flow.
//!
//! Verifies that enqueuing via the API, deserializing the produced file, and
//! draining the queue all work together correctly.

use std::fs;

use chrono::Utc;
use tempfile::TempDir;

use ralph::project::amendments::{
    drain_amendment_queue, enqueue_amendment, AmendmentPriority, AmendmentRequest, AmendmentSource,
};
use ralph::workspace::Workspace;

#[test]
fn enqueue_deserializes_and_drains_successfully() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");
    let workspace = Workspace::init(&workspace_root).expect("init workspace");

    // Create a minimal project directory (just needs to exist for enqueue)
    let project_id = "test-amend";
    let project_dir = workspace.project_dir(project_id);
    fs::create_dir_all(&project_dir).expect("create project dir");
    fs::write(project_dir.join("prompt.md"), "test prompt").expect("write prompt");

    let request = AmendmentRequest {
        id: "EXT-20260309120000".to_owned(),
        body: "Add retry logic to the API client".to_owned(),
        priority: AmendmentPriority::P1,
        source: AmendmentSource::Cli,
        source_detail: None,
        created_at: Utc::now(),
    };

    // Enqueue
    let queue_path = enqueue_amendment(&project_dir, &request).expect("enqueue should succeed");
    assert!(queue_path.exists(), "queue file should exist after enqueue");

    // Verify produced JSON deserializes correctly
    let raw = fs::read_to_string(&queue_path).expect("read queue file");
    let deserialized: AmendmentRequest =
        serde_json::from_str(&raw).expect("queue file should be valid JSON");
    assert_eq!(deserialized.id, request.id);
    assert_eq!(deserialized.body, request.body);
    assert_eq!(deserialized.priority, AmendmentPriority::P1);
    assert_eq!(deserialized.source, AmendmentSource::Cli);
    assert!(deserialized.source_detail.is_none());

    // Drain
    let drained = drain_amendment_queue(&project_dir).expect("drain should succeed");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].id, request.id);
    assert_eq!(drained[0].body, request.body);

    // Queue file should be removed after drain
    assert!(
        !queue_path.exists(),
        "queue file should be removed after drain"
    );
}

#[test]
fn multiple_enqueued_amendments_drain_in_order() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");
    let workspace = Workspace::init(&workspace_root).expect("init workspace");

    let project_id = "test-amend-multi";
    let project_dir = workspace.project_dir(project_id);
    fs::create_dir_all(&project_dir).expect("create project dir");
    fs::write(project_dir.join("prompt.md"), "test prompt").expect("write prompt");

    for i in 0..3 {
        let request = AmendmentRequest {
            id: format!("EXT-{i}"),
            body: format!("Amendment body {i}"),
            priority: AmendmentPriority::P2,
            source: AmendmentSource::Cli,
            source_detail: None,
            created_at: Utc::now(),
        };
        enqueue_amendment(&project_dir, &request).expect("enqueue should succeed");
    }

    let drained = drain_amendment_queue(&project_dir).expect("drain should succeed");
    assert_eq!(drained.len(), 3);

    // All amendments should be present (order is lexicographic by filename)
    let ids: Vec<&str> = drained.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"EXT-0"));
    assert!(ids.contains(&"EXT-1"));
    assert!(ids.contains(&"EXT-2"));
}
