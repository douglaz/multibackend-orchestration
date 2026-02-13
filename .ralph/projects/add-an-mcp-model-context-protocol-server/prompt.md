I now have all the information I need. Let me produce the revised specification.

# Engineering Specification: MCP Server for Ralph (Revised)

## Summary

Add an MCP (Model Context Protocol) server to ralph that exposes core orchestration operations as MCP tools over stdio transport. This enables AI assistants and other MCP clients to programmatically create projects, run orchestration loops, check status, and retrieve results — all through the standardized JSON-RPC 2.0 based MCP protocol. The server reuses existing library functions directly, avoiding any subprocess/shell-out overhead.

The server implements the MCP protocol version `2024-11-05` (the baseline widely supported by current clients), with strict lifecycle handling: protocol-version negotiation during `initialize`, rejection of non-ping requests before initialization completes, and proper `CallToolResult` formatting with `content` arrays and `isError` semantics.

## Acceptance Criteria

1. `ralph mcp serve` starts an MCP server on stdio (stdin/stdout) that speaks the MCP JSON-RPC 2.0 protocol.
2. The server implements correct MCP lifecycle:
   - Responds to `initialize` with server info, capabilities, and negotiated `protocolVersion`.
   - If the client requests an unsupported protocol version, responds with the server's supported version; the client decides whether to continue or disconnect.
   - Accepts `notifications/initialized` as a no-op notification (no response sent).
   - Rejects all requests except `initialize` and `ping` received before initialization completes, returning JSON-RPC error code `-32600` (invalid request).
   - Responds to `ping` at any point in the lifecycle.
3. The server advertises and handles the following tools via `tools/list` and `tools/call`:
   - `project_new` — create a new project (accepts: `id`, `name`, `prompt_file`, optional `starting_backend`)
   - `project_list` — list all projects in the workspace
   - `project_show` — show details of a specific project (accepts: `project_id`)
   - `run` — start an orchestration run in the background, return a job handle (accepts: `project_id`, optional `loops`, `until_review`, `until_complete`, `backend`, `planner_backend`, `implementer_backend`, `reviewer_backend`, `qa_backend`, `completer_backend`)
   - `status` — get current status of a project (accepts: `project_id`; optional `job_id` to query a specific run's completion state)
   - `history` — get loop history for a project (accepts: `project_id`, optional `verbose`, `json`)
   - `tail` — return recent events/log entries (accepts: `project_id`, optional `last` count)
   - `quick_prd` — run the quick-prd flow (accepts: `idea`, optional `writer_backend`, `reviewer_backend`, `max_revisions`)
   - `config_show` — show current ralph configuration
4. Each tool accepts JSON parameters as defined in its `inputSchema` (JSON Schema objects in `tools/list`). Parameter shapes mirror existing CLI argument structures and library APIs.
5. Each tool returns results wrapped in a `CallToolResult` object containing a `content` array of typed content blocks (primarily `{"type": "text", "text": "..."}` with JSON-serialized data) and an `isError` boolean.
6. Tool-level errors (project not found, validation failures, backend errors) are returned as `CallToolResult` with `isError: true` and a descriptive text content block — not as JSON-RPC error responses.
7. JSON-RPC error responses are reserved for protocol-level problems: parse errors (`-32700`), invalid requests (`-32600`), method not found (`-32601`), and invalid params (`-32602`).
8. The server runs until the client closes stdin or the stdin stream reaches EOF.
9. Concurrent tool calls are not required in v1 — sequential processing is acceptable, except that `run` spawns a background task and returns immediately.

## Technical Approach

### Protocol Layer

Use raw `serde_json` for JSON-RPC 2.0 message parsing rather than pulling in a full MCP SDK crate (the Rust MCP ecosystem is immature). Define the minimal message types:

```rust
// Incoming — id is Optional to support both requests and notifications
#[derive(Deserialize)]
struct JsonRpcMessage {
    jsonrpc: String,
    id: Option<Value>,       // Present for requests, absent for notifications
    method: String,
    params: Option<Value>,
}

// Outgoing (only sent for requests, never for notifications)
#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}
```

The `id: Option<Value>` design handles both requests (which have an `id`) and notifications (which do not). When `id` is `None`, the server must not send a response.

### MCP-Specific Types

```rust
// tools/call result — always wrapped in this structure
#[derive(Serialize)]
struct CallToolResult {
    content: Vec<ContentBlock>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    is_error: bool,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

// Tool definition for tools/list
#[derive(Serialize)]
struct ToolDefinition {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,  // JSON Schema object
}
```

### Server State and Lifecycle

The server maintains initialization state and a registry of background jobs:

```rust
struct McpServer {
    initialized: bool,
    workspace: Option<Workspace>,
    jobs: HashMap<String, JobEntry>,
}

struct JobEntry {
    job_id: String,
    project_id: String,
    started_at: DateTime<Utc>,
    handle: tokio::task::JoinHandle<Result<OrchestrationResult>>,
}
```

**Lifecycle enforcement:**
- Before `initialized == true`, only `initialize` and `ping` methods are accepted.
- Any other request received pre-initialization gets a JSON-RPC error response with code `-32600` ("Server not initialized").
- `notifications/initialized` sets `initialized = true` and produces no response (it is a notification).

### Server Loop

The server reads newline-delimited JSON from stdin asynchronously (using `tokio::io::BufReader` on stdin), dispatches to handlers, and writes JSON responses to stdout. An async loop is required because `run` spawns background orchestration tasks:

```
loop {
    line = read_line(stdin).await   // EOF → break
    message = parse(line)           // parse error → write error response, continue
    if message.id is None {
        handle_notification(message)  // no response written
        continue
    }
    if !initialized && method != "initialize" && method != "ping" {
        write error -32600
        continue
    }
    response = dispatch(message).await
    write_line(stdout, serialize(response))
    flush(stdout)
}
```

### MCP Handshake

**`initialize`** handler:
1. Extract client's `protocolVersion` from params.
2. Server supports `"2024-11-05"`. If the client requests this version, echo it back. If the client requests a different version, respond with `"2024-11-05"` as the server's supported version. The client then decides whether this is acceptable (per MCP spec, it may disconnect if unsatisfied).
3. Open the workspace via `Workspace::discover()` and cache it in server state.
4. Return:
```json
{
  "protocolVersion": "2024-11-05",
  "capabilities": { "tools": {} },
  "serverInfo": { "name": "ralph", "version": "<crate version>" }
}
```

**`notifications/initialized`** handler: Set `self.initialized = true`. No response (it's a notification — `id` is absent).

**`ping`** handler: Return `{}` (empty object result). Accepted at any lifecycle stage.

### Tool Dispatch

A central `dispatch_tool_call(name, params) -> CallToolResult` function matches on tool name and delegates to handler functions. Each handler:

1. Deserializes params into a tool-specific struct (with `#[serde(default)]` for optional fields).
2. Calls existing library functions.
3. On success: returns `CallToolResult { content: [Text { text: json_string }], is_error: false }`.
4. On failure: returns `CallToolResult { content: [Text { text: error_message }], is_error: true }`.

If the tool name is unknown, a JSON-RPC error response with code `-32602` ("Unknown tool: {name}") is returned at the protocol level, since `tools/call` itself succeeded but the params referenced a nonexistent tool.

### Integration with Existing Code

Key reuse points (verified against current codebase):

| Tool | Library entry point | Module |
|---|---|---|
| `project_new` | `create_project(workspace, CreateProjectOptions { id, name, source: PromptSource::File(path), starting_backend })` | `src/project/lifecycle.rs` |
| `project_list` | `workspace.index.projects` (Vec<ProjectRef>) | `src/workspace/index.rs` |
| `project_show` | `workspace.index.get_project(id)` + `load_project_state(project_dir)` | `src/workspace/index.rs`, `src/project/lifecycle.rs` |
| `run` | `Orchestrator::new(workspace).run(RunOptions { ... }).await` | `src/workflow/orchestrator.rs` |
| `status` | `load_project_state(project_dir)` + `workspace.index.get_project(id)` | `src/project/lifecycle.rs` |
| `history` | `load_project_state(project_dir)` — iterate `feature_loops` + `completion_attempts` | `src/project/state.rs` |
| `tail` | Reuse event-collection logic from `src/cli/tail.rs` (extract into shared function) | `src/cli/tail.rs` |
| `quick_prd` | `QuickPrdPipeline::new(writer, reviewer, options).run().await` | `src/prd/quick.rs` |
| `config_show` | `GlobalConfig::load(path)` via `workspace.config` | `src/config/global.rs` |

### Non-Blocking `run` with Job Tracking

The `run` tool cannot block the server loop because `Orchestrator::run().await` may take minutes or hours. Instead:

1. `run` handler validates params, clones the `Workspace`, constructs `RunOptions`, and spawns the orchestration into a `tokio::spawn` task.
2. A unique `job_id` (UUID v4) is generated and stored in `McpServer.jobs` alongside the `JoinHandle`.
3. The tool returns immediately with `{ "job_id": "...", "project_id": "...", "state": "running" }`.
4. The `status` tool accepts an optional `job_id` param. When provided, it checks the `JoinHandle`:
   - If not finished: returns `{ "job_state": "running", ... }` alongside normal project state.
   - If finished (success): returns `{ "job_state": "completed", "result": { "summary": "...", "loop_number": N } }`.
   - If finished (error): returns `{ "job_state": "failed", "error": "..." }`.
5. Only one run per project is allowed concurrently. If a `run` is requested for a project that already has an in-flight job, the tool returns `isError: true` with a message indicating the existing job_id.
6. Completed/failed jobs remain in the map until the server shuts down (no explicit cleanup needed for a session-scoped stdio server).

### Error Mapping

**Protocol-level errors** (JSON-RPC error responses):
| Condition | Code | Message |
|---|---|---|
| Unparseable JSON | `-32700` | Parse error |
| Request before init (except ping) | `-32600` | Server not initialized |
| Unknown method (not initialize/ping/tools/*) | `-32601` | Method not found |
| Unknown tool name in tools/call | `-32602` | Unknown tool: {name} |
| Missing/malformed required params | `-32602` | Invalid params: {detail} |

**Tool-level errors** (CallToolResult with `isError: true`):
| Condition | Text content |
|---|---|
| `RalphError::ProjectNotFound` | "Project not found: {id}" |
| `RalphError::Validation` | "Validation error: {msg}" |
| `RalphError::WorkspaceNotFound` | "Workspace not found" |
| `RalphError::ActiveProjectNotSet` | "No active project set" |
| `RalphError::BackendUnavailable` | "Backend unavailable: {name}" |
| `RalphError::StateLocked` | "Project state is locked: {id}" |
| Duplicate run for same project | "Project {id} already has an active run (job_id: {jid})" |
| Any other `RalphError` | Display string of the error |

### Stdout Protocol Purity

The MCP stdio transport requires that stdout carries only JSON-RPC messages. All diagnostic output, log messages, and human-readable text from ralph internals must be suppressed or redirected to stderr. The MCP server loop will:

1. Redirect any `println!` or `eprintln!` in handler code to stderr (handlers should use `eprintln!` for diagnostics if needed).
2. Ensure library functions called by handlers do not write to stdout. Where existing code does (e.g., `status::execute` prints to stdout), the MCP handlers call lower-level functions (e.g., `load_project_state`) instead of the CLI `execute` functions.
3. Capture and discard or redirect any stdout output from spawned orchestrator tasks (the orchestrator runs in a background tokio task — its stdout writes go to the process stdout, so we may need to suppress them via a `tracing` subscriber or by not calling print-heavy code paths).

## Files & Modules

| File | Purpose |
|---|---|
| `src/mcp/mod.rs` | Module root, re-exports `McpServer` |
| `src/mcp/server.rs` | `McpServer` struct, async stdio read/write loop, lifecycle state machine, JSON-RPC parsing, method dispatch |
| `src/mcp/types.rs` | `JsonRpcMessage`, `JsonRpcResponse`, `JsonRpcError`, `CallToolResult`, `ContentBlock`, `ToolDefinition`, server capabilities structs |
| `src/mcp/tools.rs` | Tool registry: `ToolDefinition` list with names, descriptions, `inputSchema` JSON Schemas; `dispatch_tool_call()` routing |
| `src/mcp/handlers.rs` | Individual tool handler functions (`handle_project_new`, `handle_run`, `handle_status`, `handle_tail`, `handle_quick_prd`, etc.) |
| `src/mcp/jobs.rs` | `JobEntry` struct, `JobMap` type alias, job lifecycle helpers (spawn, poll, check duplicate) |
| `src/cli/mod.rs` | Add `Mcp` variant to `Commands` enum with `serve` subcommand |
| `src/cli/mcp.rs` | CLI handler: parse args, construct `McpServer`, call `server.run().await` |
| `src/main.rs` | Wire up `Commands::Mcp` to `cli::mcp::execute` |
| `src/lib.rs` | Add `pub mod mcp;` |

## Testing Strategy

### Unit Tests

1. **`src/mcp/types.rs` tests** — Verify JSON-RPC serialization/deserialization round-trips:
   - Request with `id` (normal request) deserializes correctly.
   - Notification without `id` (`notifications/initialized`) deserializes with `id: None`.
   - `CallToolResult` serializes with `content` array and `isError` field.
   - `JsonRpcError` serializes with correct code/message/data structure.
   - Missing `params` field deserializes as `None`.

2. **`src/mcp/tools.rs` tests** — Verify tool registry:
   - `tools/list` returns all 9 tools.
   - Each tool has a non-empty description and valid JSON Schema `inputSchema`.
   - `dispatch_tool_call` routes known tool names to handlers and returns error for unknown names.

### Integration Tests

3. **`tests/mcp_server.rs`** — Spawn `ralph mcp serve` as a child process, pipe JSON-RPC messages to stdin, read responses from stdout:

   **Lifecycle tests:**
   - `initialize` with matching protocol version returns capabilities and `protocolVersion: "2024-11-05"`.
   - `initialize` with unsupported protocol version returns server's supported version `"2024-11-05"`.
   - Request (other than ping) sent before `initialize` returns error `-32600`.
   - `ping` before `initialize` succeeds.
   - `ping` after `initialize` succeeds.
   - `notifications/initialized` produces no response on stdout.

   **Tool listing:**
   - `tools/list` after initialization returns all 9 tools with valid JSON Schema `inputSchema` objects.

   **Tool invocation (using a temp workspace):**
   - `tools/call` for `config_show` returns valid config JSON in a `CallToolResult`.
   - `tools/call` for `project_new` creates a project; subsequent `project_list` includes it.
   - `tools/call` for `project_show` with valid id returns project details.
   - `tools/call` for `project_show` with nonexistent id returns `isError: true`.
   - `tools/call` for `run` returns a `job_id` and `"state": "running"`.
   - `tools/call` for `status` with `job_id` returns job state.
   - `tools/call` for `history` returns loop history array.
   - `tools/call` for `tail` returns recent events array.
   - `tools/call` for `quick_prd` invocation (may require mock backends; test at least param validation).

   **Error handling:**
   - Unknown tool name returns JSON-RPC error `-32602`.
   - Malformed params return `isError: true` in `CallToolResult` or `-32602` at the protocol level.
   - Malformed JSON input returns parse error `-32700`.

   **Protocol purity:**
   - Verify stdout contains only valid JSON-RPC messages (no interleaved log output or human-readable text).

   **Duplicate run prevention:**
   - Second `run` for the same project while first is active returns `isError: true` with existing job_id.

### Manual Validation

4. Confirm interop with an MCP client (e.g., Claude Code's MCP integration or `mcp-cli`) by adding ralph as a stdio MCP server and invoking tools.

## Out of Scope

- **MCP Resources and Prompts** — Only the Tools capability is implemented. Resources (file-like content) and Prompts (templated interactions) are deferred.
- **SSE/HTTP transport** — Only stdio transport is supported. Network transports can be added later.
- **Streaming/progress notifications** — Tools return a single result. Progress streaming for long-running operations (like `run`) is deferred. Clients poll `status` with the `job_id` instead.
- **Authentication/authorization** — Stdio transport implies local trust; no auth layer is needed.
- **Concurrent/batch requests** — Requests are processed sequentially. JSON-RPC batch arrays (JSON arrays of requests) are not supported in v1. If a batch array is received, the server responds with JSON-RPC error `-32600` ("Batch requests are not supported").
- **Tool annotations** — MCP tool annotations (`readOnlyHint`, `destructiveHint`, etc.) are deferred to a follow-up.
- **Subscription/watch** — No event subscription mechanism; clients poll `status` and `tail` instead.
- **Protocol versions beyond `2024-11-05`** — The server advertises `2024-11-05` only. Support for newer MCP protocol versions (e.g., `2025-03-26` with structured tool outputs) is deferred.
- **Lookup by project name** — All tools accept `project_id` (the unique identifier), not project name. Name-based lookup is deferred to avoid ambiguity (names are not unique).