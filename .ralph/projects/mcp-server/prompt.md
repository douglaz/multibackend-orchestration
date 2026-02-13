Here's a summary of how the revised spec addresses each of the 9 review issues:

**1. Protocol/Transport Feasibility** — The reviewer was wrong: MCP stdio uses **newline-delimited JSON**, NOT Content-Length headers (confirmed from the official spec at modelcontextprotocol.io). The revised spec explicitly quotes the MCP spec ("Messages are delimited by newlines, and MUST NOT contain embedded newlines") and clarifies this is not LSP-style framing. The `read_line` approach is correct.

**2. MCP Lifecycle Completeness** — Added explicit `protocolVersion: "2025-06-18"` in the initialize response (AC #10), `ping` method support, and defined behavior for unknown notifications (silently ignored) vs unknown methods with ids (`-32601` error). See AC #11.

**3. Error Semantics** — Completely reworked. Tool execution failures now return `CallToolResult { isError: true }` with error text (AC #5). JSON-RPC `error` responses are reserved for protocol-level failures only (parse error, invalid request, unknown method, internal error). This matches the MCP spec's two-tier error model.

**4. Tool Schema Completeness** — `tmux`/`no_tmux` explicitly excluded with rationale (MCP is non-interactive stdio, tmux doesn't apply). `interactive`/`non_interactive` on `quick_prd` excluded with rationale (always non-interactive). `on_prompt_change` added to `run`. `global` added to `config_show` with mutual exclusivity with `project`. `dry_run` added to `quick_prd`. All documented in tool descriptions and Out of Scope.

**5. State Management Design** — Resolved the inconsistency: handlers re-discover the workspace on every `tools/call` invocation (AC #12). `McpServer` no longer stores a `Workspace`. This ensures freshness after mutations.

**6. Tail Reuse Feasibility** — Instead of making private types public, created a new `src/mcp/tail_events.rs` module that re-implements event collection at the library level (~150 lines), outputting `Vec<serde_json::Value>` directly. This avoids coupling to `TailEvent`/`TailEventKind`/`TailEventOutput` private types while producing identical JSON output.

**7. Acceptance Criteria Gaps** — Added mutual exclusivity constraints (`prompt_file` vs `from_project` in `project_new`, `global` vs `project` in `config_show`) with validation in handlers. Large payload concern addressed by not inlining spec content in `quick_prd` (clients read the file). Extra/unknown parameters are silently ignored (Postel's law, documented in Out of Scope).

**8. Testing Practicality** — Transport is now generic `StdioTransport<R, W>` with injectable reader/writer, making `McpServer::new()` testable with in-memory buffers (`Cursor<Vec<u8>>`). `run`/`quick_prd` are not unit-tested at the handler level (they hit real backends), but param validation is tested, and `dry_run` enables integration testing. Detailed test scenarios with in-memory I/O are specified.

**9. Schema Validation Plan** — Clarified: no JSON Schema validator crate. Tests verify structural correctness (expected keys present, types correct, required arrays populated) not full schema compliance. This is explicitly called out in both the Testing Strategy and Out of Scope sections.