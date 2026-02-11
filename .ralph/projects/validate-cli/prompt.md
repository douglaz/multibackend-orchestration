# ralph validate — Conformance Test CLI

## Goal

Add a `ralph validate` subcommand that tests an **external** ralph binary (specified via `--bin`) for spec conformance. This enables testing new implementations of ralph in different languages.

## CLI Interface

```
ralph validate --bin /path/to/ralph-under-test [OPTIONS]

OPTIONS:
    --bin <PATH>        Path to ralph binary to test (required)
    --filter <PATTERN>  Run only tests matching pattern (substring match)
    --list              List available tests without running them
    --verbose           Show detailed output for each test
```

Output format (inspired by `cargo test`):
```
running 40 tests
test init::creates_workspace_structure ... ok
test init::creates_template_files ... ok
test run::single_feature_loop ... FAILED

failures:

--- run::single_feature_loop ---
  assertion failed: expected status "in_progress", got "pending"
  state.json: {"current_loop": 1, ...}

test result: FAILED. 39 passed; 1 failed
```

Exit code: 0 if all tests pass, 1 if any test fails.

## Architecture

Create a new source module: `src/validate/`

```
src/validate/
    mod.rs              — public interface, ValidateArgs, execute()
    runner.rs           — TestRunner: discovers tests, runs them, collects results
    harness.rs          — RalphHarness: temp dir, git repo, binary invocation
    assertions.rs       — assert helpers (exit code, JSON fields, files, git, stdout)
    mock_scripts.rs     — mock backend bash script generators
    tests_init.rs       — init command tests
    tests_project.rs    — project command tests
    tests_run.rs        — run command tests (largest)
    tests_commands.rs   — status, history, rollback, config, exit code tests
```

### Key Types

```rust
// A single conformance test
struct ConformanceTest {
    name: &'static str,           // e.g. "init::creates_workspace_structure"
    func: fn(&RalphHarness) -> TestResult,
}

enum TestResult {
    Pass,
    Fail(String),   // failure message
    Skip(String),   // skip reason
}

// Collects and runs tests
struct TestRunner {
    tests: Vec<ConformanceTest>,
    ralph_bin: PathBuf,
    filter: Option<String>,
    verbose: bool,
}
```

### RalphHarness

```rust
struct RalphHarness {
    temp_dir: TempDir,
    repo_root: PathBuf,
    ralph_bin: PathBuf,
}
```

Methods:
- `new(bin)` — creates TempDir, initializes a bare git repo inside it (git init, initial commit), returns harness
- `ralph(args)` — runs the ralph binary with given args, CWD set to repo_root, returns `Output` (stdout, stderr, exit code)
- `ralph_ok(args)` — like ralph() but asserts exit code 0, returns stdout as String
- `ralph_exit(args, code)` — runs and asserts specific exit code
- `load_state(project_id)` — reads and parses `.ralph/projects/{id}/state.json`
- `load_index()` — reads and parses `.ralph/index.json`
- `init_workspace()` — runs `ralph init`
- `write_mock_script(name, content)` — writes executable bash script to temp dir
- `setup_mock_backends(script)` — configures claude_command and codex_command to use mock scripts via `ralph config set`
- `create_project(id, name, prompt)` — writes prompt to temp file, runs `ralph project new`

Each test function receives a **fresh** harness (new TempDir + git repo per test).

## Mock Backend Strategy

Use the same proven bash script approach from `tests/orchestrator.rs`. Mock scripts:
1. Read stdin (the prompt from ralph)
2. Pattern-match on template content keywords
3. Return properly formatted H1-prefixed markdown for each phase

Example mock script structure:
```bash
#!/usr/bin/env bash
INPUT=$(cat)
if echo "$INPUT" | grep -q "Feature Specification"; then
    echo "# Feature Specification"
    echo ""
    echo "## Overview"
    echo "Mock feature spec for testing."
    # ... more content matching template structure
elif echo "$INPUT" | grep -q "Implementation Notes"; then
    echo "# Implementation Notes"
    echo ""
    echo "## Changes Made"
    echo "- Modified mock_file.txt"
    # Create a file to simulate implementation
    echo "implemented" > mock_file.txt
    git add mock_file.txt
elif echo "$INPUT" | grep -q "Code Review"; then
    echo "# Code Review"
    echo ""
    echo "## Verdict: APPROVE"
    echo "Code looks good."
elif echo "$INPUT" | grep -q "Completion Assessment"; then
    # Check env var or counter for completion behavior
    if [ "${RALPH_COMPLETE:-no}" = "yes" ]; then
        echo "# Completion Assessment"
        echo ""
        echo "## Verdict: COMPLETE"
    else
        echo "# Completion Assessment"
        echo ""
        echo "## Verdict: CONTINUE"
        echo ""
        echo "## Next Feature"
        echo "Another feature"
    fi
fi
```

Config is set via `ralph config set` calls — fully black-box testing.

For alternation tests: create separate claude/codex scripts, each with their own behavior or counter files to verify which backend was called.

## Test Inventory (~40 tests)

### init:: (5 tests)
1. `creates_workspace_structure` — after `ralph init`, `.ralph/` exists with ralph.toml, index.json, projects/, templates/
2. `creates_template_files` — 4 template files exist and are non-empty (spec.md, implementation.md, review.md, completion.md in templates/)
3. `default_config` — ralph.toml parses as valid TOML with correct defaults
4. `default_index` — index.json has workspace_version="1.0" and empty projects array
5. `rejects_nonempty_dir` — running `ralph init` twice gives exit code 2

### project:: (9 tests)
1. `new_creates_state` — state.json has current_loop=0, phase=planning, status=pending
2. `new_copies_prompt` — prompt.md in project dir matches source prompt content
3. `new_updates_index` — index.json projects array has entry, active_project is set
4. `new_creates_branch` — git branch `ralph/{id}` exists after project new
5. `new_rejects_duplicate` — creating project with same id gives exit code 2
6. `list_shows_project` — `ralph project list` stdout contains project id and name
7. `use_switches_active` — after `ralph project use`, active_project updated in index.json
8. `show_displays_info` — `ralph project show` stdout has id, status, phase
9. `show_json` — `ralph project show --json` outputs valid JSON with correct fields

### run:: (14 tests)
1. `single_feature_loop` — one full loop completes: spec, impl, review, commit artifacts; git tag exists
2. `artifact_naming` — artifacts follow `loops/NNN-slug/YYYYMMDDHHMMSS-{kind}.md` pattern
3. `artifact_frontmatter` — YAML frontmatter in artifacts has: artifact, loop, project, backend, role fields
4. `state_after_loop` — after one loop with CONTINUE verdict: current_loop=1, phase=planning, status=in_progress
5. `git_tag_format` — tag `ralph/{project_id}/loop-1` exists after one loop
6. `two_loops_alternation` — loop 1 planner=claude, loop 2 planner=codex (verified via artifact frontmatter)
7. `completion_flow` — mock returns COMPLETE verdict: status=completed
8. `review_limit_rollback` — set max_review_iterations=1, mock reviewer always rejects: loop fails, loops dir empty
9. `dry_run` — `ralph run --dry-run`: no state changes, no artifacts written
10. `until_review` — `ralph run --until review`: stops after review phase, doesn't commit
11. `resume_after_interrupt` — first run with `--until review`, second `ralph run` completes the commit
12. `dirty_tree_rejected` — create uncommitted file before run: ralph errors
13. `skip_commit` — `ralph run --skip-commit`: no commit hash in state, no git tag
14. `loops_flag` — `ralph run --loops 2`: exactly 2 completed feature loops

### commands:: (12 tests)
1. `status_shows_info` — `ralph status` shows project name and phase
2. `status_no_active_project` — `ralph status` with no active project: meaningful message (not crash)
3. `history_shows_loops` — `ralph history` lists completed loops
4. `history_json` — `ralph history --json` outputs valid JSON array
5. `history_verbose` — `ralph history --verbose` shows more detail than default
6. `rollback_removes_loops` — `ralph rollback 1` removes loop artifacts
7. `rollback_resets_phase` — after rollback, phase is reset to planning
8. `rollback_hard` — `ralph rollback --hard 1` also resets git
9. `config_get` — `ralph config get planner_backend` returns current value
10. `config_set` — `ralph config set planner_backend codex` persists change
11. `exit_code_workspace_not_found` — running ralph commands outside workspace: exit code 2
12. `exit_code_project_not_found` — `ralph project show nonexistent`: exit code 2

## Files to Create/Modify

| File | Action |
|------|--------|
| `src/validate/mod.rs` | **Create** — ValidateArgs (clap), execute(), register all tests |
| `src/validate/runner.rs` | **Create** — TestRunner, ConformanceTest, TestResult, run loop |
| `src/validate/harness.rs` | **Create** — RalphHarness struct and all methods |
| `src/validate/assertions.rs` | **Create** — Assert helpers for exit code, JSON, files, git, stdout |
| `src/validate/mock_scripts.rs` | **Create** — Mock backend script generators |
| `src/validate/tests_init.rs` | **Create** — 5 init tests |
| `src/validate/tests_project.rs` | **Create** — 9 project tests |
| `src/validate/tests_run.rs` | **Create** — 14 run tests |
| `src/validate/tests_commands.rs` | **Create** — 12 command tests |
| `src/lib.rs` | **Edit** — Add `pub mod validate;` |
| `src/cli/mod.rs` | **Edit** — Add `Validate(ValidateArgs)` variant to Commands enum, add dispatch in execute() |
| `Cargo.toml` | **Edit** — Move `tempfile = "3"` from `[dev-dependencies]` to `[dependencies]` |
| `flake.nix` | **Edit** — Add `src/validate/mock_scripts.rs` to postPatch shebang patching list |

## Important Implementation Details

1. **tempfile dependency**: Currently a dev-dependency. Must be moved to regular `[dependencies]` since validate is part of the main binary.

2. **Mock scripts use `#!/usr/bin/env bash`**: The flake.nix postPatch must patch shebangs in `src/validate/mock_scripts.rs` just like it does for test files.

3. **Each test gets a fresh harness**: The TestRunner should create a new RalphHarness for each test function call, ensuring test isolation.

4. **Black-box testing only**: Tests must ONLY interact with ralph through its CLI. No importing internal modules or directly manipulating internal state — only invoke the binary, read files it creates, and check git state.

5. **The harness git init must include an initial commit**: Ralph requires a git repo with at least one commit. The harness should create an initial commit (e.g., empty `.gitkeep` file).

6. **Mock implementer must actually create and git-add files**: The ralph run phase expects the implementer to create changes. Mock scripts must `echo content > file && git add file` to simulate this.

7. **Reference tests/orchestrator.rs for mock script patterns**: The existing integration tests have working mock scripts that handle all phases correctly. Use those as the template.
