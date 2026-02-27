Implement a best-effort daemon startup preflight that ensures Ralph lifecycle GitHub labels exist before the runtime loop starts.

### Objective
Add a one-time startup step that creates these five labels in the target repo:

- `ralph:ready`
- `ralph:in-progress`
- `ralph:completed`
- `ralph:failed`
- `ralph:aborted`

This step must never block daemon startup.

### Scope
In scope:
- Label ensure logic in daemon GitHub integration.
- Startup wiring in daemon CLI flow.
- Test and mock updates required for deterministic behavior.
- Drift guard to keep ensured labels aligned with workflow-used labels.

Out of scope:
- Any `ralph:done` alias/migration.
- Managing user-configured `workspace.daemon_labels` beyond the fixed five lifecycle labels.
- Deleting/renaming existing repo labels.
- Overwriting existing label color/description.
- Multi-repo label management.

### Required Behavior
1. Define a single source of truth constant in `src/daemon/github.rs`:

```rust
pub const REQUIRED_LABELS: &[(&str, &str, &str)] = &[
    ("ralph:ready",       "#0e8a16", "Issue is ready for Ralph daemon pickup"),
    ("ralph:in-progress", "#fbca04", "Ralph daemon is working on this issue"),
    ("ralph:completed",   "#1d76db", "Ralph daemon completed this issue"),
    ("ralph:failed",      "#d93f0b", "Ralph daemon task failed"),
    ("ralph:aborted",     "#e4e669", "Ralph daemon task was aborted"),
];
```

2. Add `ensure_labels_best_effort(owner: &str, repo: &str)` in `src/daemon/github.rs`:
- For each tuple in `REQUIRED_LABELS`, run:
  - `gh label create <name> --repo <owner/repo> --color <color> --description <description>`
- Do **not** use `--force`.
- If command succeeds: continue.
- If command fails and output indicates “already exists” (case-insensitive): treat as skip, no warning.
- Any other failure (non-zero exit or process spawn error): print warning via `eprintln!` and continue.
- Function must not return an error that aborts startup.

3. Call this once in `execute_start()` in `src/cli/daemon.rs`:
- Place after `preflight_check_gh` success and repo slug resolution.
- Place before runtime config assembly / `runtime::run`.
- Run inside `spawn_blocking_op`.
- If join fails, warn and continue (do not abort daemon startup).

### Drift Guard
Add a unit test in `src/daemon/github.rs`:
- Asserts `REQUIRED_LABELS` has no duplicate names.
- Asserts required set contains all workflow lifecycle labels used by daemon code:
  - `ralph:ready`
  - `ralph:in-progress`
  - `ralph:completed`
  - `ralph:failed`
  - `ralph:aborted`

### Test and Mock Requirements
1. Update shared daemon mock scripts in `src/validate/mock_scripts.rs` to handle top-level `gh label create` command (success path by default):
- `daemon_mock_gh_script`
- `daemon_mock_gh_edit_pr_script`
- `daemon_mock_gh_rebase_script`

2. Update inline daemon mock scripts in `src/validate/tests_daemon.rs` that currently fail unknown top-level commands, adding a `label)` case before catch-all failure.

3. Add/extend validate conformance tests (in existing daemon validate module, or create/register a dedicated one) for:
- **Startup ensure call**: daemon single-iteration run issues exactly 5 `label create` invocations (one per required label).
- **Already-exists path**: mock returns non-zero with “already exists”; daemon continues; no failure warning for that label.
- **Hard failure path**: mock returns non-zero with another error; daemon continues; warning is emitted.

4. Keep existing daemon validate coverage green after changes.

### Acceptance Criteria
- Daemon startup attempts to ensure all five lifecycle labels exactly once per invocation before runtime loop begins.
- Existing labels are not modified (no `--force`); already-existing labels are skipped.
- Label ensure failures are warn-and-continue; startup proceeds.
- `REQUIRED_LABELS` is canonical for ensured lifecycle labels.
- Drift guard test exists and passes.
- Validate tests cover success, already-exists skip, and failure-resilience behavior.
- Full project checks pass:
  - `nix develop -c cargo check`
  - `nix develop -c cargo test`
  - `nix build -L`
  - `./result/bin/ralph validate --bin ./result/bin/ralph`