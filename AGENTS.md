# Agent Guidelines for ralph

## Project overview

ralph is a multi-backend orchestration tool for structured AI development loops. It coordinates AI backends (claude, codex) through planning, implementing, QA, reviewing, and completion phases.

## Build & test

```bash
nix develop -c cargo check       # type-check
nix develop -c cargo test         # unit + integration tests
nix build -L                      # release binary at ./result/bin/ralph
```

## Conformance tests (validate)

ralph has a conformance test suite under `src/validate/`. Run it against a built binary:

```bash
./result/bin/ralph validate --bin ./result/bin/ralph           # full suite
./result/bin/ralph validate --bin ./result/bin/ralph --filter mcp  # filter by name
./result/bin/ralph validate --bin ./result/bin/ralph --list        # list all tests
```

**Every new feature or CLI command must have validate conformance tests.** These tests are the living specification of ralph's behavior. They run the actual binary in isolated temp directories with mock backends.

### Adding validate tests

1. Create `src/validate/tests_<feature>.rs` following the pattern in `tests_commands.rs` or `tests_mcp.rs`
2. Export `pub fn tests() -> Vec<ConformanceTest>`
3. Register in `src/validate/mod.rs`: add `mod tests_<feature>;` and `tests.extend(tests_<feature>::tests());`
4. Each test takes `&RalphHarness` and returns `TestResult`, wrapped in `run_case(|| { ... })`
5. Use helpers from `src/validate/assertions.rs` and `src/validate/harness.rs`

### Validate coverage by area

| Module | What it covers |
|--------|---------------|
| `tests_init` | Workspace initialization |
| `tests_project` | Project CRUD, branches, state |
| `tests_run` | Orchestration loops, artifacts, git tags, dry-run, resume |
| `tests_qa` | QA phase, acceptance gates, config |
| `tests_commands` | status, history, rollback, config get/set/show |
| `tests_tail` | Tail events, JSON output, --last flag |
| `tests_mcp` | MCP server protocol, all 9 tools, error handling |

## Architecture

```
src/
  backend/     # Backend trait, registry, claude/codex drivers
  cli/         # Clap command definitions and execute functions
  config/      # TOML config parsing and merging
  git/         # Git operations (commit, branch, tag)
  mcp/         # MCP JSON-RPC server (protocol, transport, handlers, schema)
  prd/         # PRD pipeline (full + quick-prd)
  project/     # Project lifecycle, state, artifacts
  prompts/     # Embedded prompt templates
  validate/    # Conformance test framework and test modules
  workflow/    # Orchestrator, parser, stage logic
  workspace/   # Workspace discovery and management
```

## Key conventions

- **Artifact naming**: Implementation response files use timestamp-prefixed filenames (`YYYYMMDDHHMMSS-impl-response-NNN.md`) with YAML frontmatter. See `src/workflow/parser.rs`.
- **Backend specs**: Format is `backend_name` or `backend_name(model)`, e.g. `claude(opus)`, `codex(gpt-5)`. Validated by `src/cli/backend_spec.rs`.
- **Error handling**: All errors go through `RalphError` in `src/error.rs` with specific exit codes.
- **Config merging**: Global config (`.ralph/ralph.toml`) merges with project config (`.ralph/projects/<id>/config.toml`). Project values override global.
- **State files**: Project state lives at `.ralph/projects/<id>/state.json`. Loop artifacts live under `.ralph/projects/<id>/loops/<NNN-slug>/`.

## Workspace layout

```
.ralph/
  ralph.toml              # global config
  index.json              # project registry
  templates/              # prompt templates
  projects/<id>/
    state.json            # project state
    config.toml           # project-level config overrides
    prompt.md             # project specification
    loops/<NNN-slug>/     # loop artifacts
```
