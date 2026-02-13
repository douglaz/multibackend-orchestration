//! End-to-end integration tests for MCP tool handlers.
//!
//! Each test creates a temporary Ralph workspace and sends JSON-RPC
//! `tools/call` messages through a real `McpServer` to verify that
//! handlers return proper `CallToolResult` values.
//!
//! Because `Workspace::discover()` walks up from `current_dir()`,
//! these tests must change the working directory. To prevent races
//! in the multi-threaded test runner, all tests serialize through
//! a static mutex.

use std::fs;
use std::sync::Mutex;

use ralph::mcp::protocol::CallToolResult;
use ralph::mcp::server::McpServer;
use ralph::mcp::transport::StdioTransport;
use ralph::workspace::Workspace;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

/// Global mutex to serialize tests that change the working directory.
static CWD_MUTEX: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Bootstrap: initialize + tools/call, then read all responses.
/// Acquires CWD_MUTEX for the duration of the cwd change + server run.
async fn run_tool_call(
    tool_name: &str,
    arguments: Value,
    workspace_root: &std::path::Path,
) -> Vec<Value> {
    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let notif_msg = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let call_msg = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    let input = format!(
        "{}\n{}\n{}\n",
        serde_json::to_string(&init_msg).unwrap(),
        serde_json::to_string(&notif_msg).unwrap(),
        serde_json::to_string(&call_msg).unwrap(),
    );

    let (mut request_client, request_server) = tokio::io::duplex(64 * 1024);
    let (response_server, mut response_client) = tokio::io::duplex(64 * 1024);

    request_client
        .write_all(input.as_bytes())
        .await
        .expect("write input");
    request_client.shutdown().await.expect("shutdown writer");

    // Hold the lock while we change cwd, run the server, and restore.
    let _guard = CWD_MUTEX.lock().expect("acquire cwd mutex");
    let original_dir = std::env::current_dir().expect("get cwd");
    let parent = workspace_root.parent().expect("workspace has parent");
    std::env::set_current_dir(parent).expect("cd to workspace parent");

    let transport = StdioTransport::new(BufReader::new(request_server), response_server);
    let mut server = McpServer::new(transport);
    server.run().await.expect("server run");
    drop(server);

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    drop(_guard);

    let mut raw_output = String::new();
    response_client
        .read_to_string(&mut raw_output)
        .await
        .expect("read output");

    raw_output
        .lines()
        .map(|line| serde_json::from_str(line).expect("response is valid JSON"))
        .collect()
}

/// Extract the CallToolResult from the tools/call response (id=2),
/// asserting no JSON-RPC-level error.
fn extract_tool_result(responses: &[Value]) -> CallToolResult {
    let resp = responses
        .iter()
        .find(|r| r["id"] == 2)
        .expect("should have id=2 response");
    assert!(
        resp.get("error").is_none() || resp["error"].is_null(),
        "tools/call should not return JSON-RPC error: {resp}"
    );
    let result = &resp["result"];
    serde_json::from_value(result.clone()).expect("result should be a valid CallToolResult")
}

/// Extract CallToolResult from id=2 (even if isError is true).
fn extract_tool_result_any(responses: &[Value]) -> CallToolResult {
    let resp = responses
        .iter()
        .find(|r| r["id"] == 2)
        .expect("should have id=2 response");
    serde_json::from_value(resp["result"].clone()).expect("should be CallToolResult")
}

/// Parse the inner JSON payload from CallToolResult text content.
fn parse_inner(ctr: &CallToolResult) -> Value {
    let text = &ctr.content[0].text;
    serde_json::from_str(text).expect("inner text should be valid JSON")
}

/// Create a minimal workspace with one project.
fn create_test_workspace() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");

    let workspace = Workspace::init(&workspace_root).expect("init workspace");

    let prompt_path = temp.path().join("prompt.md");
    fs::write(&prompt_path, "# Test Prompt\nBuild a test feature.").expect("write prompt");

    use ralph::project::lifecycle::{create_project, CreateProjectOptions, PromptSource};
    let mut ws = workspace;
    create_project(
        &mut ws,
        CreateProjectOptions {
            id: "test-project".to_owned(),
            name: "Test Project".to_owned(),
            source: PromptSource::File(prompt_path),
            starting_backend: None,
        },
    )
    .expect("create project");

    (temp, workspace_root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_project_list_returns_call_tool_result_with_projects() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("project_list", json!({}), &ws_root).await;

    assert!(responses.len() >= 2);

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "project_list should succeed");

    let inner = parse_inner(&ctr);
    let projects = inner["projects"].as_array().expect("projects array");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["id"], "test-project");
    assert_eq!(projects[0]["name"], "Test Project");
}

#[tokio::test]
async fn e2e_project_show_returns_project_state() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "project_show",
        json!({ "project": "test-project" }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "project_show should succeed");

    let inner = parse_inner(&ctr);
    assert!(inner["project"].is_object());
    assert!(inner["state"].is_object());
    assert_eq!(inner["state"]["project_id"], "test-project");
}

#[tokio::test]
async fn e2e_project_show_unknown_project_returns_is_error_true() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "project_show",
        json!({ "project": "nonexistent" }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result_any(&responses);
    assert!(ctr.is_error, "nonexistent project should return isError");
    assert!(ctr.content[0].text.contains("not found"));
}

#[tokio::test]
async fn e2e_status_returns_structured_state() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("status", json!({ "project": "test-project" }), &ws_root).await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "status should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["project_id"], "test-project");
    assert_eq!(inner["project_name"], "Test Project");
    assert!(inner["status"].is_string());
}

#[tokio::test]
async fn e2e_history_returns_empty_loops() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("history", json!({ "project": "test-project" }), &ws_root).await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "history should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["project_id"], "test-project");
    let loops = inner["loops"].as_array().expect("loops array");
    assert!(loops.is_empty(), "new project has no loops");
}

#[tokio::test]
async fn e2e_tail_returns_events_array() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("tail", json!({ "project": "test-project" }), &ws_root).await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "tail should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["project_id"], "test-project");
    assert!(inner["events"].is_array());
}

#[tokio::test]
async fn e2e_config_show_global_returns_config() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("config_show", json!({ "global": true }), &ws_root).await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "config_show global should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["scope"], "global");
    assert!(inner["config"].is_object());
}

#[tokio::test]
async fn e2e_config_show_project_returns_effective_config() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "config_show",
        json!({ "project": "test-project" }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "config_show project should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["scope"]["type"], "project");
    assert_eq!(inner["scope"]["project"], "test-project");
}

#[tokio::test]
async fn e2e_config_show_mutual_exclusion_returns_tool_error() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "config_show",
        json!({ "global": true, "project": "test-project" }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result_any(&responses);
    assert!(ctr.is_error);
    assert!(ctr.content[0].text.contains("mutually exclusive"));
}

#[tokio::test]
async fn e2e_quick_prd_dry_run_succeeds() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "quick_prd",
        json!({ "idea": "add retry logic", "dry_run": true }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "quick_prd dry_run should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["dry_run"], true);
    assert!(inner["prompt"]
        .as_str()
        .unwrap()
        .contains("add retry logic"));
}

#[tokio::test]
async fn e2e_project_new_creates_project() {
    let temp = TempDir::new().expect("temp dir");
    let workspace_root = temp.path().join(".ralph");
    Workspace::init(&workspace_root).expect("init workspace");

    let prompt_path = temp.path().join("new-prompt.md");
    fs::write(&prompt_path, "# New Feature\nBuild something.").expect("write prompt");

    let responses = run_tool_call(
        "project_new",
        json!({
            "id": "new-proj",
            "name": "New Project",
            "prompt_file": prompt_path.to_string_lossy()
        }),
        &workspace_root,
    )
    .await;

    let ctr = extract_tool_result(&responses);
    assert!(!ctr.is_error, "project_new should succeed");

    let inner = parse_inner(&ctr);
    assert_eq!(inner["created"], true);
    assert_eq!(inner["project_id"], "new-proj");
}

#[tokio::test]
async fn e2e_unknown_tool_returns_tool_error() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("nonexistent_tool", json!({}), &ws_root).await;

    let ctr = extract_tool_result_any(&responses);
    assert!(ctr.is_error, "unknown tool should return isError");
    assert!(ctr.content[0].text.contains("unknown tool"));
}

#[tokio::test]
async fn e2e_missing_required_argument_returns_tool_error() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call("project_new", json!({}), &ws_root).await;

    let ctr = extract_tool_result_any(&responses);
    assert!(ctr.is_error);
    assert!(ctr.content[0].text.contains("missing required argument"));
}

#[tokio::test]
async fn e2e_extra_arguments_are_silently_ignored() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "quick_prd",
        json!({
            "idea": "test postel",
            "dry_run": true,
            "unknown_key": "should be ignored",
            "another_unknown": 999
        }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result(&responses);
    assert!(
        !ctr.is_error,
        "extra arguments should not cause errors (Postel's law)"
    );
}

#[tokio::test]
async fn e2e_run_invalid_on_prompt_change_returns_tool_error() {
    let (_temp, ws_root) = create_test_workspace();
    let responses = run_tool_call(
        "run",
        json!({
            "project": "test-project",
            "on_prompt_change": "invalid-value"
        }),
        &ws_root,
    )
    .await;

    let ctr = extract_tool_result_any(&responses);
    assert!(ctr.is_error);
    assert!(ctr.content[0].text.contains("invalid on_prompt_change"));
}
