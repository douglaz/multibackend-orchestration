# Engineering Specification: MCP Server for Ralph

## Summary

Add an MCP (Model Context Protocol) server to Ralph that exposes core orchestration operations as MCP tools over stdio transport using the JSON-RPC 2.0 protocol. This enables AI assistants and other MCP clients to programmatically create projects, start orchestration runs, check status, and retrieve results. The server is a new `src/mcp/` module invoked via `ralph mcp serve`, reusing existing library functions (`Workspace::discover`, `create_project`, `Orchestrator::run`, `load_project_state`, etc.) directly rather than shelling out.

## Acceptance Criteria

1. `ralph mcp serve` starts an MCP server reading JSON-RPC from stdin and writing responses to stdout.
2. The server correctly handles the MCP lifecycle: `initialize` (returns server info + capabilities with `protocolVersion`), `notifications/initialized` (no-op), `tools/list`, `tools/call`, and `ping`.
3. Nine tools are exposed via `tools/list`:
   - **project_new** — creates a project (params: `id` required, `name` required, exactly one of `prompt_file` or `from_project` required, optional `backend`)
   - **project_list** — lists all projects (no required params)
   - **project_show** — returns project metadata + state as JSON (params: optional `project`)
   - **run** — starts an orchestration run (params: optional `project`, `loops`, `until_review`, `until_complete`, `dry_run`, `backend`, `planner_backend`, `implementer_backend`, `reviewer_backend`, `qa_backend`, `completer_backend`, `on_prompt_change`, `skip_commit`), returns `OrchestrationResult` as JSON. **Note:** `tmux`/`no_tmux` are omitted — MCP is a non-interactive stdio channel where tmux is not applicable.
   - **status** — returns current project status as JSON (params: optional `project`)
   - **history** — returns loop history as JSON (params: optional `project`, `verbose`)
   - **tail** — returns recent events as JSON array (params: optional `project`, `last`)
   - **quick_prd** — runs quick-prd pipeline (params: `idea` required, optional `writer_backend`, `reviewer_backend`, `max_revisions`, `dry_run`), returns spec path + content. **Note:** `interactive`/`non_interactive` are omitted — MCP is always non-interactive.
   - **config_show** — returns effective config as JSON (params: optional `project`, optional `global`; `project` and `global` are mutually exclusive)
4. Each tool returns results as MCP `CallToolResult` with `content` blocks of `type: "text"` containing JSON and `isError: false` on success.
5. Ralph operation failures (project not found, validation errors, orchestration failures) return a normal `CallToolResult` response with `isError: true` and the error message as a text content block. JSON-RPC `error` responses are reserved for protocol-level failures only: parse errors (`-32700`), invalid requests (`-32600`), unknown methods (`-32601`), and internal server errors (`-32603`).
6. All logging/tracing goes to stderr; stdout is reserved exclusively for JSON-RPC messages.
7. The server processes requests sequentially (single-threaded request handling).
8. `cargo test` passes including new unit tests for the MCP module.
9. `tools/list` returns an `inputSchema` (JSON Schema) for each tool describing its parameters, including `required` arrays and mutual exclusivity constraints documented in `description` fields.
10. The `initialize` response includes `protocolVersion: "2025-06-18"`, `serverInfo` with name `"ralph"` and version from `env!("CARGO_PKG_VERSION")`, and `capabilities: { tools: {} }`.
11. Unknown JSON-RPC methods (other than `initialize`, `notifications/initialized`, `tools/list`, `tools/call`, `ping`) receive a `-32601` method-not-found error. Unknown notifications (methods starting with `notifications/`) are silently ignored.
12. The server re-discovers the workspace on each `tools/call` invocation (no cached workspace). This ensures mutations (project creation, config changes, run completions) are always reflected.

## Technical Approach

### Protocol Layer (`src/mcp/protocol.rs`)

Hand-rolled JSON-RPC 2.0 message types using serde. No external MCP SDK dependency — the protocol is simple enough to implement directly with `serde_json`:

```rust
// Request/notification from stdin:
// {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{...}}
// {"jsonrpc":"2.0","method":"notifications/initialized"}

// Response to stdout:
// {"jsonrpc":"2.0","id":1,"result":{...}}
// {"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"..."}}
```

Types:
- `JsonRpcMessage` — deserialized from stdin; contains `jsonrpc`, `id: Option<Value>`, `method: Option<String>`, `params: Option<Value>`, `result: Option<Value>`, `error: Option<JsonRpcError>`. If `id` is present it's a request; if absent it's a notification.
- `JsonRpcResponse` — serialized to stdout: `{ jsonrpc: "2.0", id: Value, result?: Value, error?: JsonRpcError }`.
- `JsonRpcError { code: i32, message: String, data: Option<Value> }`
- `CallToolResult { content: Vec<ContentBlock>, isError: Option<bool> }` — the MCP tool result envelope.
- `ContentBlock { type: String, text: String }` — always `type: "text"` in v1.

Standard JSON-RPC error codes: `-32700` (parse error), `-32600` (invalid request), `-32601` (method not found), `-32603` (internal error). These are used **only** for protocol-level failures. Tool execution errors use `CallToolResult` with `isError: true`.

### Transport Layer (`src/mcp/transport.rs`)

MCP stdio transport uses **newline-delimited JSON** (per the MCP spec: "Messages are delimited by newlines, and MUST NOT contain embedded newlines"). This is NOT LSP-style Content-Length framing.

The transport is parameterized over reader/writer traits to enable testing without binding to process stdin/stdout:

```rust
pub struct StdioTransport<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> {
    reader: R,
    writer: W,
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> StdioTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self;
    pub async fn read_message(&mut self) -> Result<Option<JsonRpcMessage>>;
    pub async fn write_message(&mut self, response: &JsonRpcResponse) -> Result<()>;
}
```

- `read_message()`: reads one line via `read_line()`, returns `None` on EOF. Returns a parse-error `JsonRpcResponse` written directly to the writer if the line is not valid JSON.
- `write_message()`: serializes to a single-line JSON string (no embedded newlines possible since serde_json default serialization is single-line), appends `\n`, and flushes.

Production instantiation uses `tokio::io::BufReader::new(tokio::io::stdin())` and `tokio::io::stdout()`.

### Server Core (`src/mcp/server.rs`)

```rust
pub struct McpServer<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> {
    transport: StdioTransport<R, W>,
    initialized: bool,
}

impl McpServer<tokio::io::BufReader<tokio::io::Stdin>, tokio::io::Stdout> {
    /// Create a server bound to process stdin/stdout.
    pub fn stdio() -> Self;
}

impl<R: AsyncBufRead + Unpin, W: AsyncWrite + Unpin> McpServer<R, W> {
    /// Create a server with injected reader/writer (for testing).
    pub fn new(transport: StdioTransport<R, W>) -> Self;

    /// Run the server loop until EOF.
    pub async fn run(&mut self) -> Result<()>;
}
```

The `run()` loop:
1. Read a message from transport
2. If `None` (EOF), return `Ok(())`
3. Dispatch by `method`:
   - `"initialize"` → return `InitializeResult` with `protocolVersion: "2025-06-18"`, `serverInfo: { name: "ralph", version: env!("CARGO_PKG_VERSION") }`, `capabilities: { tools: {} }`. Set `self.initialized = true`.
   - `"notifications/initialized"` → no-op (notification, no response)
   - `"ping"` → return `{}` (empty result object)
   - Any `"notifications/*"` → silently ignored (no response for unknown notifications)
   - `"tools/list"` → return tool definitions with `inputSchema`
   - `"tools/call"` → extract `params.name` and `params.arguments`, dispatch to tool handler, return `CallToolResult`
   - Any other method with an `id` → return `-32601` method-not-found error
4. Write response (if any) to transport
5. Loop

### Tool Handlers (`src/mcp/handlers.rs`)

Each handler is an async function that takes `serde_json::Value` arguments and returns `Result<serde_json::Value, String>`. The `Ok` variant is wrapped in `CallToolResult { isError: false }`, the `Err` variant in `CallToolResult { isError: true }`.

Handler implementations discover the workspace fresh on each call:

- **project_new**: Calls `Workspace::discover()` → validates params (exactly one of `prompt_file`/`from_project` must be present) → calls `create_project()` → returns `{"status":"created","project_id":"..."}`.
- **project_list**: Calls `Workspace::discover()` → serializes `workspace.index.projects` as JSON array.
- **project_show**: Calls `Workspace::discover()` → resolves project (explicit or active) → `load_project_state()` → returns JSON with project ref + state.
- **run**: Calls `Workspace::discover()` → creates `Orchestrator::new(workspace)` → calls `orchestrator.run(RunOptions{..., tmux: None, ...})` (tmux always disabled for MCP) → returns serialized `OrchestrationResult`. Note: this is a blocking/long-running call; the MCP client must wait.
- **status**: Calls `Workspace::discover()` → resolves project → `load_project_state()` → builds a structured JSON object with project status, current loop/phase, loop details, and completion state.
- **history**: Calls `Workspace::discover()` → resolves project → `load_project_state()` → builds JSON array of loops + completion attempts sorted by loop number (same structure as the existing `--json` output in history command).
- **tail**: Calls the tail event collection logic (see Tail Reuse Strategy below) → returns events as JSON array.
- **quick_prd**: Resolves backends from workspace config → constructs `QuickPrdPipeline` with `QuickPrdOptions { ..., dry_run: args.dry_run.unwrap_or(false) }` → calls `run()` → returns `{"spec_path": "...", "summary": "...", "revision_count": N, "approved": bool}`. Spec content is **not** included inline (it can be large); clients should read the file at `spec_path`.
- **config_show**: Calls `Workspace::discover()` → resolves scope (global vs project, mutually exclusive) → builds JSON matching the config show command's output structure.

### Tail Reuse Strategy

Rather than making private types in `src/cli/tail.rs` public (which would require exposing `TailEvent`, `TailEventKind`, `TailEventOutput`, `PhaseSnapshot` and several helper functions), the MCP tail handler re-implements event collection at the library level:

Create a new `src/mcp/tail_events.rs` module that:
1. Reads `load_project_state()` to generate state transition events (loop starts, completions, git commits, completion verdicts) — same logic as `collect_state_events` in `src/cli/tail.rs`.
2. Scans the project's `loops/` directory for `.md` artifact files, parses frontmatter using the same `split_frontmatter` + `parse_artifact_filename_timestamp` approach.
3. Sorts events by timestamp.
4. Serializes directly into `Vec<serde_json::Value>` using the same JSON shape as `TailEventOutput` (fields: `project_id`, `event_type`, `timestamp`, `path`, `loop_number`, etc.).

This duplicates ~150 lines of collection/sorting logic but avoids coupling to CLI-internal types and keeps the `tail.rs` module's encapsulation intact. The JSON output schema matches exactly so clients see the same data regardless of whether they use `ralph tail --json` or the MCP `tail` tool.

### CLI Integration (`src/cli/mod.rs`)

Add a new `Mcp` variant to the `Commands` enum:

```rust
#[derive(Debug, Subcommand)]
pub enum Commands {
    // ... existing commands ...
    /// MCP server commands
    Mcp(McpArgs),
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Start the MCP JSON-RPC server on stdio
    Serve,
}
```

The dispatch in `run()`:
```rust
Commands::Mcp(args) => match args.command {
    McpCommand::Serve => crate::mcp::serve().await,
},
```

### Tracing Configuration

When running in MCP mode, tracing must write **only to stderr**. The current `main.rs` already uses `tracing_subscriber::fmt()` which defaults to stderr, so this requires no changes. The MCP server code must never use `println!` — all stdout output goes through `StdioTransport::write_message()`.

### Dependencies

No new crate dependencies needed. The existing `serde_json`, `serde`, `tokio` (with `features = ["full"]`) are sufficient for:
- JSON-RPC message parsing/serialization
- Async stdin/stdout I/O via `tokio::io::AsyncBufReadExt` and `tokio::io::AsyncWriteExt`
- Tool handler async execution

## Files & Modules

| File | Action | Purpose |
|------|--------|---------|
| `src/mcp/mod.rs` | **Create** | Module declaration, re-exports `McpServer`, provides `pub async fn serve()` entry point |
| `src/mcp/protocol.rs` | **Create** | JSON-RPC 2.0 types: `JsonRpcMessage`, `JsonRpcResponse`, `JsonRpcError`, `CallToolResult`, `ContentBlock` |
| `src/mcp/transport.rs` | **Create** | Generic `StdioTransport<R, W>` with `read_message`/`write_message` over newline-delimited JSON |
| `src/mcp/server.rs` | **Create** | `McpServer<R, W>` struct, request dispatch loop, `initialize`/`ping`/`tools/list` handlers |
| `src/mcp/handlers.rs` | **Create** | Tool handler functions for all 9 tools, each returning `Result<Value, String>` |
| `src/mcp/schema.rs` | **Create** | `fn tool_definitions() -> Vec<Value>` — JSON Schema definitions for each tool's `inputSchema`, built as `serde_json::json!()` literals |
| `src/mcp/tail_events.rs` | **Create** | MCP-specific tail event collection: scans project artifacts + state, returns `Vec<serde_json::Value>` |
| `src/lib.rs` | **Edit** | Add `pub mod mcp;` |
| `src/cli/mod.rs` | **Edit** | Add `Mcp(McpArgs)` command variant, `McpArgs`, `McpCommand` enum, dispatch arm |

## Testing Strategy

### 1. Protocol Unit Tests (`src/mcp/protocol.rs`)

- Round-trip serialization/deserialization of `JsonRpcMessage` (request with id, notification without id)
- Serialize `JsonRpcResponse` with result vs error
- `CallToolResult` serializes with `isError: true` and `isError: false`
- `ContentBlock` always has `type: "text"`

### 2. Transport Unit Tests (`src/mcp/transport.rs`)

Using `tokio::io::BufReader<&[u8]>` as the reader and `Vec<u8>` as the writer:
- `read_message` parses a valid JSON-RPC line into `JsonRpcMessage`
- `read_message` returns `None` on empty input (EOF)
- `read_message` handles malformed JSON gracefully (returns parse error)
- `write_message` produces single-line JSON terminated by `\n`
- Round-trip: write then read a message

### 3. Handler Unit Tests (`src/mcp/handlers.rs`)

Each handler tested with a temp workspace created via `Workspace::init(tempdir)`:
- **project_new**: verify project appears in index after call; verify error on duplicate id; verify error when both `prompt_file` and `from_project` given; verify error when neither given
- **project_list**: verify empty list, then list after creating projects
- **project_show**: verify JSON contains expected fields (`project_id`, `status`, `current_loop`, `current_phase`) after project creation
- **status**: test with pre-populated project state, verify JSON structure
- **history**: test with pre-populated state containing feature loops + completion attempts, verify sorted array
- **config_show**: verify returns valid JSON for global scope; verify returns valid JSON for project scope; verify error when both `global` and `project` specified
- **tail**: test with a project that has pre-populated artifact files + state.json, verify event array contains expected events
- Error cases: missing required params return `Err(String)`, nonexistent project returns `Err`, workspace not found returns `Err`

**Note on `run` and `quick_prd`**: These are not unit-tested at the handler level because they invoke real backends. They are covered by:
- The existing integration tests for `Orchestrator::run` and `QuickPrdPipeline::run`
- Verifying param parsing/validation in handler unit tests (using invalid params, missing required fields)
- The `dry_run` parameter for `run` can exercise the full handler path without backend calls in integration tests

### 4. Server Integration Tests (`src/mcp/server.rs` or `tests/mcp_integration.rs`)

Using the injectable `McpServer::new()` constructor with in-memory `BufReader<Cursor<Vec<u8>>>` reader and `Vec<u8>` writer:
- Feed a sequence of newline-delimited JSON-RPC messages as input
- Run the server to completion (reader hits EOF)
- Parse the output buffer into individual JSON-RPC responses
- Test scenarios:
  - `initialize` → verify `protocolVersion`, `serverInfo`, `capabilities.tools`
  - `tools/list` → verify all 9 tools present, each has `name`, `description`, `inputSchema` with `type: "object"` and `properties`
  - `tools/call` for `project_list` → verify `isError` absent or false, content contains JSON array
  - `tools/call` for `project_new` (with temp workspace) → verify success
  - `tools/call` with unknown tool name → verify `isError: true` in result (tool execution error, not JSON-RPC error)
  - Unknown method with `id` → verify JSON-RPC `-32601` error
  - `ping` → verify empty result `{}`
  - Malformed JSON line → verify `-32700` parse error response
  - `notifications/initialized` → verify no response written
  - Unknown notification → verify no response written

### 5. Schema Shape Tests (`src/mcp/schema.rs`)

- Each tool definition has `name` (string), `description` (string), `inputSchema` (object with `type: "object"` and `properties`)
- Tools with required params have a `required` array in `inputSchema`
- `project_new` schema has `id`, `name`, `prompt_file`, `from_project`, `backend` in properties
- `run` schema has all documented params in properties
- No external JSON Schema validator crate — tests verify structural correctness (presence of expected keys, types of values) not full JSON Schema compliance

### 6. Test Dependencies

No new test dependencies. Uses existing `tempfile` crate for temp workspaces and `tokio::test` for async tests. Transport tests use `tokio::io::BufReader<std::io::Cursor<Vec<u8>>>` and `Vec<u8>` (which implements `AsyncWrite` via `tokio::io::AsyncWriteExt` when wrapped appropriately, or use a simple newline-collecting buffer).

## Out of Scope

- **Streaming/SSE transport** — v1 is stdio only
- **Follow mode for tail** — the MCP `tail` tool returns a snapshot; no streaming events
- **Authentication/authorization** — the MCP server runs locally with the same permissions as the user
- **Concurrent request handling** — requests are processed sequentially; concurrent access requires separate server instances
- **The `prd` (multi-stage interactive PRD) tool** — only `quick_prd` is exposed since the full PRD pipeline requires interactive user input
- **The `auto` command as a single tool** — clients can compose `quick_prd` → `project_new` → `run` themselves
- **The `rollback`, `init`, `validate` commands** — not exposed as MCP tools in v1
- **MCP resources or prompts capabilities** — only the `tools` capability is implemented
- **HTTP/WebSocket/Streamable HTTP transport** — stdio only for v1
- **Progress notifications during `run`** — the `run` tool blocks until completion; progress monitoring can be done by polling `status` or `tail` from a separate MCP connection
- **`tmux`/`no_tmux` parameter on `run`** — MCP is a non-interactive stdio transport; tmux session management does not apply. The handler always passes `tmux: None` to `RunOptions`, and the server operates without tmux regardless of workspace config.
- **`interactive`/`non_interactive` parameter on `quick_prd`** — MCP is always non-interactive; the handler always runs in non-interactive mode.
- **Inline spec content in `quick_prd` response** — large specs could produce very large payloads; clients read the file at the returned `spec_path` instead.
- **Strict JSON Schema validation of tool arguments** — the server validates required fields and types within handler code but does not run a JSON Schema validator library against `inputSchema`. Invalid arguments produce `isError: true` responses with descriptive messages.
- **Rejecting unknown/extra parameters** — tool handlers use `serde_json::Value` and extract known fields; extra fields are silently ignored (following Postel's law and common MCP client behavior of passing extra metadata).
