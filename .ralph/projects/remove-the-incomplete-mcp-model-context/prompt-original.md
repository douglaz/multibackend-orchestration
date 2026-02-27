Good - `ralph_with_stdin` is a general-purpose harness method, not MCP-specific. It's used by MCP tests but will remain useful.

I now have everything I need. Here's the specification:

---

## Summary

Remove the entire MCP (Model Context Protocol) JSON-RPC server module (`src/mcp/`) and all associated CLI wiring, conformance tests, and integration tests. The MCP server is a custom-built stdio JSON-RPC 2.0 implementation exposing 9 tools that duplicate existing CLI functionality. It adds ~1,200 lines of production code and ~1,100 lines of test code without active consumers. Removing it reduces maintenance surface with zero impact on users.

## Acceptance Criteria

- [ ] `src/mcp/` directory and all 7 files within it are deleted
- [ ] `tests/mcp_handlers.rs` integration test file is deleted
- [ ] `src/validate/tests_mcp.rs` conformance test file is deleted
- [ ] No `mcp` module declaration in `src/lib.rs`
- [ ] No `Mcp` variant, `McpArgs`, or `McpCommand` in `src/cli/mod.rs`
- [ ] No `tests_mcp` import or registration in `src/validate/mod.rs`
- [ ] `nix build` passes (release build + conformance tests)
- [ ] No dead code warnings related to MCP leftovers
- [ ] `grep -ri mcp src/ tests/` returns zero hits (excluding this spec if committed)

## Technical Approach

The removal is mechanical — delete files, then remove all import/reference sites. No logic changes to surviving code are needed since the MCP module is self-contained and the CLI `tail.rs` has its own independent event collection.

**Step 1: Delete MCP source files**
- Delete the entire `src/mcp/` directory (7 files: `mod.rs`, `server.rs`, `protocol.rs`, `transport.rs`, `handlers.rs`, `schema.rs`, `tail_events.rs`)

**Step 2: Delete MCP test files**
- Delete `tests/mcp_handlers.rs` (447 lines — end-to-end integration tests against `McpServer`)
- Delete `src/validate/tests_mcp.rs` (662 lines — 22 conformance tests run via `ralph validate`)

**Step 3: Remove module declaration in `src/lib.rs`**
- Delete line 7: `pub mod mcp;`

**Step 4: Remove CLI subcommand from `src/cli/mod.rs`**
- Remove the `Mcp(McpArgs)` variant from the `Commands` enum (line 36)
- Remove the `McpArgs` struct (lines 104–108)
- Remove the `McpCommand` enum (lines 110–113)
- Remove the `Commands::Mcp` match arm in `run()` (lines 290–292)
- Remove the `McpCommand` import and `parses_mcp_serve_command` test (lines 311, 347–354)

**Step 5: Remove conformance test registration in `src/validate/mod.rs`**
- Delete line 20: `mod tests_mcp;`
- Delete line 89: `tests.extend(tests_mcp::tests());`

**Step 6: Update `AGENTS.md`**
- Remove MCP references from the conformance test table and directory tree (lines 29, 45, 55)

**Step 7: Verify no unused dependencies**
- All dependencies in `Cargo.toml` (`serde_json`, `tokio`, `serde`, `thiserror`) are used by other modules. No `Cargo.toml` changes needed.

## Files & Modules

| Action | File | Reason |
|--------|------|--------|
| **Delete** | `src/mcp/mod.rs` | Module root, `serve()` entry point |
| **Delete** | `src/mcp/server.rs` | `McpServer` JSON-RPC loop (283 lines) |
| **Delete** | `src/mcp/protocol.rs` | JSON-RPC type definitions |
| **Delete** | `src/mcp/transport.rs` | Stdio transport layer |
| **Delete** | `src/mcp/handlers.rs` | 9 tool handlers + 11 unit tests (612 lines) |
| **Delete** | `src/mcp/schema.rs` | Tool definitions + JSON schemas |
| **Delete** | `src/mcp/tail_events.rs` | MCP-specific tail event collector |
| **Delete** | `tests/mcp_handlers.rs` | End-to-end integration tests (447 lines) |
| **Delete** | `src/validate/tests_mcp.rs` | 22 conformance tests (662 lines) |
| **Edit** | `src/lib.rs` | Remove `pub mod mcp;` (line 7) |
| **Edit** | `src/cli/mod.rs` | Remove `Mcp` variant, `McpArgs`, `McpCommand`, match arm, test |
| **Edit** | `src/validate/mod.rs` | Remove `mod tests_mcp;` and `tests.extend(tests_mcp::tests())` |
| **Edit** | `AGENTS.md` | Remove MCP from test table and directory listing |

## Testing Strategy

1. **Build verification**: `nix build` must succeed — this runs the release build and the full conformance test suite, confirming no compile errors and no broken tests.
2. **Dead code check**: The release build uses `-D warnings`, so any dead code from dangling MCP references will fail the build.
3. **Grep audit**: Run `grep -ri mcp src/ tests/` after removal to confirm no stale references remain in source/test code.
4. **Conformance count**: The `ralph validate` suite should report its test count decreased by 22 (the removed `mcp::*` tests) with all remaining tests passing.

## Out of Scope

- Removing MCP references in `.ralph/` project data directories (historical project artifacts, prompts, review notes) — these are runtime data, not source code
- Removing MCP mentions in `AGENTS.md` beyond the structural references (changelog-style mentions are acceptable)
- Adding any replacement functionality — this is a pure removal
- Modifying `Cargo.toml` dependencies — all current deps are shared with other modules