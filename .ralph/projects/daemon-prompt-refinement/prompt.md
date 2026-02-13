Now I have all the information I need. Let me write the revised spec addressing all 6 review issues.

## Summary

Add a prompt refinement step to the daemon's issue lifecycle. When the daemon claims a GitHub issue, it fetches the issue body, sends the title+body through an LLM backend to produce a well-structured `ralph auto --idea` prompt, posts the refined prompt as a comment on the issue, and uses it (instead of the raw issue text) as the `--idea` argument to the spawned `ralph auto` subprocess. Refinement is configurable and degrades gracefully — if it fails, the daemon falls back to the raw issue text.

## Acceptance Criteria

1. **Issue body fetched**: `poll_issues()` retrieves the issue body in addition to title/labels. The `GhIssue` struct gains a `body: Option<String>` field. `RawGhIssue` gains `body: Option<String>` (nullable in GitHub's JSON).
2. **Refinement produces a structured prompt**: A new function sends `"{title}\n\n{body}"` to a configured backend with a system prompt instructing it to rewrite the text as a well-structured `ralph auto --idea` prompt with clarified requirements and acceptance criteria. The raw output string from the backend is the refined prompt.
3. **Refined prompt posted as comment (best-effort)**: Before spawning `ralph auto`, the refined prompt is posted to the issue via `post_idempotent_comment()` with phase `"refined-prompt"` (marker `<!-- ralph:task:<task_id>:refined-prompt -->`). If the comment API call fails, a warning is logged and dispatch continues — comment posting must never block or abort the dispatch flow.
4. **Refined prompt used as `--idea`**: `spawn_ralph_auto()` passes the refined prompt (or raw fallback) using `["auto", "--idea", &idea]` instead of the current `["auto", idea]` positional invocation. This matches `ralph auto`'s required `--idea` long flag.
5. **Graceful fallback**: If refinement fails (backend error, timeout, empty output), the daemon logs a warning and falls back to using `"{title}\n\n{body}"` as the idea.
6. **Configuration**: Two new fields in `[daemon]` configuration (workspace-level `WorkspaceConfig`, project-level `ProjectDaemonOverrides`, resolved via `EffectiveDaemonConfig`):
   - `daemon_refinement_enabled: bool` (default `true`)
   - `daemon_refinement_backend: String` (default `"claude(sonnet)"`)

   These follow the same pattern as `prompt_review_backend`: a full backend spec string, not a role-based lookup. No new role is added to `BackendRoleModels`.
7. **Timing & ordering**: The strict dispatch sequence is: `create_worktree` → `refine_prompt` (if enabled) → `post refined-prompt comment` (best-effort) → `spawn_ralph_auto` → CAS state update. Refinement and comment posting occur while the task is in `Pending` state. The task transitions to `InProgress` only upon successful spawn, which is unchanged from current behavior.
8. **`spawn_ralph_auto` argv correctness**: The spawned command must be `["auto", "--idea", <idea_string>]`, not positional `["auto", <idea_string>]`. Tests must assert the exact argv structure.
9. **Restart with missing `raw_idea`**: When the daemon restarts and re-adopts a pending task where `raw_idea` is `None` (legacy or pre-refinement task), the daemon fetches the issue title+body from GitHub on demand via a new `fetch_issue_body()` helper. It must never fall back to the synthetic `"Implement task {task_id}"` string.

## Technical Approach

### 1. Extend `GhIssue` with body

Add `body: Option<String>` to `GhIssue` and `body: Option<String>` to `RawGhIssue`. Update `poll_issues()` to request `"number,title,labels,body"` from `gh issue list --json`. The body is nullable (GitHub issues can have empty bodies), so `Option<String>` is correct at both layers. The body is fetched for all polled issues but only used when an issue is actually claimed — acceptable since `poll_issues` is bounded to 100 issues and the body is a lightweight string field in `gh`'s JSON output.

Add a new `fetch_issue_body()` function in `github.rs`:

```rust
pub fn fetch_issue_body(owner: &str, repo: &str, issue_number: u32) -> Result<(String, Option<String>)>
```

This calls `gh issue view <number> --repo owner/repo --json title,body` and returns `(title, body)`. It is used only on restart for legacy tasks where `raw_idea` is `None`, avoiding the synthetic fallback.

### 2. Propagate issue title+body to dispatch

Add `raw_idea: Option<String>` to `DaemonTask`. This field is `Option` for backwards compatibility with existing `tasks.json` files (serde will deserialize missing field as `None`).

**After claiming**, compose `raw_idea = format!("{}\n\n{}", issue.title, issue.body.unwrap_or_default())` and store it on the `DaemonTask` before persisting.

**On restart/re-adoption** (`adopt_pending_tasks`): If `task.raw_idea` is `None`, call `fetch_issue_body(task.owner, task.repo, task.issue_number)` to retrieve the title+body, compose the raw idea, and update the task's `raw_idea` field in the store. If the fetch fails, log a warning and use `"{title}\n\n{body}"` with whatever partial data is available (title from task_id parsing if necessary), but never synthesize `"Implement task {task_id}"`.

### 3. Fix `spawn_ralph_auto` argument passing

Change `process.rs`:

```rust
cmd.args(["auto", "--idea", idea])
```

This matches `ralph auto`'s `#[arg(long)]` declaration for `idea`. The current positional invocation `["auto", idea]` would cause clap to reject the argument. This is a pre-existing latent bug — the daemon has never successfully dispatched a real `ralph auto` run because the argv is wrong.

### 4. New module: `src/daemon/refine.rs`

```rust
/// Refine raw issue text into a structured ralph auto prompt.
///
/// Uses `tokio::runtime::Handle::current().block_on()` since the daemon's
/// synchronous code runs on a thread within the existing tokio runtime
/// (the `#[tokio::main]` in main.rs). Creating a nested `Runtime::new()`
/// would panic; `Handle::current().block_on()` is safe from a blocking
/// context within an active runtime.
pub fn refine_prompt(
    raw_idea: &str,
    backend_spec: &str,
    global_config: &GlobalConfig,
) -> Result<String>
```

**Runtime safety (addressing review issue #1)**: The daemon's `execute_start()` is a synchronous function, but it runs inside the tokio runtime established by `#[tokio::main]` in `main.rs` → async `cli::run()` → sync `daemon::execute()`. Creating a new `tokio::runtime::Runtime::new()` inside this context would panic with "Cannot start a runtime from within a runtime."

The correct approach is `tokio::runtime::Handle::current().block_on(backend.execute(&prompt))`. This obtains a handle to the already-running tokio runtime and blocks the current thread waiting for the future. This is safe because:
- The daemon loop runs on the main thread, not inside a tokio task (it's called synchronously from an async function).
- `Handle::block_on` is designed for exactly this pattern: blocking synchronous code that needs to call async functions within an active runtime.
- The daemon is single-threaded and doesn't hold any tokio mutexes during this call.

Implementation:
- Parse `backend_spec` using `parse_backend_spec()`.
- Build a `CliBackend` via `claude::backend_from_config()` or `codex::backend_from_config()`, injecting the model from the spec.
- Construct the refinement prompt wrapping `raw_idea` with the system prompt.
- Execute via `tokio::runtime::Handle::current().block_on(backend.execute(&prompt))`.
- Validate output: if empty or < 20 chars, return `Err` to trigger fallback.

The refinement system prompt (hardcoded constant — a daemon-internal concern, not a user-facing workflow template):

```
You are a prompt refinement assistant. Rewrite the following GitHub issue into a
clear, structured task description suitable for an autonomous coding agent.

Include:
- A concise summary of what needs to be done
- Specific requirements and constraints
- Acceptance criteria as a checklist

Do NOT include meta-commentary. Output ONLY the refined task description.

--- ISSUE ---
{raw_idea}
```

### 5. Integration into `dispatch_task`

Modify `dispatch_task` in `runtime.rs`. The function gains two new parameters: the effective daemon config fields (`refinement_enabled`, `refinement_backend`) and a reference to `GlobalConfig` for backend construction.

The strict ordering is:

```
1. Create worktree (existing)
2. Determine raw_idea:
   a. Use task.raw_idea if Some
   b. Else fetch from GitHub via fetch_issue_body() and update task in store
   c. (Never fall back to "Implement task {task_id}")
3. If refinement enabled:
   a. Call refine::refine_prompt(raw_idea, refinement_backend, global_config)
   b. On success: idea = refined_prompt
   c. On error: log warning, idea = raw_idea
4. Post prompt as comment (best-effort):
   a. Call post_idempotent_comment(..., "refined-prompt", &idea)
   b. On error: log warning, continue (never abort dispatch)
5. Spawn child with idea using ["auto", "--idea", &idea]
6. CAS update (existing)
```

**Comment idempotency and correction (addressing review issue #4)**: `post_idempotent_comment` checks for an existing marker before posting. This means:
- If dispatch is retried after a comment was already posted (e.g., first attempt posted raw fallback, second attempt produces refined prompt), the second comment will not be posted because the marker already exists.
- This is acceptable. The first posted prompt (whether refined or raw fallback) is the authoritative record. Attempting to "correct" a previously-posted comment would add complexity and create confusing comment histories. The spawned `ralph auto` process always receives the latest idea regardless of what was commented.

### 6. Configuration plumbing

Add to `WorkspaceConfig` (`config/global.rs`):
```rust
#[serde(default = "default_daemon_refinement_enabled")]
pub daemon_refinement_enabled: bool,
#[serde(default = "default_daemon_refinement_backend")]
pub daemon_refinement_backend: String,
```

Defaults: `true` and `"claude(sonnet)"`.

Add to `ProjectDaemonOverrides` (`config/project.rs`):
```rust
pub refinement_enabled: Option<bool>,
pub refinement_backend: Option<String>,
```

Add to `EffectiveDaemonConfig` (`config/mod.rs`):
```rust
pub refinement_enabled: bool,
pub refinement_backend: String,
```

Wire through `resolve_daemon_config()` with standard project-override-or-workspace-default precedence.

Add to `DaemonRuntimeConfig` (`daemon/runtime.rs`):
```rust
pub refinement_enabled: bool,
pub refinement_backend: String,
pub global_config: GlobalConfig,
```

Populate in `cli/daemon.rs`'s `execute_start()` from `EffectiveDaemonConfig` and the loaded `GlobalConfig`.

**TOML schema** — configuration lives under the existing flat `[workspace]` section, consistent with `daemon_poll_seconds`, `daemon_max_concurrent`, etc.:
```toml
[workspace]
daemon_refinement_enabled = true
daemon_refinement_backend = "claude(sonnet)"
```

Project-level override in project config:
```toml
[daemon]
refinement_enabled = false
refinement_backend = "codex(gpt-5.3-codex-medium)"
```

No new `daemon_refiner` role is added to `BackendRoleModels`. The full backend spec string approach (like `prompt_review_backend`) is simpler and sufficient.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/refine.rs` | **New**. `refine_prompt()` function, `REFINEMENT_SYSTEM_PROMPT` constant, backend construction helper using `Handle::current().block_on()`. |
| `src/daemon/mod.rs` | Add `pub mod refine;`. Add `raw_idea: Option<String>` to `DaemonTask` (with `#[serde(default)]`). |
| `src/daemon/github.rs` | Add `body: Option<String>` to `GhIssue` and `RawGhIssue`. Update `poll_issues()` JSON fields to include `body`. Add `fetch_issue_body()` function for on-demand title+body retrieval. |
| `src/daemon/runtime.rs` | Add `refinement_enabled`, `refinement_backend`, and `global_config` to `DaemonRuntimeConfig`. Modify `poll_and_claim()` to store `raw_idea` on `DaemonTask`. Modify `adopt_pending_tasks()` to fetch issue body for legacy tasks with `raw_idea == None`. Modify `dispatch_task()` to call `refine::refine_prompt()`, post best-effort comment, and pass idea to spawn. |
| `src/daemon/process.rs` | Fix `spawn_ralph_auto()` to use `["auto", "--idea", idea]` instead of `["auto", idea]`. |
| `src/config/global.rs` | Add `daemon_refinement_enabled` and `daemon_refinement_backend` to `WorkspaceConfig` with serde defaults. Add defaults to `Default for GlobalConfig`. |
| `src/config/project.rs` | Add `refinement_enabled: Option<bool>` and `refinement_backend: Option<String>` to `ProjectDaemonOverrides`. |
| `src/config/mod.rs` | Add `refinement_enabled` and `refinement_backend` to `EffectiveDaemonConfig`. Wire in `resolve_daemon_config()`. |
| `src/cli/daemon.rs` | Populate `refinement_enabled`, `refinement_backend`, and `global_config` on `DaemonRuntimeConfig` from `EffectiveDaemonConfig` and workspace config. |

## Testing Strategy

1. **Unit tests for `refine.rs`**:
   - Test refinement prompt construction (verify the system prompt wraps raw_idea correctly).
   - Test output validation (empty output → error, short output < 20 chars → error, valid output → Ok).

2. **Unit tests for config**:
   - Deserialize `ralph.toml` with `daemon_refinement_enabled = false` and `daemon_refinement_backend = "codex"` — verify fields populate.
   - Deserialize without the fields — verify defaults (`true`, `"claude(sonnet)"`).
   - `resolve_daemon_config` with project overrides for refinement fields.

3. **Unit tests for `GhIssue` body**:
   - Verify `poll_issues` JSON deserialization handles `body` field present, `body: null`, and `body` absent — all should succeed with appropriate `Option<String>` values.

4. **Unit test for `spawn_ralph_auto` argv** (addresses review issue #2):
   - Assert that the spawned command arguments are exactly `["auto", "--idea", <idea_string>]`.
   - This can be tested by inspecting the `Command` builder or via a mock binary that echoes its argv to a file.

5. **`DaemonTask` backwards compatibility**:
   - Deserialize a `tasks.json` without `raw_idea` field — verify it defaults to `None`.
   - Deserialize with `raw_idea: "some text"` — verify it round-trips.

6. **Integration/conformance tests for dispatch flow** (extend existing daemon conformance tests):

   a. **Strict ordering** (addresses review issue #6): Assert the sequence `claim → worktree → refine → refined-prompt comment → spawn` via ordered mock assertions. The mock `gh` binary and mock refinement backend record call order; the test verifies the exact sequence.

   b. **Happy path with refinement**: Mock `gh` CLI and refinement backend. Verify that when refinement is enabled, the `ralph auto` subprocess receives the exact refined prompt string via `--idea` (assert the argv of the spawned process).

   c. **Refinement failure → raw fallback**: Mock refinement backend to return an error. Verify: (1) warning is logged, (2) raw idea is used as `--idea` argument, (3) dispatch completes successfully.

   d. **Comment API failure → dispatch continues** (addresses review issue #4): Mock `gh issue comment` to fail. Verify: (1) warning is logged, (2) `ralph auto` is still spawned with the correct idea, (3) dispatch does not abort.

   e. **Refinement disabled**: Set `daemon_refinement_enabled = false`. Verify no refinement call is made and the raw idea is used directly as `--idea`.

   f. **Restart with legacy task** (addresses review issue #5): Create a `tasks.json` with a pending task lacking `raw_idea`. Mock `gh issue view` to return title+body. Verify `adopt_pending_tasks` fetches the body and populates `raw_idea` before dispatching, and the spawned process receives the fetched title+body (not `"Implement task {task_id}"`).

   g. **Retry scenario — comment idempotency** (addresses review issue #6): Simulate: first dispatch attempt refines and posts comment, then process crashes. Second adoption re-refines (possibly different output). Verify the marker-based idempotency prevents a duplicate comment, and the spawned process uses the latest refinement.

## Out of Scope

- **Template-based refinement prompt**: The refinement system prompt is hardcoded, not loaded from a template file. It's a daemon-internal concern. If users need customization, a template can be added later.
- **Async daemon runtime**: The daemon runtime is synchronous. The refinement backend call uses `Handle::current().block_on()` for the async `Backend::execute()`. Migrating the daemon to fully async is a separate effort.
- **Streaming or multi-turn refinement**: Single-shot prompt → response. No iterative refinement or human-in-the-loop.
- **Refinement caching**: Each dispatch refines from scratch. No caching of refined prompts across restarts (the `raw_idea` field on `DaemonTask` allows re-refinement on restart).
- **New `daemon_refiner` role in `BackendRoleModels`**: Using a direct backend spec string (like `prompt_review_backend`) instead. Simpler, fewer touchpoints.
- **Fetching issue body separately for initial claim**: The body is fetched inline with `poll_issues` rather than via a separate `gh issue view` call. `fetch_issue_body()` is only used on restart for legacy tasks with missing `raw_idea`.
- **Comment correction on retry**: If a comment was already posted (e.g., with raw fallback text) and a subsequent dispatch produces a different refined prompt, the existing comment is not updated or replaced. The marker prevents duplicates, and the spawned process always uses the latest refinement. Comment history is append-only.