Now I have all the information I need. Let me write the complete engineering specification.

---

# Engineering Specification: MCP Server for Ralph

## Summary

Add an MCP (Model Context Protocol) server to Ralph that exposes core orchestration operations as MCP tools over stdio transport using the JSON-RPC 2.0 protocol. This enables AI assistants and other MCP clients to programmatically create projects, start orchestration runs, check status, and retrieve results. The server is a new `src/mcp/` module invoked via `ralph mcp serve`, reusing existing library functions (`Workspace::discover`, `create_project`, `Orchestrator::run`, `load_project_state`, etc.) directly rather than shelling out.

## Acceptance Criteria

1. `ralph mcp serve` starts an MCP server reading JSON-RPC from stdin and writing responses to stdout.
2. The server correctly handles the MCP lifecycle: `initialize` (returns server info + capabilities), `notifications/initialized`, `tools/list`, and `tools/call`.
3. Nine tools are exposed via `tools/list`:
   - **project_new** — creates a project (params: `id`, `name`, `prompt_file` or `from_project`, optional `backend`)
   - **project_list** — lists all projects (no required params)
   - **project_show** — returns project metadata + state as JSON (params: optional `project`)
   - **run** — starts an orchestration run (params: optional `project`, `loops`, `until_review`, `until_complete`, `dry_run`, `backend`, role overrides, `skip_commit`), returns `OrchestrationResult` as JSON
   - **status** — returns current project status as JSON (params: optional `project`)
   - **history** — returns loop history as JSON (params: optional `project`, `verbose`)
   - **tail** — returns recent events as JSON array (params: optional `project`, `last`)
   - **quick_prd** — runs quick-prd pipeline (params: `idea`, optional `writer_backend`, `reviewer_backend`, `max_revisions`), returns spec path + content
   - **config_show** — returns effective config as JSON (params: optional `project`)
4. Each tool returns results as MCP `content` blocks with `type: "text"` containing JSON.
5. Errors from Ralph operations map to JSON-RPC error responses with appropriate codes and human-readable messages.
6. All logging/tracing goes to stderr; stdout is reserved exclusively for JSON-RPC messages.
7. The server processes requests sequentially (single-threaded request handling).
8. `cargo test` passes including new unit tests for the MCP module.
9. `tools/list` returns an `inputSchema` (JSON Schema) for each tool describing its parameters.

## Technical Approach

### Protocol Layer (`src/mcp/protocol.rs`)

Hand-rolled JSON-RPC 2.0 message types using serde. No external MCP SDK dependency — the protocol is simple enough to implement directly with `serde_json`:

```rust
// Request: {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{...}}
// Response: {"jsonrpc":"2.0","id":1,"result":{...}}
// Notification: {"jsonrpc":"2.0","method":"notifications/initialized"}
```

Types:
- `JsonRpcRequest { jsonrpc, id, method, params }`
- `JsonRpcResponse { jsonrpc, id, result?, error? }`
- `JsonRpcError { code, message, data? }`
- `id` is `Option<serde_json::Value>` (can be number, string, or null for notifications)

Standard error codes: `-32700` (parse error), `-32600` (invalid request), `-32601` (method not found), `-32602` (invalid params), `-32603` (internal error). Application errors use `-32000` range.

### Transport Layer (`src/mcp/transport.rs`)

Stdio transport: read lines from `tokio::io::BufReader<tokio::io::stdin()>`, write responses to `tokio::io::stdout()`. Each JSON-RPC message is one line (newline-delimited JSON).

### Server Core (`src/mcp/server.rs`)

```rust
pub struct McpServer {
    workspace: Workspace,
}

impl McpServer {
    pub async fn run_stdio(self) -> Result<()>;
}
```

The `run_stdio` loop:
1. Read a line from stdin
2. Parse as `JsonRpcRequest`
3. Dispatch by `method`:
   - `initialize` → return `ServerInfo` with name, version, capabilities (`tools: {}`)
   - `notifications/initialized` → no-op (no response for notifications)
   - `tools/list` → return tool definitions with `inputSchema`
   - `tools/call` → dispatch to tool handler based on `params.name`
4. Write response as one-line JSON to stdout
5. Loop until stdin EOF

### Tool Handlers (`src/mcp/handlers.rs`)

Each handler is a function that takes `serde_json::Value` params, performs the operation using existing library code, and returns `serde_json::Value`:

- **project_new**: Calls `Workspace::discover()` → validates params → calls `create_project()` → returns `{"status":"created","project_id":"..."}`.
- **project_list**: Calls `Workspace::discover()` → serializes `workspace.index.projects` as JSON array.
- **project_show**: Calls `Workspace::discover()` → `load_project_state()` → returns JSON with project ref + state (reuses the `--json` logic from `project show`).
- **run**: Calls `Workspace::discover()` → creates `Orchestrator::new(workspace)` → calls `orchestrator.run(RunOptions{...})` → returns `{"summary":"...","loop_number":N}`. Note: this is a blocking/long-running call; the MCP client must wait.
- **status**: Calls `Workspace::discover()` → `load_project_state()` → builds JSON matching the status command output.
- **history**: Calls `Workspace::discover()` → `load_project_state()` → serializes loops + completion attempts (reuses the `--json` path from history command).
- **tail**: Calls the existing `collect_all_events()` + `sort_events()` functions from `src/cli/tail.rs` (need to make them `pub(crate)`) → serializes events as JSON using `event_output()`. Non-follow mode only (no streaming).
- **quick_prd**: Constructs `QuickPrdPipeline` and calls `run()` → returns spec path + spec content.
- **config_show**: Calls `Workspace::discover()` → serializes global config or effective project config as JSON (reuses logic from `config show`).

### Visibility Changes

Several functions in existing CLI modules need `pub(crate)` visibility to be callable from the MCP handlers:
- `src/cli/tail.rs`: `collect_all_events()`, `sort_events()`, `event_output()` → make `pub(crate)`
- `src/cli/config.rs`: `execute_show` logic → extract a shared function that returns `serde_json::Value` instead of printing
- `src/cli/status.rs`: extract a helper that returns structured data
- `src/cli/history.rs`: the `--json` serialization logic → extract reusable function

Alternatively, the MCP handlers can directly call the underlying library functions (`load_project_state`, `Workspace::discover`, etc.) and build JSON themselves, which avoids coupling to CLI formatting code. **This is the preferred approach** — MCP handlers should build structured responses from library primitives, not reuse CLI print logic.

### CLI Integration (`src/cli/mod.rs`)

Add a new `Mcp` variant to the `Commands` enum:

```rust
#[derive(Debug, Subcommand)]
pub enum Commands {
    // ... existing commands ...
    Mcp(MpcArgs),
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    Serve,
}
```

The `Mcp(Serve)` handler initializes tracing to stderr only, discovers the workspace, and calls `McpServer::new(workspace).run_stdio().await`.

### Tracing Configuration

When running in MCP mode, tracing must be configured to write **only to stderr**. The current `main.rs` already uses `tracing_subscriber::fmt()` which defaults to stderr, so this should work without changes. However, any `println!` in tool handlers must be avoided — all output goes through the JSON-RPC response writer.

### Dependencies

No new crate dependencies needed. The existing `serde_json`, `serde`, `tokio` are sufficient for:
- JSON-RPC message parsing/serialization
- Async stdin/stdout I/O
- Tool handler async execution

## Files & Modules

| File | Action | Purpose |
|------|--------|---------|
| `src/mcp/mod.rs` | **Create** | Module declaration, re-exports `McpServer` |
| `src/mcp/protocol.rs` | **Create** | JSON-RPC 2.0 types: request, response, error, MCP-specific envelopes |
| `src/mcp/transport.rs` | **Create** | Stdio reader/writer with newline-delimited JSON framing |
| `src/mcp/server.rs` | **Create** | `McpServer` struct, request dispatch loop, `initialize`/`tools/list` handlers |
| `src/mcp/handlers.rs` | **Create** | Tool handler functions for all 9 tools |
| `src/mcp/schema.rs` | **Create** | JSON Schema definitions for each tool's `inputSchema` |
| `src/lib.rs` | **Edit** | Add `pub mod mcp;` |
| `src/cli/mod.rs` | **Edit** | Add `Mcp` command variant, `McpArgs`, `McpCommand` |
| `src/cli/mcp.rs` | **Create** | CLI handler: configure stderr-only tracing, start MCP server |
| `src/cli/tail.rs` | **Edit** | Make `collect_all_events`, `sort_events`, `event_output` `pub(crate)` |

## Testing Strategy

1. **Protocol unit tests** (`src/mcp/protocol.rs`):
   - Round-trip serialization/deserialization of JSON-RPC request/response/error
   - Parse valid requests, reject malformed JSON, reject missing `jsonrpc` field
   - Notification detection (no `id` field)

2. **Handler unit tests** (`src/mcp/handlers.rs`):
   - Each tool handler tested with a temp workspace (using `tempdir` + `Workspace::init`)
   - `project_new` → verify project appears in index
   - `project_list` → verify empty list, then list after creating projects
   - `project_show` → verify JSON contains expected fields after project creation
   - `status` / `history` / `tail` → test with a project that has pre-populated state
   - `config_show` → verify returns valid JSON matching workspace config
   - Error cases: missing required params, nonexistent project, workspace not found

3. **Server integration test** (`src/mcp/server.rs` or `tests/`):
   - Spawn `McpServer` with piped stdin/stdout
   - Send `initialize` → verify capabilities response
   - Send `tools/list` → verify all 9 tools present with valid `inputSchema`
   - Send `tools/call` for `project_list` → verify empty array
   - Send `tools/call` for `project_new` → verify success
   - Send `tools/call` with unknown tool → verify method-not-found error
   - Send malformed JSON → verify parse error response

4. **Schema validation tests** (`src/mcp/schema.rs`):
   - Verify each tool's `inputSchema` is valid JSON Schema
   - Verify required fields are marked as such

5. **No new external test dependencies** — use existing `tempfile` crate + `tokio::test`.

## Out of Scope

- **Streaming/SSE transport** — v1 is stdio only
- **Follow mode for tail** — the MCP `tail` tool returns a snapshot; no streaming events
- **Authentication/authorization** — the MCP server runs locally with the same permissions as the user
- **Concurrent request handling** — requests are processed sequentially
- **The `prd` (multi-stage interactive PRD) tool** — only `quick_prd` is exposed since the full PRD pipeline requires interactive user input
- **The `auto` command as a single tool** — clients can compose `quick_prd` → `project_new` → `run` themselves
- **The `rollback`, `init`, `validate` commands** — not exposed as MCP tools in v1
- **MCP resources or prompts** — only the `tools` capability is implemented
- **HTTP/WebSocket transport** — stdio only for v1
- **Progress notifications during `run`** — the `run` tool blocks until completion; progress monitoring can be done by polling `status` or `tail` from a separate request (sequential server means this requires a separate connection)