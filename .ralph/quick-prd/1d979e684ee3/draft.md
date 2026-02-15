I now have all the information needed. Let me write the spec.

---

## Summary

Add an `ensure_labels` preflight step to the daemon startup that creates the five Ralph workflow labels (`ralph:ready`, `ralph:in-progress`, `ralph:completed`, `ralph:failed`, `ralph:aborted`) using `gh label create --force`. This runs once at startup in `execute_start()` (after `preflight_check_gh` succeeds and the repo slug is resolved, before entering `runtime::run`), using the best-effort/warn-and-continue pattern so label creation failures never block the daemon.

## Acceptance Criteria

- All five Ralph labels are created automatically during daemon startup before the runtime loop begins.
- The operation is idempotent — runs `gh label create --force` so existing labels are silently updated, never errored.
- Label creation failures log a warning via `eprintln!` but do **not** abort daemon startup.
- Freshly-cloned repos (with no pre-existing labels) can immediately have issues claimed without manual label setup.
- Existing integration tests continue to pass (mock gh script handles the new `label create` subcommand).
- A unit-style integration test verifies the label creation function calls `gh label create` with correct arguments for each label.

## Technical Approach

### 1. Define the canonical label list as a constant in `src/daemon/github.rs`

```rust
pub const REQUIRED_LABELS: &[(&str, &str, &str)] = &[
    ("ralph:ready",       "#0e8a16", "Issue is ready for Ralph daemon pickup"),
    ("ralph:in-progress", "#fbca04", "Ralph daemon is working on this issue"),
    ("ralph:completed",   "#1d76db", "Ralph daemon completed this issue"),
    ("ralph:failed",      "#d93f0b", "Ralph daemon task failed"),
    ("ralph:aborted",     "#e4e669", "Ralph daemon task was aborted"),
];
```

A single source of truth replaces the scattered string literals used in `claim_issue`, `update_terminal_labels_best_effort`, `update_abort_labels_best_effort`, and `complete_task`. The `(name, color, description)` tuple provides reasonable defaults for freshly-created labels.

### 2. Add `ensure_labels_best_effort` function in `src/daemon/github.rs`

```rust
pub fn ensure_labels_best_effort(owner: &str, repo: &str) {
    let full_repo = format!("{owner}/{repo}");
    for &(name, color, description) in REQUIRED_LABELS {
        let output = Command::new("gh")
            .args([
                "label", "create", name,
                "--repo", &full_repo,
                "--color", color,
                "--description", description,
                "--force",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                eprintln!(
                    "warning: failed to ensure label '{}' on {}: {}",
                    name, full_repo,
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to run gh label create for '{}' on {}: {}",
                    name, full_repo, err
                );
            }
        }
    }
}
```

This follows the exact same `match output` best-effort pattern as the existing `update_terminal_labels_best_effort` and `update_abort_labels_best_effort` functions. The `--force` flag on `gh label create` makes it idempotent — it updates the existing label instead of erroring.

### 3. Call the new function from `execute_start` in `src/cli/daemon.rs`

Insert the call after the repo slug is resolved and parsed (`owner`/`repo_name` are available) but before `runtime::run()`. Wrap in `spawn_blocking_op` since it does synchronous I/O:

```rust
// Ensure required Ralph labels exist (best-effort, non-blocking)
{
    let owner = owner.clone();
    let repo_name = repo_name.clone();
    let _ = spawn_blocking_op(move || {
        github::ensure_labels_best_effort(&owner, &repo_name);
        Ok(())
    }).await;
}
```

This placement means labels are ensured once per daemon invocation, not once per poll cycle.

### 4. Update mock gh script in `src/validate/mock_scripts.rs`

Add a `label)` case to `daemon_mock_gh_script()` at the top level of the `case "$1"` block:

```bash
label)
    # label create --force — always succeed
    exit 0
    ;;
```

This prevents existing integration tests from failing when the new startup code runs against the mock.

### 5. (Optional) Replace scattered label string literals

Replace hardcoded `"ralph:in-progress"`, `"ralph:completed"`, etc. across `github.rs`, `runtime.rs`, and `mod.rs` with references to `REQUIRED_LABELS` entries or derived constants. This is a follow-up cleanup — the core feature works without it.

## Files & Modules

| File | Change | Scope |
|---|---|---|
| `src/daemon/github.rs` | Add `REQUIRED_LABELS` constant and `ensure_labels_best_effort()` function | ~30 lines |
| `src/cli/daemon.rs` | Add `ensure_labels_best_effort` call in `execute_start()` between repo resolution and `runtime::run()` | ~8 lines |
| `src/validate/mock_scripts.rs` | Add `label)` handler to `daemon_mock_gh_script()` (and any variant mock scripts like the one at line 776) | ~4 lines per mock |

## Testing Strategy

1. **Mock script update** — All existing daemon integration tests (`src/validate/tests_daemon.rs`) use mock gh scripts. Adding the `label)` handler to the mock ensures they pass with the new startup step. No existing test behavior changes.

2. **New integration test** — Add a test in `src/validate/tests_daemon.rs` that:
   - Configures a mock gh script that logs `label create` invocations to a temp file (via env var like `MOCK_GH_LABEL_LOG`).
   - Starts the daemon with `--single-iteration`.
   - Asserts the log file contains exactly 5 `gh label create --force` calls, one for each required label.

3. **Failure-resilience test** — Add a test where the mock gh script returns a non-zero exit for `label create`. Assert the daemon still starts and completes its poll/claim cycle (the warning is logged but startup is not blocked).

4. **Unit test for `REQUIRED_LABELS`** — A simple `#[test]` that asserts the constant contains all five expected label names and that none are duplicated. Guards against accidental removal.

## Out of Scope

- **Removing labels** — The daemon will not delete stale or renamed labels from the repository.
- **Customizable label colors/descriptions** — Colors and descriptions are hardcoded defaults. Users can manually adjust them in GitHub after creation.
- **User-configurable label names** — The `ralph:*` prefix and label names remain hardcoded. Making label names configurable is a separate feature.
- **Replacing scattered string literals** — Consolidating hardcoded label strings in `claim_issue`, `update_terminal_labels_best_effort`, etc. to reference the new `REQUIRED_LABELS` constant is a follow-up cleanup, not part of this change.
- **Label creation for multi-repo setups** — The daemon operates on a single `owner/repo`; multi-repo label management is not addressed.
- **GitHub API token permission checks** — If the token lacks `label:write` scope, the best-effort pattern handles it (warns and continues). No proactive permission validation is added.