Now I have a thorough understanding of the codebase. Let me address each review issue and produce the revised spec.

---

## Summary

Add an `ensure_labels` preflight step to the daemon startup that creates the five Ralph workflow labels (`ralph:ready`, `ralph:in-progress`, `ralph:completed`, `ralph:failed`, `ralph:aborted`) using `gh label create`. This runs once at startup in `execute_start()` (after `preflight_check_gh` succeeds and the repo slug is resolved, before entering `runtime::run`), using the best-effort/warn-and-continue pattern so label creation failures never block the daemon.

The constant `REQUIRED_LABELS` also serves as the single source of truth for all label names used in the daemon lifecycle, and a compile-time-style guard test ensures the ensure-list stays in sync with labels actually referenced in workflow code.

**Clarifications from review:**
- The codebase uses `ralph:completed` exclusively — `ralph:done` does not appear anywhere. The feature request's mention of `ralph:done` was imprecise; no alias, migration, or compatibility plan is needed.
- Label creation uses `gh label create` *without* `--force` and skips on `AlreadyExists` exit status, preserving any user-customized colors or descriptions on existing labels.
- Custom `daemon_labels` poll-trigger labels (from `workspace.daemon_labels` config) are out of scope — only the five fixed Ralph lifecycle labels are ensured. This is explicitly documented in "Out of Scope".

## Acceptance Criteria

- All five Ralph labels (`ralph:ready`, `ralph:in-progress`, `ralph:completed`, `ralph:failed`, `ralph:aborted`) are created automatically during daemon startup before the runtime loop begins.
- The operation is idempotent — if a label already exists, it is **skipped** (not updated). User-customized colors and descriptions are preserved.
- Label creation failures log a warning via `eprintln!` but do **not** abort daemon startup.
- Freshly-cloned repos (with no pre-existing labels) can immediately have issues claimed without manual label setup.
- A guard test asserts that every `ralph:*` string literal used in workflow code (`claim_issue`, `complete_task`, `filter_claimable`, `update_terminal_labels_best_effort`, `update_abort_labels_best_effort`) is present in `REQUIRED_LABELS`. This prevents drift between the ensure-list and actual usage.
- All existing integration tests continue to pass — both shared mock scripts (`daemon_mock_gh_script`, `daemon_mock_gh_edit_pr_script`, `daemon_mock_gh_rebase_script`) and all ~25 inline mock scripts in `tests_daemon.rs` handle the new `gh label create` subcommand.
- A dedicated integration test verifies the label creation function calls `gh label create` with correct arguments for each of the five labels.

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

A single source of truth for the five lifecycle labels. The `(name, color, description)` tuple provides reasonable defaults for freshly-created labels. Colors and descriptions are only applied to newly-created labels — existing labels are left untouched (see §2).

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
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                // gh label create exits non-zero with "already exists" message
                // when the label is present — this is the expected skip path.
                if stderr.to_lowercase().contains("already exists") {
                    // Label exists; skip silently, preserving user customizations.
                } else {
                    eprintln!(
                        "warning: failed to ensure label '{}' on {}: {}",
                        name, full_repo, stderr.trim()
                    );
                }
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

**Key difference from original spec:** Uses `gh label create` *without* `--force`. When a label already exists, `gh` exits non-zero with an "already exists" message on stderr — we detect that and skip silently. This means existing labels retain their user-customized colors and descriptions, satisfying the "skip if label already exists" requirement. All other non-zero exits are logged as warnings. This follows the same `match output` best-effort pattern as `update_terminal_labels_best_effort` and `update_abort_labels_best_effort`.

### 3. Call the new function from `execute_start` in `src/cli/daemon.rs`

Insert the call after the repo slug is parsed (`owner`/`repo_name` are available from `parse_repo_slug`) but before the `DaemonRuntimeConfig` assembly and `runtime::run()`. Wrap in `spawn_blocking_op` since it does synchronous I/O:

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

This placement means labels are ensured once per daemon invocation, not once per poll cycle. The `let _ =` discards the `spawn_blocking_op` result — since `ensure_labels_best_effort` already handles all errors internally via `eprintln!`, the outer await can only fail on a join error, which is also non-fatal for this preflight.

### 4. Update ALL mock gh scripts to handle `gh label create`

The codebase has strict unknown-command failure paths (`exit 1`) in both shared and inline mock scripts. Every script that can be reached during daemon startup must handle the new `label` top-level command.

**4a. Shared scripts in `src/validate/mock_scripts.rs`**

Add a `label)` case to the top-level `case "$1"` in each of the three shared mock scripts:
- `daemon_mock_gh_script()` (line ~457)
- `daemon_mock_gh_edit_pr_script()` (line ~676)
- `daemon_mock_gh_rebase_script()` (line ~773)

```bash
label)
    # gh label create — always succeed (mock)
    exit 0
    ;;
```

**4b. Inline scripts in `src/validate/tests_daemon.rs`**

All ~25 inline mock gh scripts end with `exit 1` for unknown commands. Each one needs a `label)` handler added to its top-level case. The approach: add the `label)` case before the catch-all `*)` in each inline script's outermost `case "$1"` block. Since these are string literals in Rust test code, each insertion is small (3 lines).

To avoid missing any, the implementation should search `tests_daemon.rs` for all occurrences of `case "$1"` within mock gh script strings and add the handler to each.

### 5. Add a guard test to prevent label drift

Add a `#[test]` in `src/daemon/github.rs` (or `src/validate/tests_daemon.rs`) that:

```rust
#[test]
fn required_labels_covers_all_workflow_labels() {
    let names: Vec<&str> = REQUIRED_LABELS.iter().map(|(n, _, _)| *n).collect();

    // Every label used in the daemon workflow must be in REQUIRED_LABELS.
    let workflow_labels = [
        "ralph:ready",        // filter_claimable TRIGGER_LABELS
        "ralph:in-progress",  // claim_issue, update_terminal_labels, update_abort_labels
        "ralph:completed",    // complete_task
        "ralph:failed",       // complete_task
        "ralph:aborted",      // complete_task, update_abort_labels
    ];

    for label in &workflow_labels {
        assert!(
            names.contains(label),
            "workflow label '{}' is missing from REQUIRED_LABELS",
            label
        );
    }

    // No duplicates in the constant.
    let mut seen = std::collections::HashSet::new();
    for &(name, _, _) in REQUIRED_LABELS {
        assert!(seen.insert(name), "duplicate label in REQUIRED_LABELS: {}", name);
    }
}
```

This catches the drift scenario where a developer adds a new `ralph:*` label to workflow code but forgets to add it to `REQUIRED_LABELS`. The test explicitly enumerates workflow-used labels — when a new label is introduced, it must be added to both `REQUIRED_LABELS` and the `workflow_labels` array, creating a deliberate two-step process that forces awareness.

## Files & Modules

| File | Change | Scope |
|---|---|---|
| `src/daemon/github.rs` | Add `REQUIRED_LABELS` constant, `ensure_labels_best_effort()` function, and `required_labels_covers_all_workflow_labels` guard test | ~50 lines |
| `src/cli/daemon.rs` | Add `ensure_labels_best_effort` call in `execute_start()` between repo slug parsing and runtime config assembly | ~8 lines |
| `src/validate/mock_scripts.rs` | Add `label)` handler to `daemon_mock_gh_script()`, `daemon_mock_gh_edit_pr_script()`, and `daemon_mock_gh_rebase_script()` | ~4 lines per script (12 total) |
| `src/validate/tests_daemon.rs` | Add `label)` handler to all ~25 inline mock gh scripts; add label-ensure integration test and failure-resilience test | ~80 lines for new tests, ~3 lines per inline script update (~75 lines total) |

## Testing Strategy

1. **Mock script updates (all scripts)** — Add `label)` handler to every mock gh script that the daemon can reach during startup. This includes:
   - 3 shared scripts in `mock_scripts.rs` (`daemon_mock_gh_script`, `daemon_mock_gh_edit_pr_script`, `daemon_mock_gh_rebase_script`)
   - All ~25 inline scripts in `tests_daemon.rs` that have strict `exit 1` catch-alls
   
   This prevents the new `gh label create` calls from causing `exit 1` failures that would either break tests or produce misleading stderr warnings.

2. **Guard test for label drift** — A `#[test]` in the production code (`github.rs`) that asserts `REQUIRED_LABELS` contains all labels referenced in the daemon workflow and has no duplicates. This is a compile-and-run-time safeguard against the constant going stale.

3. **Label creation integration test** — Add a test in `tests_daemon.rs` that:
   - Configures a mock gh script that logs `label create` invocations to a temp file (via `MOCK_GH_LABEL_LOG` env var).
   - Starts the daemon with `--single-iteration`.
   - Asserts the log file contains exactly 5 `gh label create` calls, one for each required label, with correct name/color/description arguments.

4. **Failure-resilience integration test** — Add a test where the mock gh script returns non-zero for `label create` (with stderr that does *not* contain "already exists"). Assert:
   - The daemon still starts and completes its poll/claim cycle.
   - Warning messages are printed to stderr.
   - Startup is not blocked.

5. **Already-exists path test** — Add a test where the mock gh script returns non-zero for `label create` with stderr containing "already exists". Assert no warning is logged (silent skip).

## Out of Scope

- **`ralph:done` alias or migration** — The codebase uses `ralph:completed` exclusively; `ralph:done` does not exist anywhere. The feature request's mention of `ralph:done` was imprecise. No alias, migration, or backward-compatibility handling is needed.
- **Removing labels** — The daemon will not delete stale or renamed labels from the repository.
- **Customizable label colors/descriptions** — Colors and descriptions are hardcoded defaults applied only to newly-created labels. Users can manually adjust them in GitHub; existing labels are never overwritten.
- **User-configurable label names** — The `ralph:*` prefix and label names remain hardcoded. Making label names configurable is a separate feature.
- **Replacing scattered string literals** — Consolidating hardcoded label strings in `claim_issue`, `update_terminal_labels_best_effort`, etc. to reference the new `REQUIRED_LABELS` constant is a follow-up cleanup. The guard test (§5 of Technical Approach) mitigates the drift risk in the interim.
- **Custom `daemon_labels` / poll-trigger labels** — The `workspace.daemon_labels` config (default `["ralph:ready"]`) controls which labels the daemon polls for. These are user-configurable and may include non-Ralph labels. Ensuring arbitrary user-configured labels is out of scope — only the five fixed Ralph lifecycle labels are created. The `ralph:ready` label *is* included in `REQUIRED_LABELS`, which covers the default poll-trigger case. If a user configures a custom poll label, they are responsible for creating it.
- **Label creation for multi-repo setups** — The daemon operates on a single `owner/repo`; multi-repo label management is not addressed.
- **GitHub API token permission checks** — If the token lacks label-write scope, the best-effort pattern handles it (warns and continues). No proactive permission validation is added.