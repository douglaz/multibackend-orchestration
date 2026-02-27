# Remove MCP Server and All Source/Test Wiring

## Goal
Remove the entire MCP JSON-RPC server implementation and all code/test wiring that exposes or validates it, with no behavior changes to non-MCP features.

## Background
The MCP server is a custom stdio JSON-RPC 2.0 module with no active consumers and duplicates existing CLI behavior. This is a pure removal to reduce maintenance surface.

## In Scope
- Delete all MCP production code under `src/mcp/`.
- Delete MCP integration and conformance tests.
- Remove MCP CLI command types, parsing, dispatch, and related unit tests.
- Remove MCP module registration/imports from crate wiring.
- Remove structural MCP references from `AGENTS.md` conformance coverage and architecture tree sections.

## Out of Scope
- Any replacement functionality.
- Changes to historical runtime/project artifacts under `.ralph/`.
- Broad dependency cleanup unless required for successful build/tests.
- Unrelated refactors.

## Required File Changes
- Delete `src/mcp/mod.rs`
- Delete `src/mcp/server.rs`
- Delete `src/mcp/protocol.rs`
- Delete `src/mcp/transport.rs`
- Delete `src/mcp/handlers.rs`
- Delete `src/mcp/schema.rs`
- Delete `src/mcp/tail_events.rs`
- Delete `tests/mcp_handlers.rs`
- Delete `src/validate/tests_mcp.rs`
- Edit `src/lib.rs` to remove `pub mod mcp;`
- Edit `src/cli/mod.rs` to remove MCP-only CLI types/variants/dispatch/tests
- Edit `src/validate/mod.rs` to remove `tests_mcp` module import and registration
- Edit `AGENTS.md` to remove MCP entries from the validate coverage table and architecture tree

## Guardrails
- Keep `ralph_with_stdin` and other shared harness helpers unless they are proven unused outside MCP and safe to remove.
- Do not change behavior of non-MCP commands.
- Treat prior line numbers as hints only; use symbol/content-based edits.

## Acceptance Criteria
- No `src/mcp/` directory remains.
- `tests/mcp_handlers.rs` and `src/validate/tests_mcp.rs` are removed.
- No `mcp` module declaration remains in `src/lib.rs`.
- No `Mcp` CLI command types/variants/dispatch/tests remain in `src/cli/mod.rs`.
- No `tests_mcp` module import/registration remains in `src/validate/mod.rs`.
- `AGENTS.md` no longer lists MCP in validate coverage or architecture tree.
- `src/` and `tests/` contain no MCP references:
  - `rg -n -i '\bmcp\b' src tests` returns no matches.
- Build and test verification passes:
  - `nix develop -c cargo check`
  - `nix develop -c cargo test`
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph`
- Validate suite no longer contains MCP cases:
  - `./result/bin/ralph validate --bin ./result/bin/ralph --list | rg -i mcp` returns no matches.

## Implementation Notes
- This is a mechanical removal. Prefer deleting MCP files first, then compile-fixing references.
- If additional MCP references appear (docs/tests/snapshots), remove or update them when they are part of source/test/CLI behavior.
- Avoid unrelated cleanup.

## Deliverable
Provide:
- A list of changed/deleted files.
- The exact verification commands run.
- A short summary of command outcomes (pass/fail and key result lines).