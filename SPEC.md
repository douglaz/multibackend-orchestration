Now I have full understanding of the codebase. Let me produce the revised specification.

Key findings for addressing the review issues:

1. **Runtime feasibility**: `daemon::execute()` is called synchronously from `async fn run()` at line 300 of `src/cli/mod.rs`. The daemon's `run()` in `runtime.rs` is entirely synchronous with `thread::sleep()`. Since it's called from within a tokio runtime context (the `#[tokio::main]`), `Handle::current().block_on()` would actually panic when called from a tokio runtime thread. The correct approach is to use `std::process::Command` (synchronous) directly, mirroring how `gh` is already called, rather than going through the async `Backend::execute()`.

2. **Configuration**: CLI config surfaces are already fully implemented — `workspace.daemon_refinement_enabled`, `workspace.daemon_refinement_backend`, `daemon.refinement_enabled`, `daemon.refinement_backend` are all handled in `src/cli/config.rs`.

3-6. Other issues need careful specification work.

Let me write the revised spec:

---

## Summary

Add a prompt refinement step to the daemon's issue-dispatch lifecycle. When the daemon claims a GitHub issue, it fetches the issue body, sends `"{title}\n\n{body}"` through a configurable LLM backend to produce a well-structured `ralph auto --idea` prompt, posts the refined prompt as a GitHub comment, and uses it as the `--idea` argument to the spawned subprocess. Refinement is enabled by default and degrades gracefully — on failure, the daemon falls back to the raw issue text.

## Acceptance Criteria

1. **Issue body fetched**: `poll_issues()` requests `"number,title,labels,body"` from `gh`. `GhIssue` and `RawGhIssue` include `body: Option<String>`. A `fetch_issue_body(owner, repo, issue_number) -> Result<(String, Option<String>)>` helper retrieves title+body on demand for legacy tasks. *(Already implemented in current codebase.)*

2. **Raw idea persisted on task**: `DaemonTask` includes `raw_idea: Option<String>` (with `#[serde(default)]` for backwards compatibility). On claim, `raw_idea` is set to `"{title}\n\n{body}"`. *(Already implemented in current codebase.)*

3. **Hydration of legacy tasks with deterministic failure policy**: `adopt_pending_tasks()` is the single authoritative hydration point for missing `raw_idea`. If `fetch_issue_body()` fails (GitHub unavailable, issue deleted, auth error), the task is **skipped** for this iteration with a warning logged. The task remains `Pending` and hydration is retried on subsequent iterations. After **3 consecutive hydration failures** for the same task, the daemon transitions the task to `Failed` with a log message explaining the reason. This prevents indefinite pending-task churn while tolerating transient GitHub outages.

4. **Refinement uses synchronous subprocess execution**: New `src/daemon/refine.rs` module with `refine_prompt(raw_idea, backend_spec, global_config) -> Result<String>`. Uses `std::process::Command` (synchronous) to invoke the backend CLI directly, bypassing the async `Backend::execute()` trait entirely. This avoids the `Handle::current().block_on()` panic that would occur when blocking from within the tokio runtime thread. The function constructs the same CLI command that `CliBackend` would (resolved command path, args, model flags, env vars, stdin pipe) but executes it synchronously with `wait_with_output()`. A dedicated `daemon_refinement_timeout` (default 120 seconds) is enforced via a background reaper thread that kills the child process if it exceeds the deadline. Output < 20 chars is rejected.

5. **Refinement timeout preserves daemon liveness**: The refinement subprocess timeout defaults to 120 seconds (configurable via `workspace.daemon_refinement_timeout`). This is separate from the backend's general `timeout_seconds` to ensure the daemon's synchronous dispatch path has a bounded, predictable blocking duration. The timeout is enforced via a spawned `std::thread` that sleeps for the timeout duration then kills the child process, allowing the main thread's `wait_with_output()` to return promptly.

6. **Refined prompt posted as comment (best-effort)**: Before spawning, the final idea text (refined or raw fallback) is posted via `post_idempotent_comment()` with phase `"refined-prompt"` (marker `<!-- ralph:task:<id>:refined-prompt -->`). The comment is posted **only when refinement is enabled**. When refinement is disabled, no `refined-prompt` comment is posted. When refinement fails and falls back to raw, the raw idea is posted as the comment (labeled as fallback). Comment failure logs a warning and does not block dispatch.

7. **Correct `spawn_ralph_auto` argv**: Uses `["auto", "--idea", &idea]` to match clap's `#[arg(long)]` declaration. *(Already implemented in current codebase.)*

8. **Graceful fallback**: If refinement fails (backend error, timeout, empty/short output), the daemon logs a warning and uses the raw `"{title}\n\n{body}"` string as `--idea`.

9. **Configuration**: Two existing fields following the flat `daemon_*` naming pattern in `[workspace]`:
   - `daemon_refinement_enabled: bool` (default `true`)
   - `daemon_refinement_backend: String` (default `"claude(sonnet)"`)

   Plus one new field:
   - `daemon_refinement_timeout: u64` (default `120`, in seconds)

   Project-level overrides in `[daemon]`: `refinement_enabled: Option<bool>`, `refinement_backend: Option<String>`, `refinement_timeout: Option<u64>`. Resolved through `resolve_daemon_config()` with project-override precedence. CLI config get/set support via `ralph config get/set` for all keys at both global and project scope. *(Global/project config plumbing for `refinement_enabled` and `refinement_backend` already exists; only `refinement_timeout` is new.)*

10. **Strict dispatch ordering**: `create_worktree` → `resolve raw_idea` → `refine_prompt` (if enabled) → `post refined-prompt comment` (best-effort, only if refinement enabled) → `spawn_ralph_auto` → CAS state update. Task remains `Pending` until successful spawn.

11. **Test fixture compatibility**: Existing daemon conformance tests that create pending tasks without `raw_idea` continue to work. The mock `gh` scripts' `issue view` handler already returns valid JSON for `title,body` requests. The `task_json()` test helper is updated to optionally include `raw_idea`. New tests use mock backends (shell scripts echoing refined prompts) rather than real LLM calls.

## Technical Approach

### 1. GhIssue body and fetch_issue_body (already implemented)

`GhIssue` and `RawGhIssue` already have `body: Option<String>`. `poll_issues()` already requests `"number,title,labels,body"`. `fetch_issue_body()` already exists in `src/daemon/github.rs:83-119`. No changes needed.

### 2. raw_idea on DaemonTask (already implemented)

`DaemonTask` already has `raw_idea: Option<String>` with `#[serde(default)]`. `poll_and_claim()` already stores `raw_idea` on claim. `adopt_pending_tasks()` already calls `fetch_and_persist_raw_idea()` for legacy tasks. No structural changes needed.

### 3. Hydration failure policy

Add a `hydration_failures: HashMap<String, u32>` to the daemon's runtime state (local to the `run()` function, not persisted). In `adopt_pending_tasks()`:

- On hydration failure, increment the counter for that task_id and `continue` (skip this iteration).
- If the counter reaches 3, transition the task to `Failed` via `store.update_task()` with a descriptive message logged to stderr: `"task {task_id} failed: could not fetch issue body after 3 attempts"`.
- On successful hydration, remove the task_id from the map.

This provides deterministic behavior: transient GitHub outages cause retries across daemon iterations, but permanently inaccessible issues are failed after 3 attempts.

### 4. New module: `src/daemon/refine.rs`

```rust
pub fn refine_prompt(
    raw_idea: &str,
    backend_spec: &str,
    global_config: &GlobalConfig,
    timeout_seconds: u64,
) -> Result<String>
```

**Why synchronous `std::process::Command` instead of async `Backend::execute()`**:

The daemon's `run()` function is entirely synchronous (it uses `thread::sleep()`, `std::process::Command` for `gh` calls, and `child.try_wait()` for process management). However, it is called from within the tokio runtime established by `#[tokio::main]` in `main.rs` → `async fn run()` → sync `daemon::execute()`. This means:

- `tokio::runtime::Runtime::new().block_on(...)` panics: "Cannot start a runtime from within a runtime."
- `tokio::runtime::Handle::current().block_on(...)` panics when called from an async context on a tokio worker thread.
- `tokio::task::block_in_place(...)` requires the multi-threaded runtime and has subtle constraints.

The simplest and most correct approach is to use `std::process::Command` directly, exactly as the daemon already does for all `gh` CLI calls. The backend CLI tools (`claude`, `codex`) are just shell commands that accept stdin and produce stdout — the async `Backend` trait adds no value here since we need synchronous blocking execution.

**Implementation**:

1. Parse `backend_spec` via `parse_backend_spec()` to get backend name + model.
2. Look up the backend config in `global_config.backends` (claude or codex).
3. Build a `std::process::Command` with the same args, env vars, and model flags that `claude::backend_from_config()` or `codex::backend_from_config()` would produce.
4. Pipe the refinement prompt (system prompt + raw_idea) to stdin.
5. Enforce timeout via a reaper thread: spawn a `std::thread` that sleeps for `timeout_seconds` then kills the child. The main thread calls `child.wait_with_output()`. If the reaper fires, `wait_with_output()` returns immediately with a non-zero/signal exit.
6. Validate output: if empty or < 20 chars, return `Err` to trigger fallback.

**Refinement system prompt** (hardcoded constant):

```
You are a prompt refinement assistant. Given a raw GitHub issue, produce a clear,
well-structured task description suitable for an AI coding agent.

Your output should include:
- A concise summary of what needs to be done
- Specific requirements extracted from the issue
- Acceptance criteria (what "done" looks like)
- Any constraints or considerations mentioned

Do NOT include commentary, greetings, or meta-discussion. Output only the refined
task description.
```

The full prompt sent to the backend is: `"{REFINEMENT_SYSTEM_PROMPT}\n\n---\n\n{raw_idea}"`.

### 5. Integration into `dispatch_task`

Modify `dispatch_task()` in `runtime.rs` to accept refinement config. Updated flow:

1. Create worktree (unchanged)
2. Determine raw idea: use `task.raw_idea` if `Some`, else call `fetch_and_persist_raw_idea()` (unchanged)
3. If `refinement_enabled`: call `refine_prompt(raw_idea, refinement_backend, global_config, refinement_timeout)`.
   - On success: use refined text as `idea`.
   - On error: log warning, use raw idea as `idea`.
4. If `refinement_enabled`: post `idea` as comment via `post_idempotent_comment()` with phase `"refined-prompt"` (best-effort, warn-and-continue on failure).
5. Spawn with `["auto", "--idea", &idea]` (unchanged)
6. CAS update (unchanged)

### 6. Configuration plumbing

**New field only** — `daemon_refinement_timeout`:

Add `daemon_refinement_timeout: u64` (default `120`) to `WorkspaceConfig` with `#[serde(default = "default_daemon_refinement_timeout")]`.

Add `refinement_timeout: Option<u64>` to `ProjectDaemonOverrides`.

Add `refinement_timeout: u64` to `EffectiveDaemonConfig`. Wire through `resolve_daemon_config()`.

Add `refinement_timeout: u64` to `DaemonRuntimeConfig`. Populate in `cli/daemon.rs`'s `execute_start()`.

Add CLI config get/set support:
- Global: `"workspace.daemon_refinement_timeout" => parse_u64(...)` in `set_global_value()`
- Project: `"daemon.refinement_timeout" => parse_optional_u64(...)` in `set_project_value()`
- Show/get: include `refinement_timeout` in the daemon JSON object for project-scoped views

*(The `refinement_enabled` and `refinement_backend` fields are already fully plumbed through config, CLI config get/set, project overrides, and effective config resolution.)*

### 7. Hydration failure tracking

Pass `&mut HashMap<String, u32>` (hydration failure counts) into `adopt_pending_tasks()`. The map is owned by the `run()` function and persists across loop iterations but not across daemon restarts (intentional — restart resets the counter, giving the task fresh attempts).

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/refine.rs` | **New.** `refine_prompt()` function using synchronous `std::process::Command`, `REFINEMENT_SYSTEM_PROMPT` constant, timeout enforcement via reaper thread, backend command construction helper. |
| `src/daemon/mod.rs` | Add `pub mod refine;`. |
| `src/daemon/runtime.rs` | Add `refinement_timeout: u64` to `DaemonRuntimeConfig`. Add `hydration_failures: HashMap<String, u32>` to `run()`. Update `adopt_pending_tasks()` to accept failure map, enforce 3-attempt limit, transition to `Failed` on exhaustion. Update `dispatch_task()` to call `refine::refine_prompt()` when enabled, post best-effort `refined-prompt` comment, use refined/raw idea for spawn. |
| `src/config/global.rs` | Add `daemon_refinement_timeout: u64` with `#[serde(default = "default_daemon_refinement_timeout")]`. Add default function. |
| `src/config/project.rs` | Add `refinement_timeout: Option<u64>` to `ProjectDaemonOverrides`. |
| `src/config/mod.rs` | Add `refinement_timeout: u64` to `EffectiveDaemonConfig`. Wire in `resolve_daemon_config()`. |
| `src/cli/daemon.rs` | Populate `refinement_timeout` in `DaemonRuntimeConfig` from `EffectiveDaemonConfig`. |
| `src/cli/config.rs` | Add `workspace.daemon_refinement_timeout` to `set_global_value()`. Add `daemon.refinement_timeout` to `set_project_value()`. Add `refinement_timeout` to daemon JSON in `execute_show()` and `execute_get()`. |
| `src/daemon/github.rs` | No changes (body fetching already implemented). |
| `src/daemon/process.rs` | No changes (argv already correct). |

## Testing Strategy

1. **`refine.rs` unit tests**:
   - Verify prompt construction wraps `raw_idea` with system prompt correctly.
   - Verify output validation rejects empty and < 20 char responses.
   - Verify command construction matches expected args/env for both claude and codex backend specs.
   - Verify timeout enforcement: use a mock command (`sleep 999`) with a 1-second timeout; assert error returned promptly.

2. **Config deserialization tests**:
   - TOML with/without `daemon_refinement_timeout` — verify default (`120`).
   - `resolve_daemon_config` with project override for `refinement_timeout`.
   - CLI `config get workspace.daemon_refinement_timeout` returns `120` by default.
   - CLI `config set workspace.daemon_refinement_timeout 60` persists correctly.

3. **Hydration failure policy tests**:
   - Mock `gh issue view` to fail. Run 3 iterations. Verify task transitions to `Failed` after 3rd failure.
   - Mock `gh issue view` to fail once then succeed. Verify task is hydrated and dispatched on 2nd iteration.
   - Legacy task with `raw_idea: None` and GitHub unavailable: verify the daemon does not panic, synthesize a fallback, or loop infinitely.

4. **Conformance/integration tests for dispatch flow**:
   - **Happy path (refinement enabled)**: Mock `gh` + mock refinement backend (shell script echoing structured prompt). Verify `ralph auto` receives refined prompt via `--idea`. Verify `refined-prompt` comment is posted.
   - **Refinement failure → raw fallback**: Mock backend that exits non-zero. Verify warning logged, raw idea used, dispatch completes. Verify `refined-prompt` comment posts raw idea with fallback label.
   - **Refinement timeout → raw fallback**: Mock backend that hangs (`sleep 999`). Set `daemon_refinement_timeout` to 2 seconds. Verify timeout fires, fallback to raw idea, dispatch completes within bounded time.
   - **Comment failure → dispatch continues**: Mock `gh issue comment` failure. Verify warning logged, spawn proceeds.
   - **Refinement disabled**: Set `daemon_refinement_enabled = false`. Verify no refinement call, no `refined-prompt` comment, raw idea used directly.
   - **Strict ordering**: Mock `gh` and refinement backend record call order to files. Assert sequence: `worktree → refine → comment → spawn`.

5. **Existing test compatibility**:
   - All 19 existing daemon conformance tests must continue to pass without modification.
   - Tests that create pending tasks without `raw_idea` already have mock `gh issue view` handlers that return valid JSON. The `runtime_adopt_pending_fetches_raw_idea_and_uses_idea_flag` test specifically validates this path.
   - Tests that use the standard `write_daemon_mock_gh()` script already handle `issue view` with `title,body` JSON format.
   - New `task_json()` overload or parameter allows setting `raw_idea` for tests that need it pre-populated.

6. **`DaemonTask` backwards compatibility**: Deserialize a `tasks.json` without `raw_idea` — verify default `None`. *(Already tested by existing task serialization tests.)*

## Out of Scope

- **Template-based refinement prompt**: The system prompt is a hardcoded constant, not loaded from a file. Customizable templates can be added later if needed.
- **Async daemon runtime**: The daemon remains fully synchronous. Refinement uses `std::process::Command` directly. Full async migration is separate work.
- **Streaming or multi-turn refinement**: Single-shot prompt → response only.
- **Refinement caching**: Each dispatch refines from scratch. `raw_idea` on `DaemonTask` preserves the input but refined output is not persisted.
- **New `daemon_refiner` role in `BackendRoleModels`**: Using a direct backend spec string (like `prompt_review_backend`) is simpler.
- **Comment correction on retry**: If a `refined-prompt` comment was already posted and a retry produces a different refined prompt, the existing comment is not updated. The marker prevents duplicates; the spawned process always uses the latest refinement result.
- **Persistent hydration failure counter**: The 3-attempt counter resets on daemon restart. This is intentional — a restart implies the operator is intervening, and giving fresh attempts is appropriate.