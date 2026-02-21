The revised specification has been written. Here's a summary of how each review issue was addressed:

**Issue 1 — Testing coverage (dead-code check unreliable):**
- Removed the false claim that `nix build` uses `-D warnings` (confirmed via `flake.nix` — it doesn't)
- Added an explicit `RUSTFLAGS='-D warnings' cargo check --all-targets` step in Testing Strategy (item 2) with rationale
- Added a matching acceptance criterion for this check
- Clarified in Out of Scope that permanently adding `-D warnings` to `flake.nix` is a separate concern

**Issue 2 — AGENTS.md cleanup incomplete:**
- Step 6 now enumerates all 4 MCP references in `AGENTS.md` by line number: the `--filter mcp` example (line 21), the `tests_mcp.rs` pattern reference (line 29), the coverage table row (line 45), and the architecture tree entry (line 55)
- Also added Step 7 to clean up `README.md`, which has 7 MCP references across the highlights, subcommand list, filter example, and the entire "## MCP Mode" section (lines 126–144)
- The Files & Modules table now includes `README.md` as an edit target
- Removed the Out of Scope bullet that previously excluded some `AGENTS.md` references

**Issue 3 — Missing user-facing behavior checks:**
- Added two acceptance criteria: `ralph --help` must not list `mcp`, and `ralph mcp serve` must return an unknown-command error
- Added Testing Strategy item 4 ("CLI surface verification") describing exactly how to verify this

**Issue 4 — Breaking change not acknowledged:**
- Rewrote the Summary to include a **Breaking change** paragraph explicitly naming the removed subcommand, listing all 9 affected MCP tools, and providing the migration path (use corresponding CLI commands directly)
- States this removal should be noted in the next release changelog