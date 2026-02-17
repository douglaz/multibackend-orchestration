## Summary

Add a `--dry-run` / `-n` flag to `ralph init` that simulates initialization by printing each action (directory creation, file writes, symlinks) to stdout without performing any filesystem operations. Both the dry-run and real paths derive their action list from a single shared source to prevent drift. The default behavior (no flag) remains unchanged.

## Acceptance Criteria

- `--dry-run` and `-n` are both accepted as CLI flags on `ralph init`
- `ralph init --dry-run` prints one line per filesystem action to stdout, in execution order:
  - `would create <dir>/` for each directory (root, `projects/`, `templates/`)
  - `would write <dir>/ralph.toml`
  - `would write <dir>/templates/<name>.md` for each template file
  - `would link <dir>/templates/<legacy>.md -> <canonical>.md` for each legacy symlink
- No files or directories are created when `--dry-run` is active
- `ralph init --dry-run` does **not** print `initialized workspace at ...` (that message is only for the real path)
- `ralph init` without the flag behaves identically to current behavior (including the final `initialized workspace at ...` message)
- Validation parity: dry-run produces the same errors as the real path for these edge cases:
  - Target directory exists and is non-empty → `RalphError::Validation` (exit code 2)
  - Target path exists but is not a directory (e.g., a regular file) → `std::io::Error` from `read_dir` (exit code 1)
  - Target path exists but is unreadable (permissions) → `std::io::Error` from `read_dir` (exit code 1)
- No regressions in existing `init` tests; new unit tests and validate conformance tests cover dry-run behavior

## Technical Approach

### 1. Add `dry_run` field to `InitArgs` (`src/cli/mod.rs`)

Add a boolean field with both `--dry-run` and `-n` short alias, defaulting to `false`:

```rust
#[derive(Debug, Args)]
pub struct InitArgs {
    #[arg(long, default_value = ".ralph")]
    pub dir: PathBuf,

    #[arg(long, short = 'n')]
    pub dry_run: bool,
}
```

This matches the existing pattern used by `RunArgs` and `RollbackArgs` (which already have `dry_run` fields).

### 2. Introduce a shared action plan (`src/cli/init.rs`)

Define an enum and a function that produces the ordered list of actions `create_workspace` would perform, without executing them:

```rust
enum InitAction {
    CreateDir(PathBuf),
    WriteFile(PathBuf),
    Symlink { link: PathBuf, target: String },
}

fn plan_actions(root: &Path) -> Vec<InitAction> {
    let templates = root.join("templates");
    let mut actions = vec![
        InitAction::CreateDir(root.to_path_buf()),
        InitAction::CreateDir(root.join("projects")),
        InitAction::CreateDir(templates.clone()),
        InitAction::WriteFile(root.join("ralph.toml")),
    ];

    for name in TEMPLATE_FILES {
        actions.push(InitAction::WriteFile(templates.join(name)));
    }

    for (canonical, legacy) in LEGACY_LINKS {
        actions.push(InitAction::Symlink {
            link: templates.join(legacy),
            target: canonical.to_string(),
        });
    }

    actions
}
```

The `TEMPLATE_FILES` and `LEGACY_LINKS` constants are shared between `plan_actions` and `create_workspace`:

```rust
const TEMPLATE_FILES: &[&str] = &[
    "spec.md",
    "implementation.md",
    "review.md",
    "prompt_reviewer.md",
    "completion.md",
    "qa.md",
];

const LEGACY_LINKS: &[(&str, &str)] = &[
    ("spec.md", "planner.md"),
    ("implementation.md", "implementer.md"),
    ("review.md", "reviewer.md"),
    ("completion.md", "completer.md"),
];
```

Refactor `create_workspace` to iterate `TEMPLATE_FILES` and `LEGACY_LINKS` instead of its current inline lists, so the real path and the plan are guaranteed to stay in sync.

### 3. Branch in `execute()` on `dry_run`

```rust
pub fn execute(args: InitArgs) -> Result<()> {
    // Validation runs unconditionally (both paths).
    validate_target(&args.dir)?;

    if args.dry_run {
        for action in plan_actions(&args.dir) {
            match action {
                InitAction::CreateDir(p) => println!("would create {}/", p.display()),
                InitAction::WriteFile(p) => println!("would write {}", p.display()),
                InitAction::Symlink { link, target } => {
                    println!("would link {} -> {}", link.display(), target);
                }
            }
        }
        Ok(())
    } else {
        let workspace = create_workspace(&args.dir)?;
        println!("initialized workspace at {}", workspace.root.display());
        Ok(())
    }
}
```

### 4. Extract validation into `validate_target()` (`src/cli/init.rs`)

Extract the validation logic from `Workspace::init` into a shared `validate_target` helper in `init.rs` so both paths run it. This is a thin wrapper that calls `read_dir` on an existing path and checks for non-emptiness, returning the same `RalphError::Validation` or `io::Error`:

```rust
fn validate_target(root: &Path) -> Result<()> {
    if root.exists() {
        let mut entries = std::fs::read_dir(root)?;
        if entries.next().is_some() {
            return Err(RalphError::Validation(format!(
                "workspace directory '{}' already exists and is not empty",
                root.display()
            )));
        }
    }
    Ok(())
}
```

`Workspace::init` can then call `validate_target` internally (or the duplicate check can remain in `Workspace::init` since `create_workspace` calls it after `execute` has already validated — either approach is acceptable as long as the dry-run path runs the same check). The simplest approach: call `validate_target` in `execute()` before the branch, and leave `Workspace::init` unchanged (the double-check on the real path is harmless and idempotent).

## Files & Modules

| File | Change |
|---|---|
| `src/cli/mod.rs` | Add `dry_run: bool` field with `#[arg(long, short = 'n')]` to `InitArgs` |
| `src/cli/init.rs` | Add `InitAction` enum, `TEMPLATE_FILES` / `LEGACY_LINKS` constants, `plan_actions()`, `validate_target()`; refactor `create_workspace` to use the shared constants; branch on `args.dry_run` in `execute()` |
| `src/validate/tests_init.rs` | Add conformance tests for `init --dry-run` and `init -n` |

No other files are modified. `Workspace::init` is untouched.

## Testing Strategy

### CLI parsing tests (`src/cli/mod.rs::tests`)

- **`parses_init_with_dry_run_flag`**: Assert `Cli::parse_from(["ralph", "init", "--dry-run"])` yields `dry_run == true`.
- **`parses_init_with_short_n_flag`**: Assert `Cli::parse_from(["ralph", "init", "-n"])` yields `dry_run == true`.
- **`parses_init_without_dry_run`**: Assert `Cli::parse_from(["ralph", "init"])` yields `dry_run == false`.

### Unit tests (`src/cli/init.rs::tests`)

- **`dry_run_prints_actions_without_creating_files`**: Call `execute(InitArgs { dir: tempdir.join(".ralph"), dry_run: true })`. Capture stdout (or test `plan_actions` directly and assert the returned `Vec<InitAction>` matches expectations). Assert:
  - Output contains "would create" and "would write" lines for all expected paths
  - Output does **not** contain "initialized workspace at"
  - The target directory does not exist after execution
- **`dry_run_rejects_non_empty_directory`**: Pre-populate a directory with a file, call with `dry_run: true`, assert the same `RalphError::Validation` error as the real path.
- **`plan_actions_matches_create_workspace`**: Call `plan_actions(root)` and verify the returned action list covers every file, directory, and symlink that `create_workspace` would produce.

### Existing test verification

- **`create_workspace_writes_all_templates`**: Unchanged, continues to pass, confirming no regression in the default path.

### Validate conformance tests (`src/validate/tests_init.rs`)

Add the following tests to the existing `tests()` function, using the standard `catch_unwind` + `TestResult` pattern:

- **`init::dry_run_prints_actions`**: Run `h.ralph_ok(["init", "--dry-run"])`. Assert:
  - stdout contains `"would create"` for root, `projects/`, `templates/`
  - stdout contains `"would write"` for `ralph.toml` and all 6 template files
  - stdout contains `"would link"` for all 4 legacy symlinks
  - stdout does **not** contain `"initialized workspace at"`
  - `.ralph/` directory does not exist (`assert_path_not_exists`)
- **`init::dry_run_short_flag`**: Run `h.ralph_ok(["init", "-n"])`. Assert stdout contains `"would create"` (confirming `-n` is accepted).
- **`init::dry_run_rejects_nonempty_dir`**: Run `h.init_workspace()` first, then `h.ralph_exit(["init", "--dry-run"], 2)`. Assert exit code 2 (same as `rejects_nonempty_dir` on the real path).
- **`init::dry_run_rejects_file_target`**: Create a regular file at `.ralph` (not a directory), then `h.ralph_exit(["init", "--dry-run", "--dir", path], 1)`. Assert exit code 1.

## Out of Scope

- Adding `--dry-run` to other subcommands (already exists on `run`, `rollback`, `prd`, `quick-prd` independently)
- Extracting validation out of `Workspace::init` (duplicate validation on the real path is harmless)
- JSON or machine-readable output format for the dry-run listing
- Colored or formatted output (plain text lines are sufficient)
