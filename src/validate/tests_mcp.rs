use super::*;

use std::collections::HashSet;

use crate::validate::assertions::assert_exit_code;
use crate::validate::harness::RalphHarness;
use serde_json::{json, Value};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "mcp::initialize_returns_protocol_info",
            func: initialize_returns_protocol_info,
        },
        ConformanceTest {
            name: "mcp::ping_returns_empty_object",
            func: ping_returns_empty_object,
        },
        ConformanceTest {
            name: "mcp::tools_list_returns_all_tools",
            func: tools_list_returns_all_tools,
        },
        ConformanceTest {
            name: "mcp::unknown_method_returns_error",
            func: unknown_method_returns_error,
        },
        ConformanceTest {
            name: "mcp::notification_without_id_is_silent",
            func: notification_without_id_is_silent,
        },
        ConformanceTest {
            name: "mcp::notification_with_id_returns_method_not_found",
            func: notification_with_id_returns_method_not_found,
        },
        ConformanceTest {
            name: "mcp::malformed_json_returns_parse_error",
            func: malformed_json_returns_parse_error,
        },
        ConformanceTest {
            name: "mcp::tool_project_list_empty",
            func: tool_project_list_empty,
        },
        ConformanceTest {
            name: "mcp::tool_project_list_with_projects",
            func: tool_project_list_with_projects,
        },
        ConformanceTest {
            name: "mcp::tool_project_show_returns_state",
            func: tool_project_show_returns_state,
        },
        ConformanceTest {
            name: "mcp::tool_project_new_creates_project",
            func: tool_project_new_creates_project,
        },
        ConformanceTest {
            name: "mcp::tool_status_shows_project_info",
            func: tool_status_shows_project_info,
        },
        ConformanceTest {
            name: "mcp::tool_history_empty_project",
            func: tool_history_empty_project,
        },
        ConformanceTest {
            name: "mcp::tool_tail_empty_project",
            func: tool_tail_empty_project,
        },
        ConformanceTest {
            name: "mcp::tool_config_show_global",
            func: tool_config_show_global,
        },
        ConformanceTest {
            name: "mcp::tool_config_show_project",
            func: tool_config_show_project,
        },
        ConformanceTest {
            name: "mcp::tool_unknown_returns_error",
            func: tool_unknown_returns_error,
        },
        ConformanceTest {
            name: "mcp::tool_project_new_missing_id",
            func: tool_project_new_missing_id,
        },
        ConformanceTest {
            name: "mcp::tool_project_new_missing_prompt_source",
            func: tool_project_new_missing_prompt_source,
        },
        ConformanceTest {
            name: "mcp::tool_project_new_mutual_exclusion",
            func: tool_project_new_mutual_exclusion,
        },
        ConformanceTest {
            name: "mcp::tool_config_show_mutual_exclusion",
            func: tool_config_show_mutual_exclusion,
        },
        ConformanceTest {
            name: "mcp::tool_status_no_project",
            func: tool_status_no_project,
        },
    ]
}

fn initialize_returns_protocol_info(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_exchange(h, &jsonrpc_request(1, "initialize", json!({})));
        assert_eq!(responses.len(), 1, "expected one initialize response");

        let response = &responses[0];
        assert_eq!(response["jsonrpc"], json!("2.0"));
        assert_eq!(response["id"], json!(1));
        assert_eq!(response["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(response["result"]["serverInfo"]["name"], json!("ralph"));
        let version = response["result"]["serverInfo"]["version"]
            .as_str()
            .expect("serverInfo.version should be a string");
        assert!(
            !version.trim().is_empty(),
            "serverInfo.version should not be empty"
        );
        assert_eq!(response["result"]["capabilities"]["tools"], json!({}));
    })
}

fn ping_returns_empty_object(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_call(h, 2, "ping", json!({}));
        assert_eq!(
            responses.len(),
            2,
            "expected initialize + ping responses, got: {responses:?}"
        );
        assert_eq!(responses[1]["id"], json!(2));
        assert_eq!(responses[1]["result"], json!({}));
    })
}

fn tools_list_returns_all_tools(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_call(h, 2, "tools/list", json!({}));
        assert_eq!(
            responses.len(),
            2,
            "expected initialize + tools/list responses"
        );

        let tools = responses[1]["result"]["tools"]
            .as_array()
            .expect("tools/list result.tools should be an array");
        assert_eq!(tools.len(), 9, "expected 9 MCP tools");

        let mut names = HashSet::new();
        for tool in tools {
            let name = tool["name"]
                .as_str()
                .expect("tool.name should be a string")
                .to_owned();
            names.insert(name);
            assert!(
                tool["description"].as_str().is_some(),
                "tool.description should be a string: {tool}"
            );
            assert_eq!(
                tool["inputSchema"]["type"],
                json!("object"),
                "tool.inputSchema.type should be object: {tool}"
            );
        }

        let expected = HashSet::from([
            "project_new".to_owned(),
            "project_list".to_owned(),
            "project_show".to_owned(),
            "run".to_owned(),
            "status".to_owned(),
            "history".to_owned(),
            "tail".to_owned(),
            "quick_prd".to_owned(),
            "config_show".to_owned(),
        ]);
        assert_eq!(names, expected, "tools/list names should match");
    })
}

fn unknown_method_returns_error(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_call(h, 2, "bogus/method", json!({}));
        assert_eq!(
            responses.len(),
            2,
            "expected initialize + unknown method error responses"
        );
        assert_eq!(responses[1]["id"], json!(2));
        assert_jsonrpc_error(&responses[1], -32601);
    })
}

fn notification_without_id_is_silent(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let mut input = init_handshake();
        input.push_str(&jsonrpc_notification(
            "notifications/custom",
            json!({ "x": 1 }),
        ));
        input.push_str(&jsonrpc_request(3, "ping", json!({})));

        let responses = mcp_exchange(h, &input);
        assert_eq!(
            responses.len(),
            2,
            "notification without id should not emit a response"
        );
        assert_eq!(responses[0]["id"], json!(1));
        assert_eq!(responses[1]["id"], json!(3));
    })
}

fn notification_with_id_returns_method_not_found(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_call(h, 99, "notifications/custom", json!({}));
        assert_eq!(
            responses.len(),
            2,
            "expected initialize + method-not-found response"
        );
        assert_eq!(responses[1]["id"], json!(99));
        assert_jsonrpc_error(&responses[1], -32601);
    })
}

fn malformed_json_returns_parse_error(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let mut input = String::new();
        input.push_str("{not valid json}\n");
        input.push_str(&jsonrpc_request(1, "initialize", json!({})));

        let responses = mcp_exchange(h, &input);
        assert_eq!(
            responses.len(),
            2,
            "expected parse error then initialize response"
        );

        assert_eq!(responses[0]["id"], Value::Null);
        assert_jsonrpc_error(&responses[0], -32700);

        assert_eq!(responses[1]["id"], json!(1));
        assert_eq!(
            responses[1]["result"]["protocolVersion"],
            json!("2025-06-18")
        );
    })
}

fn tool_project_list_empty(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace failed");

        let responses = mcp_tool_call(h, "project_list", json!({}));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        let projects = result["projects"]
            .as_array()
            .expect("project_list result.projects should be an array");
        assert!(
            projects.is_empty(),
            "expected no projects in fresh workspace"
        );
    })
}

fn tool_project_list_with_projects(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace failed");
        h.create_project("mcp-list-a", "MCP List A", "MCP list prompt A")
            .expect("create first project failed");
        h.create_project("mcp-list-b", "MCP List B", "MCP list prompt B")
            .expect("create second project failed");

        let responses = mcp_tool_call(h, "project_list", json!({}));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        let projects = result["projects"]
            .as_array()
            .expect("project_list result.projects should be an array");
        assert_eq!(projects.len(), 2, "expected two projects");

        let ids: HashSet<String> = projects
            .iter()
            .map(|project| {
                assert!(
                    project.get("name").is_some(),
                    "project entry should include name: {project}"
                );
                assert!(
                    project.get("status").is_some(),
                    "project entry should include status: {project}"
                );
                project["id"]
                    .as_str()
                    .expect("project.id should be string")
                    .to_owned()
            })
            .collect();
        let expected = HashSet::from(["mcp-list-a".to_owned(), "mcp-list-b".to_owned()]);
        assert_eq!(ids, expected, "project IDs should match created projects");
    })
}

fn tool_project_show_returns_state(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_with_project(h, "mcp-show");

        let responses = mcp_tool_call(h, "project_show", json!({ "project": "mcp-show" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        assert!(
            result.get("project").is_some(),
            "project_show result should include project"
        );
        assert!(
            result.get("state").is_some(),
            "project_show result should include state"
        );
        assert_eq!(result["state"]["current_phase"], json!("planning"));
    })
}

fn tool_project_new_creates_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace failed");
        let prompt_file = h
            .write_mock_script("mcp-new-prompt.md", "# MCP prompt\n")
            .expect("failed to write prompt file");

        let responses = mcp_tool_call(
            h,
            "project_new",
            json!({
                "id": "mcp-new",
                "name": "MCP New",
                "prompt_file": prompt_file.to_string_lossy().into_owned(),
            }),
        );
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        assert_eq!(result["created"], json!(true));
        assert_eq!(result["project_id"], json!("mcp-new"));

        h.load_state("mcp-new")
            .expect("new project state should exist after project_new");
    })
}

fn tool_status_shows_project_info(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_with_project(h, "mcp-status");

        let responses = mcp_tool_call(h, "status", json!({ "project": "mcp-status" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        assert!(
            result.get("project_id").is_some(),
            "status should include project_id"
        );
        assert!(
            result.get("current_phase").is_some(),
            "status should include current_phase"
        );
        assert!(
            result.get("status").is_some(),
            "status should include status"
        );
    })
}

fn tool_history_empty_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_with_project(h, "mcp-hist");

        let responses = mcp_tool_call(h, "history", json!({ "project": "mcp-hist" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        let loops = result["loops"]
            .as_array()
            .expect("history result.loops should be an array");
        assert!(
            loops.is_empty(),
            "new project history loops should be empty"
        );
    })
}

fn tool_tail_empty_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_with_project(h, "mcp-tail");

        let responses = mcp_tool_call(h, "tail", json!({ "project": "mcp-tail" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        let events = result["events"]
            .as_array()
            .expect("tail result.events should be an array");
        assert!(events.is_empty(), "new project tail events should be empty");
    })
}

fn tool_config_show_global(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace failed");

        let responses = mcp_tool_call(h, "config_show", json!({ "global": true }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        assert_eq!(result["scope"], json!("global"));
        assert!(
            result["config"].is_object(),
            "global config should be an object"
        );
        assert!(
            result["config"]["workspace"].is_object(),
            "global config should include workspace"
        );
        assert!(
            result["config"]["backends"].is_object(),
            "global config should include backends"
        );
        assert!(
            result["config"]["workflow"].is_object(),
            "global config should include workflow"
        );
        assert!(
            result["config"]["templates"].is_object(),
            "global config should include templates"
        );
    })
}

fn tool_config_show_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_with_project(h, "mcp-cfg");

        let responses = mcp_tool_call(h, "config_show", json!({ "project": "mcp-cfg" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_eq!(responses[1]["result"]["isError"], json!(false));

        let result = extract_tool_result_json(&responses[1]);
        assert!(
            result["workflow"].is_object(),
            "project config should include workflow"
        );
        assert!(
            result["backends"].is_object(),
            "project config should include backends"
        );
        assert!(
            result["templates"].is_object(),
            "project config should include templates"
        );
    })
}

fn tool_unknown_returns_error(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace failed");

        let responses = mcp_tool_call(h, "nonexistent_tool", json!({}));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_tool_error(&responses[1]);

        let text = responses[1]["result"]["content"][0]["text"]
            .as_str()
            .expect("tool error content should include text");
        assert!(
            text.to_lowercase().contains("unknown tool"),
            "unknown tool error should mention 'unknown tool', got: {text}"
        );
    })
}

fn tool_project_new_missing_id(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_tool_call(h, "project_new", json!({ "name": "Test" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_tool_error(&responses[1]);
    })
}

fn tool_project_new_missing_prompt_source(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_tool_call(h, "project_new", json!({ "id": "test", "name": "Test" }));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_tool_error(&responses[1]);
    })
}

fn tool_project_new_mutual_exclusion(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_tool_call(
            h,
            "project_new",
            json!({
                "id": "test",
                "name": "Test",
                "prompt_file": "/tmp/prompt.md",
                "from_project": "parent",
            }),
        );
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_tool_error(&responses[1]);
    })
}

fn tool_config_show_mutual_exclusion(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let responses = mcp_tool_call(
            h,
            "config_show",
            json!({
                "global": true,
                "project": "some-proj",
            }),
        );
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_tool_error(&responses[1]);
    })
}

fn tool_status_no_project(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace failed");

        let responses = mcp_tool_call(h, "status", json!({}));
        assert_eq!(responses.len(), 2, "expected initialize + tool response");
        assert_tool_error(&responses[1]);
    })
}

fn jsonrpc_request(id: u64, method: &str, params: Value) -> String {
    format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
    )
}

fn jsonrpc_notification(method: &str, params: Value) -> String {
    format!(
        "{}\n",
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
    )
}

fn init_handshake() -> String {
    let mut input = String::new();
    input.push_str(&jsonrpc_request(1, "initialize", json!({})));
    input.push_str(&jsonrpc_notification(
        "notifications/initialized",
        json!({}),
    ));
    input
}

fn mcp_exchange(h: &RalphHarness, input: &str) -> Vec<Value> {
    let output = h
        .ralph_with_stdin(["mcp", "serve"], input)
        .expect("ralph mcp serve should execute");
    assert_exit_code(&output, 0);

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("MCP response line should be valid JSON"))
        .collect()
}

fn mcp_call(h: &RalphHarness, id: u64, method: &str, params: Value) -> Vec<Value> {
    let mut input = init_handshake();
    input.push_str(&jsonrpc_request(id, method, params));
    mcp_exchange(h, &input)
}

fn mcp_tool_call(h: &RalphHarness, tool_name: &str, arguments: Value) -> Vec<Value> {
    mcp_call(
        h,
        2,
        "tools/call",
        json!({
            "name": tool_name,
            "arguments": arguments,
        }),
    )
}

fn extract_tool_result_json(response: &Value) -> Value {
    let content = response["result"]["content"]
        .as_array()
        .expect("tool response result.content should be an array");
    let first = content
        .first()
        .expect("tool response content should not be empty");
    let text = first["text"]
        .as_str()
        .expect("tool response content[0].text should be a string");
    serde_json::from_str(text).expect("tool response text should contain JSON")
}

fn assert_jsonrpc_error(response: &Value, code: i64) {
    assert_eq!(
        response["error"]["code"].as_i64(),
        Some(code),
        "unexpected JSON-RPC error: {response}"
    );
}

fn assert_tool_error(response: &Value) {
    assert_eq!(
        response["result"]["isError"].as_bool(),
        Some(true),
        "expected tool error response with isError=true: {response}"
    );
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

fn setup_with_project(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init workspace failed");
    h.create_project(project_id, "MCP Conformance Project", "MCP suite prompt")
        .expect("create_project failed");
}
