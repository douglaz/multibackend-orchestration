I now have a thorough understanding of the codebase. Let me write the spec.

## Summary

Add a prompt refinement step to the daemon's issue lifecycle. When the daemon claims a GitHub issue, it fetches the issue body, sends the title+body through an LLM backend to produce a well-structured `ralph auto --idea` prompt, posts the refined prompt as a comment on the issue, and uses it (instead of the raw issue text) as the `--idea` argument to the spawned `ralph auto` subprocess. Refinement is configurable and degrades gracefully — if it fails, the daemon falls back to the raw issue text.

## Acceptance Criteria

1. **Issue body fetched**: `poll_issues()` (or a new `fetch_issue_body()` helper) retrieves the issue body in addition to title/labels. The `GhIssue` struct gains a `body: Option<String>` field.
2. **Refinement produces a structured prompt**: A new function sends `"{title}\n\n{body}"` to a configured backend with a system prompt instructing it to rewrite the text as a well-structured `ralph auto --idea` prompt with clarified requirements and acceptance criteria. The raw output string from the backend is the refined prompt.
3. **Refined prompt posted as comment**: Before spawning `ralph auto`, the refined prompt is posted to the issue via `post_idempotent_comment()` with marker `<!-- ralph:task:<task_id>:refined-prompt -->`.
4. **Refined prompt used as `--idea`**: `spawn_ralph_auto()` receives the refined prompt string (or raw fallback) instead of the current `format!("Implement task {}", task.task_id)` placeholder.
5. **Graceful fallback**: If refinement fails (backend error, timeout, empty output), the daemon logs a warning and falls back to using `"{title}\n\n{body}"` as the idea.
6. **Configuration**: Two new fields on `WorkspaceConfig` (and mirrored as optionals on `ProjectDaemonOverrides` / `EffectiveDaemonConfig`):
   - `daemon_refinement_enabled: bool` (default `true`)
   - `daemon_refinement_backend: String` (default `"claude(sonnet)"`)
7. **Timing**: Refinement + comment posting occurs after `claim_issue()` and worktree creation, but before `spawn_ralph_auto()`. The task is in `Pending` state during refinement (it transitions to `InProgress` only when the child is spawned, which is the existing behavior).

## Technical Approach

### 1. Extend `GhIssue` with body

Add `body: Option<String>` to `GhIssue` and `body: String` to `RawGhIssue`. Update `poll_issues()` to request `"number,title,labels,body"` from `gh issue list --json`. This avoids a second API call per issue. The body is fetched for all polled issues but only used when an issue is actually claimed — acceptable since `poll_issues` is bounded to 100 issues and the body is a lightweight string field in `gh`'s JSON output.

### 2. Propagate issue title+body to dispatch

Currently `poll_and_claim()` creates a `DaemonTask` but discards the `GhIssue` title/body. The flow needs to carry the raw issue text from `poll_and_claim` → `dispatch_task`. Options:

- **Pass the idea string directly**: After claiming, compose `raw_idea = format!("{}\n\n{}", issue.title, issue.body.unwrap_or_default())` and pass it alongside the task to `dispatch_task`. This avoids storing the body in `DaemonTask` (which is persisted to JSON and should stay lean).
- Store a `raw_idea: Option<String>` on `DaemonTask` for reconciliation on restart. On restart, if `raw_idea` is `None` (legacy tasks), fall back to `"Implement task {task_id}"`.

**Chosen approach**: Add `raw_idea: Option<String>` to `DaemonTask`. This handles the restart/reconciliation case where the daemon restarts and re-adopts pending tasks — it needs the raw idea to refine. The field is `Option` for backwards compatibility with existing `tasks.json` files.

### 3. New module: `src/daemon/refine.rs`

```rust
/// Refine raw issue text into a structured ralph auto prompt.
///
/// Uses a synchronous backend call (blocking on tokio runtime) since the
/// daemon runtime is synchronous.
pub fn refine_prompt(
    raw_idea: &str,
    backend_spec: &str,
    global_config: &GlobalConfig,
) -> Result<String>
```

Implementation:
- Build a `CliBackend` from `backend_spec` using the existing `claude::backend_from_config()` or `codex::backend_from_config()` factory, injecting the model from the spec.
- Construct a refinement prompt wrapping `raw_idea` with instructions to produce a well-structured task description with clarified requirements, acceptance criteria, and scope.
- Execute via `tokio::runtime::Runtime::new().block_on(backend.execute(&prompt))` (the daemon is synchronous; this one-shot runtime is fine for a single blocking call).
- Validate output: if empty or < 20 chars, return `Err` to trigger fallback.

The refinement system prompt (hardcoded, not a template file — this is a daemon-internal concern, not a user-facing workflow template):

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

### 4. Integration into `dispatch_task`

Modify `dispatch_task` in `runtime.rs`:

```
1. Create worktree (existing)
2. NEW: If refinement enabled:
   a. Call refine_prompt(raw_idea, refinement_backend, config)
   b. On success: idea = refined_prompt
   c. On error: log warning, idea = raw_idea
3. NEW: Post refined/raw prompt as comment (marker: refined-prompt)
4. Spawn child with idea (existing, but now using refined text)
5. CAS update (existing)
```

`DaemonRuntimeConfig` gains two new fields:
- `refinement_enabled: bool`
- `refinement_backend: String`

These are populated from `EffectiveDaemonConfig` in `cli/daemon.rs`.

### 5. Configuration plumbing

Add to `WorkspaceConfig`:
```rust
#[serde(default = "default_daemon_refinement_enabled")]
pub daemon_refinement_enabled: bool,
#[serde(default = "default_daemon_refinement_backend")]
pub daemon_refinement_backend: String,
```

Add to `ProjectDaemonOverrides`:
```rust
pub refinement_enabled: Option<bool>,
pub refinement_backend: Option<String>,
```

Add to `EffectiveDaemonConfig`:
```rust
pub refinement_enabled: bool,
pub refinement_backend: String,
```

Wire through `resolve_daemon_config()`.

### 6. `BackendRoleModels` — new `daemon_refiner` role

Add `daemon_refiner: Option<String>` to `BackendRoleModels`. This allows per-backend model overrides for the refinement role (e.g., `[backends.claude.models] daemon_refiner = "sonnet"`). Wire into `for_role()` and `fill_from()`. Default for claude: `"sonnet"`. Default for codex: `"gpt-5.3-codex-medium"`.

However — the spec says the backend is configured as a full backend spec string (`"claude(sonnet)"`), not as a role on a backend. This means `resolve_backend_for_role` is the right mechanism: the configured `daemon_refinement_backend` is a spec like `"claude"`, and the `daemon_refiner` role model override injects the model. This matches how `prompt_review_backend` works.

**Decision**: Use the full backend spec approach (like `prompt_review_backend`), where the user sets `daemon_refinement_backend = "claude(sonnet)"` directly. This is simpler and doesn't require a new role in `BackendRoleModels`. The `daemon_refiner` role concept from the feature idea is unnecessary complexity — a direct backend spec is sufficient and consistent with `prompt_review_backend`.

## Files & Modules

| File | Change |
|------|--------|
| `src/daemon/refine.rs` | **New**. `refine_prompt()` function, refinement system prompt constant, `build_refinement_backend()` helper. |
| `src/daemon/mod.rs` | Add `pub mod refine;`. Add `raw_idea: Option<String>` to `DaemonTask`. |
| `src/daemon/github.rs` | Add `body: Option<String>` to `GhIssue`, `body: String` to `RawGhIssue`. Update `poll_issues()` JSON fields to include `body`. |
| `src/daemon/runtime.rs` | Add `refinement_enabled` and `refinement_backend` to `DaemonRuntimeConfig`. Modify `poll_and_claim()` to store `raw_idea` on `DaemonTask`. Modify `dispatch_task()` to call `refine::refine_prompt()` and post the refined prompt as a comment before spawning. |
| `src/config/global.rs` | Add `daemon_refinement_enabled` and `daemon_refinement_backend` to `WorkspaceConfig` with serde defaults. Add defaults to `Default for GlobalConfig`. |
| `src/config/project.rs` | Add `refinement_enabled: Option<bool>` and `refinement_backend: Option<String>` to `ProjectDaemonOverrides`. |
| `src/config/mod.rs` | Add `refinement_enabled` and `refinement_backend` to `EffectiveDaemonConfig`. Wire in `resolve_daemon_config()`. |
| `src/cli/daemon.rs` | Populate `refinement_enabled` and `refinement_backend` on `DaemonRuntimeConfig` from `EffectiveDaemonConfig`. |

## Testing Strategy

1. **Unit tests for `refine.rs`**:
   - Test refinement prompt construction (verify the system prompt wraps raw_idea correctly).
   - Test output validation (empty output → error, short output → error, valid output → Ok).

2. **Unit tests for config**:
   - Deserialize `ralph.toml` with `daemon_refinement_enabled = false` and `daemon_refinement_backend = "codex"` — verify fields populate.
   - Deserialize without the fields — verify defaults (`true`, `"claude(sonnet)"`).
   - `resolve_daemon_config` with project overrides for refinement fields.

3. **Unit tests for `GhIssue` body**:
   - Verify `poll_issues` JSON deserialization handles `body` field (and tolerates `null` body).

4. **Integration/conformance tests for dispatch flow** (extend existing daemon conformance tests):
   - Mock the `gh` CLI and refinement backend.
   - Verify that when refinement is enabled, the `ralph auto` subprocess receives the refined prompt as its idea argument.
   - Verify that when refinement fails, the raw idea is used and a warning is logged.
   - Verify the refined-prompt comment is posted with the correct marker.
   - Verify that when `daemon_refinement_enabled = false`, no refinement call is made and the raw idea is used directly.

5. **`DaemonTask` backwards compatibility**:
   - Deserialize a `tasks.json` without `raw_idea` field — verify it defaults to `None`.

## Out of Scope

- **Template-based refinement prompt**: The refinement system prompt is hardcoded, not loaded from a template file. It's a daemon-internal concern. If users need customization, a template can be added later.
- **Async daemon runtime**: The daemon runtime is synchronous. The refinement backend call uses a one-shot tokio runtime for the async `Backend::execute()`. Migrating the daemon to fully async is a separate effort.
- **Streaming or multi-turn refinement**: Single-shot prompt → response. No iterative refinement or human-in-the-loop.
- **Refinement caching**: Each dispatch refines from scratch. No caching of refined prompts across restarts (the `raw_idea` field on `DaemonTask` allows re-refinement on restart).
- **New `daemon_refiner` role in `BackendRoleModels`**: Using a direct backend spec string (like `prompt_review_backend`) instead. Simpler, fewer touchpoints.
- **Fetching issue body separately**: The body is fetched inline with `poll_issues` rather than via a separate `gh issue view` call. If body size becomes a concern, this can be revisited.