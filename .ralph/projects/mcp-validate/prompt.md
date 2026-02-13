# Add MCP server conformance tests to `ralph validate`

## Summary

The MCP server (`ralph mcp serve`) has 9 tools, JSON-RPC protocol handling, and error handling — but the `ralph validate` conformance suite has zero MCP coverage. Add a `tests_mcp` module with 22 conformance tests covering protocol compliance, tool dispatch, and error handling.

## Acceptance Criteria

1. `src/validate/harness.rs` has a new `ralph_with_stdin()` method that spawns a ralph subprocess with `Stdio::piped()` stdin/stdout/stderr, writes input, drops stdin (signals EOF), and collects output via `wait_with_output()`.

2. `src/validate/tests_mcp.rs` exists as a new file with 22 conformance tests and helper functions.

3. `src/validate/mod.rs` registers the new module: `mod tests_mcp;` and `tests.extend(tests_mcp::tests());` in `register_tests()`.

4. All 22 tests pass when run via `ralph validate --bin <path> --filter mcp`.

5. The full validate suite still passes (no regressions).

6. `cargo check`, `cargo test`, and `nix build` all succeed.

## Technical Approach

### Harness addition (`src/validate/harness.rs`)

Add to `RalphHarness`:

```rust
pub fn ralph_with_stdin<I, S>(&self, args: I, input: &str) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(&self.ralph_bin)
        .args(args)
        .current_dir(&self.repo_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }

    let output = child.wait_with_output()?;
    Ok(output)
}
```

### Test module (`src/validate/tests_mcp.rs`)

Follow the exact pattern of `src/validate/tests_commands.rs` and `src/validate/tests_tail.rs`.

**Helper functions needed:**

- `jsonrpc_request(id, method, params) -> String` — builds `{"jsonrpc":"2.0","id":id,"method":method,"params":params}\n`
- `jsonrpc_notification(method, params) -> String` — builds `{"jsonrpc":"2.0","method":method,"params":params}\n`
- `init_handshake() -> String` — initialize request (id=1) + notifications/initialized notification
- `mcp_exchange(h, input) -> Vec<Value>` — pipes input to `ralph mcp serve`, asserts exit code 0, parses response lines
- `mcp_call(h, id, method, params) -> Vec<Value>` — init_handshake + one request
- `mcp_tool_call(h, tool_name, arguments) -> Vec<Value>` — init_handshake + tools/call request
- `extract_tool_result_json(response) -> Value` — unwraps `result.content[0].text` as parsed JSON
- `assert_jsonrpc_error(response, code)` — asserts response has error with expected code
- `assert_tool_error(response)` — asserts `result.isError == true`
- `run_case(f)`, `setup_with_project(h, project_id)` — standard patterns from other test modules

**Group 1: Protocol conformance (7 tests)**

1. `mcp::initialize_returns_protocol_info` — Send initialize. Assert: protocolVersion="2025-06-18", serverInfo.name="ralph", serverInfo.version is non-empty string, capabilities.tools={}.

2. `mcp::ping_returns_empty_object` — Send init handshake + ping (id=2). Assert: responses[1].result == {}.

3. `mcp::tools_list_returns_all_tools` — Send init handshake + tools/list (id=2). Assert: result.tools is array of length 9 containing all 9 tool names (project_new, project_list, project_show, run, status, history, tail, quick_prd, config_show). Each tool has name, description, inputSchema with type="object".

4. `mcp::unknown_method_returns_error` — Send init handshake + {"id":2,"method":"bogus/method"}. Assert: error.code == -32601.

5. `mcp::notification_without_id_is_silent` — Send init handshake + notification without id + ping (id=3). Assert: only 2 responses (initialize + ping), no response for the notification.

6. `mcp::notification_with_id_returns_method_not_found` — Send init handshake + {"id":99,"method":"notifications/custom"}. Assert: responses[1].error.code == -32601.

7. `mcp::malformed_json_returns_parse_error` — Send `{not valid json}\n` then initialize (id=1). Assert: first response has error.code == -32700, second response is successful initialize (server recovers).

**Group 2: Tool dispatch (9 tests)**

8. `mcp::tool_project_list_empty` — Setup: init_workspace. Call project_list tool. Assert: isError absent/false, parsed content has projects as empty array.

9. `mcp::tool_project_list_with_projects` — Setup: init_workspace + create 2 projects via CLI. Call project_list. Assert: projects array length 2, entries have id/name/status.

10. `mcp::tool_project_show_returns_state` — Setup: init_workspace + create project "mcp-show". Call project_show with project="mcp-show". Assert: success, has project and state keys, state.current_phase == "planning".

11. `mcp::tool_project_new_creates_project` — Setup: init_workspace, write temp prompt file using write_mock_script. Call project_new with id="mcp-new", name="MCP New", prompt_file=<abs path>. Assert: success, created==true, h.load_state("mcp-new") succeeds.

12. `mcp::tool_status_shows_project_info` — Setup: init_workspace + create project "mcp-status". Call status with project="mcp-status". Assert: success, has project_id, current_phase, status fields.

13. `mcp::tool_history_empty_project` — Setup: init_workspace + create project "mcp-hist". Call history with project="mcp-hist". Assert: success, loops is empty array.

14. `mcp::tool_tail_empty_project` — Setup: init_workspace + create project "mcp-tail". Call tail with project="mcp-tail". Assert: success, events is empty array.

15. `mcp::tool_config_show_global` — Setup: init_workspace. Call config_show with global=true. Assert: success, has config object with workspace/backends/workflow/templates keys.

16. `mcp::tool_config_show_project` — Setup: init_workspace + create project "mcp-cfg". Call config_show with project="mcp-cfg". Assert: success, has workflow/backends/templates keys.

**Group 3: Error handling (6 tests)**

17. `mcp::tool_unknown_returns_error` — Setup: init_workspace. Call tools/call with name="nonexistent_tool". Assert: isError==true, content text mentions "unknown tool".

18. `mcp::tool_project_new_missing_id` — Call project_new with only name="Test". Assert: isError==true.

19. `mcp::tool_project_new_missing_prompt_source` — Call project_new with id="test", name="Test" but no prompt_file or from_project. Assert: isError==true.

20. `mcp::tool_project_new_mutual_exclusion` — Call project_new with id="test", name="Test", prompt_file="/tmp/x.md", from_project="parent". Assert: isError==true.

21. `mcp::tool_config_show_mutual_exclusion` — Call config_show with global=true AND project="some-proj". Assert: isError==true.

22. `mcp::tool_status_no_project` — Setup: init_workspace (no projects created). Call status with no arguments. Assert: isError==true.

## Files & Modules

- `src/validate/harness.rs` — add `ralph_with_stdin()` method
- `src/validate/tests_mcp.rs` — new file with 22 tests + helpers
- `src/validate/mod.rs` — add `mod tests_mcp;` + registration in `register_tests()`

## Testing Strategy

1. `cargo check` compiles without errors
2. `cargo test` passes (unit tests)
3. `nix build -L` succeeds
4. `./result/bin/ralph validate --bin ./result/bin/ralph --filter mcp` — all 22 MCP tests pass
5. `./result/bin/ralph validate --bin ./result/bin/ralph` — full suite passes (no regressions)

## Out of Scope

- Testing tools that require real backend execution (run, quick_prd non-dry-run)
- Adding MCP-specific assertions to `src/validate/assertions.rs` (helpers live in the test module)
- Documentation files
- Changes to the MCP server itself
