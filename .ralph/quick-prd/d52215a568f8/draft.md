I now have complete understanding. Here is the engineering specification:

---

## Summary

Add a prompt refinement step to the daemon's issue-dispatch lifecycle. When the daemon claims a GitHub issue, it fetches the issue body, sends `"{title}\n\n{body}"` through a configurable LLM backend to produce a well-structured `ralph auto --idea` prompt, posts the refined prompt as a GitHub comment, and uses it as the `--idea` argument to the spawned subprocess. Refinement is opt-in-by-default and degrades gracefully — on failure, the daemon falls back to the raw issue text.

## Acceptance Criteria

1. **Issue body fetched**: `poll_issues()` requests `"number,title,labels,body"` from `gh`. `GhIssue` and `RawGhIssue` gain `body: Option<String>`. A new `fetch_issue_body(owner, repo, issue_number) -> Result<(String, Option<String>)>` helper retrieves title+body on demand for legacy tasks.

2. **Raw idea persisted on task**: `DaemonTask` gains `raw_idea: Option<String>` (with `#[serde(default)]` for backwards compatibility). On claim, `raw_idea` is set to `"{title}\n\n{body}"`. The synthetic `"Implement task {task_id}"` fallback is eliminated — on restart with missing `raw_idea`, the daemon fetches from GitHub.

3. **Refinement produces a structured prompt**: New `src/daemon/refine.rs` module with `refine_prompt(raw_idea, backend_spec, global_config) -> Result<String>`. Uses `tokio::runtime::Handle::current().block_on()` for the async backend call. Hardcoded system prompt instructs the LLM to rewrite raw issue text into a clear task description with requirements and acceptance criteria. Output < 20 chars is rejected.

4. **Refined prompt posted as comment (best-effort)**: Before spawning, the refined prompt is posted via `post_idempotent_comment()` with phase `"refined-prompt"` (marker `<!-- ralph:task:<id>:refined-prompt -->`). Comment failure logs a warning and does not block dispatch.

5. **Correct `spawn_ralph_auto` argv**: Changed from `["auto", idea]` to `["auto", "--idea", &idea]` to match clap's `#[arg(long)]` declaration. This fixes a latent bug — the current positional invocation would be rejected by clap.

6. **Graceful fallback**: If refinement fails (backend error, timeout, empty/short output), the daemon logs a warning and uses the raw `"{title}\n\n{body}"` string as `--idea`.

7. **Configuration**: Two new fields following the existing flat `daemon_*` naming pattern in `[workspace]`:
   - `daemon_refinement_enabled: bool` (default `true`)
   - `daemon_refinement_backend: String` (default `"claude(sonnet)"`)

   Project-level overrides in `[daemon]`: `refinement_enabled: Option<bool>`, `refinement_backend: Option<String>`. Resolved through `resolve_daemon_config()` with project-override precedence. No new role in `BackendRoleModels`.

8. **Strict dispatch ordering**: `create_worktree` → `resolve raw_idea` → `refine_prompt` (if enabled) → `post refined-prompt comment` (best-effort) → `spawn_ralph_auto` → CAS state update. Task remains `Pending` until successful spawn.

## Technical Approach

### 1. Extend `GhIssue` with body

Add `body: Option<String>` to both `GhIssue` and `RawGhIssue` in `src/daemon/github.rs`. Update `poll_issues()` to request `"number,title,labels,body"`. The body is nullable (GitHub issues can have empty bodies). Propagate in the `RawGhIssue → GhIssue` mapping.

Add `fetch_issue_body()`:
```rust
pub fn fetch_issue_body(owner: &str, repo: &str, issue_number: u32) -> Result<(String, Option<String>)>
```
Calls `gh issue view <number> --repo owner/repo --json title,body`. Used only on restart for legacy tasks where `raw_idea` is `None`.

### 2. Store `raw_idea` on `DaemonTask`

Add `raw_idea: Option<String>` with `#[serde(default)]` to `DaemonTask` in `src/daemon/mod.rs`. After claiming in `poll_and_claim()`, compose `raw_idea = format!("{}\n\n{}", issue.title, issue.body.unwrap_or_default())` and store on the task before persisting.

In `adopt_pending_tasks()`: if `task.raw_idea.is_none()`, call `fetch_issue_body()` to populate it. Never synthesize `"Implement task {task_id}"`.

### 3. Fix `spawn_ralph_auto` argv

In `src/daemon/process.rs`, change line 28 from `cmd.args(["auto", idea])` to `cmd.args(["auto", "--idea", idea])`.

### 4. New module: `src/daemon/refine.rs`

```rust
pub fn refine_prompt(
    raw_idea: &str,
    backend_spec: &str,
    global_config: &GlobalConfig,
) -> Result<String>
```

- Parse `backend_spec` via `parse_backend_spec()` to get backend name + model.
- Construct a `CliBackend` using `claude::backend_from_config()` or `codex::backend_from_config()`.
- Build the refinement prompt by wrapping `raw_idea` with a hardcoded system prompt constant.
- Execute via `tokio::runtime::Handle::current().block_on(backend.execute(&prompt))`. This is safe because the daemon's synchronous loop runs on the main thread within the `#[tokio::main]` runtime — `Handle::current()` obtains the active runtime handle without nesting.
- Validate: if output is empty or < 20 chars, return `Err` to trigger fallback.

### 5. Integration into `dispatch_task`

Modify `dispatch_task()` in `runtime.rs` to accept refinement config. The updated flow:

1. Create worktree (unchanged)
2. Determine raw idea: use `task.raw_idea` if `Some`, else fetch via `fetch_issue_body()` and update task in store
3. If `refinement_enabled`: call `refine_prompt()`. On success, use refined text. On error, log warning and use raw idea.
4. Post idea as comment via `post_idempotent_comment()` with phase `"refined-prompt"` (best-effort, warn-and-continue on failure)
5. Spawn with `["auto", "--idea", &idea]`
6. CAS update (unchanged)

Comment idempotency: the marker prevents duplicate comments on retry. The spawned process always receives the latest idea regardless.

### 6. Configuration plumbing

Add `daemon_refinement_enabled` (bool, default `true`) and `daemon_refinement_backend` (String, default `"claude(sonnet)"`) to `WorkspaceConfig` with `#[serde(default = "...")]`.

Add `refinement_enabled: Option<bool>` and `refinement_backend: Option<String>` to `ProjectDaemonOverrides`.

Add both resolved fields to `EffectiveDaemonConfig`. Wire through `resolve_daemon_config()`.

Add `refinement_enabled: bool`, `refinement_backend: String`, and `global_config: GlobalConfig` to `DaemonRuntimeConfig`. Populate in `cli/daemon.rs`'s `execute_start()`.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/refine.rs` | **New.** `refine_prompt()`, `REFINEMENT_SYSTEM_PROMPT` constant, backend construction helper. |
| `src/daemon/mod.rs` | Add `pub mod refine;`. Add `raw_idea: Option<String>` (with `#[serde(default)]`) to `DaemonTask`. Update test helper `task()` to include `raw_idea: None`. |
| `src/daemon/github.rs` | Add `body: Option<String>` to `GhIssue` and `RawGhIssue`. Update `poll_issues()` JSON fields. Add `fetch_issue_body()`. |
| `src/daemon/runtime.rs` | Add `refinement_enabled`, `refinement_backend`, `global_config` to `DaemonRuntimeConfig`. Update `poll_and_claim()` to store `raw_idea`. Update `adopt_pending_tasks()` to fetch body for legacy tasks. Update `dispatch_task()` with refinement → comment → spawn flow. |
| `src/daemon/process.rs` | Fix `spawn_ralph_auto()` argv: `["auto", "--idea", idea]`. |
| `src/config/global.rs` | Add `daemon_refinement_enabled` and `daemon_refinement_backend` to `WorkspaceConfig` with serde defaults. |
| `src/config/project.rs` | Add `refinement_enabled: Option<bool>` and `refinement_backend: Option<String>` to `ProjectDaemonOverrides`. |
| `src/config/mod.rs` | Add fields to `EffectiveDaemonConfig`. Wire in `resolve_daemon_config()`. |
| `src/cli/daemon.rs` | Populate new `DaemonRuntimeConfig` fields from `EffectiveDaemonConfig` + `GlobalConfig`. |

## Testing Strategy

1. **`refine.rs` unit tests**: Verify prompt construction wraps `raw_idea` correctly. Verify output validation rejects empty and < 20 char responses.

2. **Config deserialization tests**: TOML with/without refinement fields — verify defaults (`true`, `"claude(sonnet)"`). `resolve_daemon_config` with project overrides for refinement fields.

3. **`GhIssue` body tests**: Deserialize `poll_issues` JSON output with `body` present, `body: null`, and `body` absent — all should succeed with correct `Option<String>` values.

4. **`spawn_ralph_auto` argv test**: Assert the spawned command arguments are exactly `["auto", "--idea", <idea>]` via a mock binary that records its argv to a file.

5. **`DaemonTask` backwards compatibility**: Deserialize a `tasks.json` without `raw_idea` — verify default `None`. Round-trip with `raw_idea: Some(...)`.

6. **Conformance/integration tests for dispatch flow**:
   - **Happy path**: Mock `gh` + refinement backend. Verify `ralph auto` receives refined prompt via `--idea`.
   - **Refinement failure → raw fallback**: Mock backend error. Verify warning logged, raw idea used, dispatch completes.
   - **Comment failure → dispatch continues**: Mock `gh issue comment` failure. Verify warning logged, spawn proceeds.
   - **Refinement disabled**: Set `daemon_refinement_enabled = false`. Verify no refinement call; raw idea used directly.
   - **Restart with legacy task**: Create `tasks.json` with pending task lacking `raw_idea`. Mock `gh issue view`. Verify `adopt_pending_tasks` fetches body, never uses `"Implement task {id}"`.
   - **Strict ordering**: Mock `gh` and refinement backend record call order. Assert sequence: `claim → worktree → refine → comment → spawn`.

## Out of Scope

- **Template-based refinement prompt**: The system prompt is a hardcoded constant, not loaded from a file. Customizable templates can be added later if needed.
- **Async daemon runtime**: The daemon remains synchronous. `Handle::current().block_on()` bridges to the async backend. Full async migration is separate work.
- **Streaming or multi-turn refinement**: Single-shot prompt → response only.
- **Refinement caching**: Each dispatch refines from scratch. `raw_idea` on `DaemonTask` preserves the input but refined output is not persisted.
- **New `daemon_refiner` role in `BackendRoleModels`**: Using a direct backend spec string (like `prompt_review_backend`) is simpler.
- **Comment correction on retry**: If a comment was already posted with raw fallback and a retry produces a different refined prompt, the existing comment is not updated. The marker prevents duplicates; the spawned process always uses the latest refinement.