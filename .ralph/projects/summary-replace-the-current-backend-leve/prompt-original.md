Now let me produce the revised specification.

---

## Summary

Add per-agent-role timeout configuration to `BackendConfig`, allowing each role (planner, implementer, reviewer, qa, completer, acceptance_qa, reformatter, prompt_reviewer) to have its own configurable timeout value. The `timeout_seconds` field on `BackendConfig` is retained as the fallback default (2 hours / 7200 seconds). Per-role overrides live under `[backends.<name>.role_timeouts]` in TOML — still scoped per-backend because different backends (claude vs codex) have legitimately different performance characteristics. The key semantic shift is that timeout is resolved per *(backend, role)* pair at execution time rather than baked into the `CliBackend` at construction time.

The timeout override is threaded from `BackendRegistry` through `execute_with_timeout_retries` to a new `execute_with_log_and_timeout` method on the `Backend` trait. This covers all orchestrator call sites (including acceptance QA, reformatter retries, and prompt reviewer). Non-orchestrator paths (daemon refinement, PRD pipelines) continue to use the backend's `timeout_seconds` fallback via the existing `backend.execute()` path, which is explicitly documented as intentional and out-of-scope for role-based overrides.

## Acceptance Criteria

- [ ] `BackendConfig` gains a `role_timeouts: RoleTimeouts` field with optional `u64` values for all 8 executing roles: planner, implementer, reviewer, qa, completer, acceptance_qa, reformatter, prompt_reviewer
- [ ] `BackendConfig.timeout_seconds` is retained as the fallback default; existing configs with only `timeout_seconds` continue to work unchanged
- [ ] `BackendConfig::timeout_for_role(&self, role: &str) -> Duration` resolves per-role timeout, falling back to `timeout_seconds`
- [ ] `PartialBackendConfig` and its `into_backend_config_with_defaults` handle merging `role_timeouts` from user TOML with coded defaults, following the existing `BackendRoleModels::fill_from` pattern
- [ ] `Backend` trait gains `execute_with_log_and_timeout(&self, prompt, log_writer, timeout) -> Result<String>` with a default implementation that delegates to `execute_with_log` (ignoring the timeout parameter); `CliBackend` and `TmuxBackend` override it to use the provided timeout
- [ ] `BackendRegistry::timeout_for_role(&self, backend_spec: &str, role: &str) -> Duration` resolves the effective timeout by parsing the spec to its base backend name and delegating to `BackendConfig::timeout_for_role`
- [ ] `execute_with_timeout_retries` gains a `timeout: Duration` parameter and calls `execute_with_log_and_timeout` instead of `execute_with_log`
- [ ] All 9 `execute_with_parse_retries` call sites in the orchestrator resolve the correct role-specific timeout via `BackendRegistry::timeout_for_role` before passing it down — including prompt_reviewer (role `"prompt_reviewer"`), acceptance QA (role `"qa"` with explicit override to `"acceptance_qa"` for timeout resolution), and reformatter retries (role `"reformatter"` for timeout resolution)
- [ ] `TmuxBackend::execute_with_log_and_timeout` passes the provided timeout to `tmux::wait_for_exit` instead of reading `self.inner.timeout()`
- [ ] TOML config `[backends.claude.role_timeouts]` and `[backends.codex.role_timeouts]` sections are supported and optional
- [ ] Default timeout for all roles is 7200 seconds when no override is specified
- [ ] `BackendTimeout` error variant gains `role: String` and `timeout_secs: u64` fields; all match arms updated
- [ ] `BackendTimeoutExhausted` error variant gains `role: String` and `timeout_secs: u64` fields; all match arms updated
- [ ] `config set` supports `backends.claude.role_timeouts.<role>` and `backends.codex.role_timeouts.<role>` keys; `config get`/`config show` exposes them
- [ ] E2E conformance test `backend_timeout_exhausted_fails_task` updated for enriched error fields
- [ ] Unit tests verify `RoleTimeouts` resolution, TOML deserialization, fallback behavior, and prompt_reviewer/acceptance_qa/reformatter timeout selection

## Technical Approach

### 1. New `RoleTimeouts` struct (`src/config/global.rs`)

Add a struct following the same pattern as `BackendRoleModels`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct RoleTimeouts {
    pub planner: Option<u64>,
    pub implementer: Option<u64>,
    pub reviewer: Option<u64>,
    pub qa: Option<u64>,
    pub completer: Option<u64>,
    pub acceptance_qa: Option<u64>,
    pub reformatter: Option<u64>,
    pub prompt_reviewer: Option<u64>,
}

impl RoleTimeouts {
    pub fn for_role(&self, role: &str) -> Option<u64> {
        match role {
            "planner" => self.planner,
            "implementer" => self.implementer,
            "reviewer" => self.reviewer,
            "qa" => self.qa,
            "completer" => self.completer,
            "acceptance_qa" => self.acceptance_qa,
            "reformatter" => self.reformatter,
            "prompt_reviewer" => self.prompt_reviewer,
            _ => None,
        }
    }

    pub fn fill_from(&mut self, defaults: &RoleTimeouts) {
        macro_rules! fill {
            ($field:ident) => {
                if self.$field.is_none() {
                    self.$field = defaults.$field;
                }
            };
        }
        fill!(planner);
        fill!(implementer);
        fill!(reviewer);
        fill!(qa);
        fill!(completer);
        fill!(acceptance_qa);
        fill!(reformatter);
        fill!(prompt_reviewer);
    }
}
```

This includes all 8 roles that execute backends: the 7 from `BackendRoleModels` plus `prompt_reviewer` which is an executing role in the orchestrator but was absent from the original spec.

Add to `BackendConfig`:

```rust
pub struct BackendConfig {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_seconds: u64,
    pub env: BTreeMap<String, String>,
    pub models: BackendRoleModels,
    #[serde(default)]
    pub role_timeouts: RoleTimeouts,  // NEW
}
```

Add to `PartialBackendConfig`:

```rust
struct PartialBackendConfig {
    // ... existing fields ...
    role_timeouts: Option<RoleTimeouts>,
}
```

Merge in `into_backend_config_with_defaults`:

```rust
if let Some(mut role_timeouts) = self.role_timeouts {
    role_timeouts.fill_from(&defaults.role_timeouts);
    defaults.role_timeouts = role_timeouts;
}
```

### 2. Timeout resolution method on `BackendConfig`

```rust
impl BackendConfig {
    pub fn timeout_for_role(&self, role: &str) -> Duration {
        let secs = self.role_timeouts.for_role(role)
            .unwrap_or(self.timeout_seconds);
        Duration::from_secs(secs)
    }
}
```

### 3. New `Backend` trait method with timeout parameter (`src/backend/mod.rs`)

Add a new method to the `Backend` trait with a default implementation:

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, prompt: &str) -> Result<String>;
    async fn execute_with_log(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
    ) -> Result<String> { /* existing default */ }

    /// Execute with an explicit timeout override. Default delegates to
    /// execute_with_log (ignoring timeout), which preserves behavior for
    /// MockBackend and any future backends that don't need timeout control.
    async fn execute_with_log_and_timeout(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
        _timeout: Duration,
    ) -> Result<String> {
        self.execute_with_log(prompt, log_writer).await
    }

    async fn health_check(&self) -> Result<()>;
}
```

**`CliBackend` override**: Modify `execute_streaming` to accept a `timeout: Duration` parameter instead of reading `self.timeout`. The existing `execute` and `execute_with_log` methods pass `self.timeout` to `execute_streaming` (preserving backward compat for non-orchestrator callers like daemon refinement and PRD pipelines). The new `execute_with_log_and_timeout` passes the caller-supplied timeout:

```rust
impl CliBackend {
    async fn execute_streaming(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
        timeout: Duration,  // was self.timeout
    ) -> Result<String> {
        // ... existing logic, but use `timeout` parameter in
        // tokio::time::timeout(timeout, ...) instead of self.timeout
    }
}

#[async_trait]
impl Backend for CliBackend {
    async fn execute(&self, prompt: &str) -> Result<String> {
        self.execute_streaming(prompt, None, self.timeout).await
    }

    async fn execute_with_log(&self, prompt: &str, log_writer: Option<&mut LogWriter>) -> Result<String> {
        self.execute_streaming(prompt, log_writer, self.timeout).await
    }

    async fn execute_with_log_and_timeout(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
        timeout: Duration,
    ) -> Result<String> {
        self.execute_streaming(prompt, log_writer, timeout).await
    }
}
```

**`TmuxBackend` override**: The `execute_with_log_and_timeout` implementation passes the timeout to `tmux::wait_for_exit` instead of `self.inner.timeout()`. The existing `execute`/`execute_with_log` methods continue to use `self.inner.timeout()` for backward compat:

```rust
#[async_trait]
impl Backend for TmuxBackend {
    // existing execute/execute_with_log unchanged (use self.inner.timeout())

    async fn execute_with_log_and_timeout(
        &self,
        prompt: &str,
        log_writer: Option<&mut LogWriter>,
        timeout: Duration,
    ) -> Result<String> {
        // Same logic as current execute(), but:
        //   tmux::wait_for_exit(&exit_file, timeout, POLL_INTERVAL)
        // instead of:
        //   tmux::wait_for_exit(&exit_file, self.inner.timeout(), POLL_INTERVAL)
    }
}
```

This approach avoids the structural problem identified in review issue #6: there is exactly one design (parameterized `execute_streaming` + new trait method), not multiple conflicting options.

### 4. `BackendRegistry::timeout_for_role` (`src/backend/mod.rs`)

```rust
impl BackendRegistry {
    pub fn timeout_for_role(&self, backend_spec: &str, role: &str) -> Duration {
        let base_name = parse_backend_spec(backend_spec)
            .map(|p| p.name)
            .unwrap_or_else(|_| backend_spec.to_owned());
        self.config.backend_config(&base_name)
            .map(|bc| bc.timeout_for_role(role))
            .unwrap_or_else(|| Duration::from_secs(7200))
    }
}
```

### 5. Thread timeout through the orchestrator (`src/workflow/orchestrator.rs`)

**`execute_with_timeout_retries`** gains a `timeout: Duration` parameter and calls `execute_with_log_and_timeout`:

```rust
async fn execute_with_timeout_retries(
    backend: Arc<dyn Backend>,
    role: &str,
    phase: &str,
    prompt: &str,
    log_writer: &mut LogWriter,
    timeout: Duration,  // NEW
) -> Result<String> {
    for attempt in 1..=3_u8 {
        // ...
        match backend.execute_with_log_and_timeout(prompt, Some(log_writer), timeout).await {
            // ... existing retry logic with updated error constructors
        }
    }
}
```

**`execute_with_parse_retries`** gains a `timeout: Duration` parameter and passes it through:

```rust
async fn execute_with_parse_retries<T, F>(
    backend: Arc<dyn Backend>,
    registry: &BackendRegistry,
    role: &str,
    phase: &str,
    original_prompt: &str,
    parse_fn: F,
    expected_format: &str,
    log_writer: &mut LogWriter,
    timeout: Duration,  // NEW
) -> Result<T>
```

All three calls to `execute_with_timeout_retries` inside `execute_with_parse_retries` use this timeout, *except* the reformatter retry path which resolves its own timeout:

```rust
// Reformatter retry: resolve timeout for the reformatter role
let reformatter_timeout = registry.timeout_for_role(&reformatter_spec, "reformatter");
let second_output = execute_with_timeout_retries(
    reformatter_backend, role, phase, &reformat_prompt,
    log_writer, reformatter_timeout,
).await?;
```

**All 9 orchestrator call sites** resolve timeout before calling `execute_with_parse_retries`:

| Call site | Backend spec source | Role for timeout resolution |
|---|---|---|
| Prompt reviewer (line ~260) | `pr_backend_spec` | `"prompt_reviewer"` |
| Planner (line ~390) | `planner_backend_name` | `"planner"` |
| Implementer initial (line ~539) | `impl_backend_name` | `"implementer"` |
| Implementer QA response (line ~645) | `impl_backend_name` | `"implementer"` |
| Implementer review response (line ~754) | `impl_backend_name` | `"implementer"` |
| QA (line ~918) | `qa_backend_name` | `"qa"` |
| Reviewer (line ~1115) | `reviewer_backend_name` | `"reviewer"` |
| Completer (line ~1355) | `completer_backend_name` | `"completer"` |
| Acceptance QA (line ~1450) | `acceptance_qa_backend_name` | `"acceptance_qa"` |

The critical fix for review issue #3: acceptance QA currently passes `role: "qa"` to `execute_with_parse_retries`, which would resolve the `qa` timeout, not `acceptance_qa`. The fix: pass `"acceptance_qa"` as the timeout-resolution role. Since the `role` parameter is also used for parse error messages and log context, we add a separate `timeout_role` parameter to `execute_with_parse_retries` (or resolve the timeout at the call site and pass it as `Duration`). The cleaner approach is to resolve at the call site:

```rust
// Acceptance QA call site
let acceptance_timeout = registry.timeout_for_role(acceptance_qa_backend_name, "acceptance_qa");
let acceptance_decision = execute_with_parse_retries(
    acceptance_qa_backend,
    &registry,
    "qa",           // role for logging/parse context (unchanged)
    "completing",   // phase
    &acceptance_prompt,
    parse_qa_output,
    &expected_format_template_for("qa", None),
    &mut acceptance_log,
    acceptance_timeout,  // timeout resolved for acceptance_qa role
).await?;
```

### 6. Enrich error types (`src/error.rs`)

```rust
#[error("backend timeout: {backend} (role={role}, timeout={timeout_secs}s)")]
BackendTimeout {
    backend: String,
    role: String,
    timeout_secs: u64,
},

#[error("BackendTimeoutExhausted: backend timeout retries exhausted for {backend} during {phase} (role={role}, timeout={timeout_secs}s) after {attempts} attempts")]
BackendTimeoutExhausted {
    backend: String,
    phase: String,
    role: String,
    timeout_secs: u64,
    attempts: u8,
},
```

Update all producers and consumers:

- **`CliBackend::execute_streaming`**: The timeout `Duration` is already available as the parameter; convert to `u64` via `.as_secs()`. The `role` is not available at this level — pass `role: String` as an additional parameter to `execute_streaming`, or produce the error with an empty role and let the orchestrator enrich it. The cleaner approach: add `role: &str` to `execute_with_log_and_timeout` trait method signature so the backend can include it in the error. However, adding role to the trait method is invasive. Simpler: produce `BackendTimeout` with just `backend`, and have `execute_with_timeout_retries` wrap it with the role/timeout info before converting to `BackendTimeoutExhausted`:

    ```rust
    // In execute_with_timeout_retries:
    Err(RalphError::BackendTimeout { backend: backend_name }) => {
        if attempt == 3 {
            return Err(RalphError::BackendTimeoutExhausted {
                backend: backend_name,
                phase: phase.to_owned(),
                role: role.to_owned(),
                timeout_secs: timeout.as_secs(),
                attempts: attempt,
            });
        }
        // ... retry
    }
    ```

    Leave `BackendTimeout` with just `backend: String` (no enrichment at the backend layer). The enriched `role` and `timeout_secs` fields go only on `BackendTimeoutExhausted`, which is the variant that surfaces to users. This is simpler and avoids threading role info through the backend layer.

**Revised error approach:**

```rust
// BackendTimeout stays simple (internal retry signal)
#[error("backend timeout: {backend}")]
BackendTimeout { backend: String },

// BackendTimeoutExhausted gains role + timeout (user-facing)
#[error("BackendTimeoutExhausted: backend timeout retries exhausted for {backend} during {phase} (role={role}, timeout={timeout_secs}s) after {attempts} attempts")]
BackendTimeoutExhausted {
    backend: String,
    phase: String,
    role: String,
    timeout_secs: u64,
    attempts: u8,
},
```

Update match arms:
- `orchestrator.rs` `execute_with_timeout_retries`: add `role` and `timeout_secs` when constructing `BackendTimeoutExhausted`
- `orchestrator.rs` `is_terminal_orchestration_error`: update destructure to include new fields (use `..` rest pattern)
- `cli/run.rs` `mark_project_failed`: update destructure
- `tmux_backend.rs`: no change (still produces `BackendTimeout { backend }`)

### 7. Config CLI surface (`src/cli/config.rs`)

Add `config set` support for role timeouts, following the same wildcard-prefix pattern used for `backends.*.models.*`:

```rust
_ if key.starts_with("backends.claude.role_timeouts.") => {
    let role = key.trim_start_matches("backends.claude.role_timeouts.");
    set_backend_role_timeout(&mut config.backends.claude.role_timeouts, role, raw_value)?;
}
_ if key.starts_with("backends.codex.role_timeouts.") => {
    let role = key.trim_start_matches("backends.codex.role_timeouts.");
    set_backend_role_timeout(&mut config.backends.codex.role_timeouts, role, raw_value)?;
}
```

New helper:

```rust
fn set_backend_role_timeout(
    timeouts: &mut RoleTimeouts,
    role: &str,
    raw_value: &str,
) -> Result<()> {
    let value = if raw_value == "null" {
        None
    } else {
        Some(parse_u64(raw_value, &format!("role_timeouts.{role}"))?)
    };
    match role {
        "planner" => timeouts.planner = value,
        "implementer" => timeouts.implementer = value,
        "reviewer" => timeouts.reviewer = value,
        "qa" => timeouts.qa = value,
        "completer" => timeouts.completer = value,
        "acceptance_qa" => timeouts.acceptance_qa = value,
        "reformatter" => timeouts.reformatter = value,
        "prompt_reviewer" => timeouts.prompt_reviewer = value,
        _ => return Err(RalphError::Validation(
            format!("unknown role timeout: {role}")
        )),
    }
    Ok(())
}
```

`config get` and `config show` work automatically via JSON serialization — `RoleTimeouts` derives `Serialize` so it appears in the JSON output. No additional code needed.

Legacy key behavior: `backends.*.timeout_seconds` continues to work as the fallback default. It is not deprecated or rejected. Setting `timeout_seconds` and `role_timeouts` simultaneously is valid — role overrides take precedence, `timeout_seconds` is the fallback.

### 8. Non-orchestrator execution paths

**Daemon refinement** (`src/daemon/refine.rs`): Calls `backend.execute()`, which uses `self.timeout` (i.e., `BackendConfig.timeout_seconds`). Since refinement is not an agent role in the orchestrator, it correctly uses the fallback timeout. No change needed.

**PRD / Quick-PRD pipelines** (`src/prd/pipeline.rs`, `src/prd/quick.rs`): Call `backend.execute()` directly. Same as refinement — use fallback timeout. No change needed.

These paths are explicitly out of scope for role-based timeouts because they don't correspond to orchestrator agent roles. If future role-based timeout control is desired for these, it can be added as a separate feature.

## Files & Modules

| File | Change |
|---|---|
| `src/config/global.rs` | Add `RoleTimeouts` struct (8 fields + `for_role` + `fill_from`); add `role_timeouts: RoleTimeouts` to `BackendConfig`; add `timeout_for_role` method to `BackendConfig`; add `role_timeouts: Option<RoleTimeouts>` to `PartialBackendConfig` with merge in `into_backend_config_with_defaults`; update `Default` impls for `BackendConfig` to include `role_timeouts: RoleTimeouts::default()` |
| `src/backend/mod.rs` | Add `execute_with_log_and_timeout` to `Backend` trait with default impl; parameterize `CliBackend::execute_streaming` to accept `timeout: Duration`; update `CliBackend`'s `execute` and `execute_with_log` to pass `self.timeout` to `execute_streaming`; add `CliBackend::execute_with_log_and_timeout` override; add `BackendRegistry::timeout_for_role` method |
| `src/backend/tmux_backend.rs` | Add `execute_with_log_and_timeout` override that passes caller-supplied timeout to `tmux::wait_for_exit`; refactor shared logic between `execute` and `execute_with_log_and_timeout` into a private helper to avoid duplication |
| `src/backend/claude.rs` | No change (timeout resolved at execution time, not construction) |
| `src/backend/codex.rs` | No change (same reason) |
| `src/workflow/orchestrator.rs` | Add `timeout: Duration` param to `execute_with_timeout_retries` and `execute_with_parse_retries`; update all 9 call sites to resolve timeout via `registry.timeout_for_role(backend_spec, role)`; acceptance QA call site uses `"acceptance_qa"` for timeout resolution; reformatter retry path resolves its own timeout; update `BackendTimeoutExhausted` construction to include `role` and `timeout_secs`; update `is_terminal_orchestration_error` destructure |
| `src/error.rs` | Add `role: String` and `timeout_secs: u64` to `BackendTimeoutExhausted` variant; update error message format string |
| `src/cli/run.rs` | Update `BackendTimeoutExhausted` match destructuring (use `..` rest pattern) |
| `src/cli/config.rs` | Add `backends.claude.role_timeouts.*` and `backends.codex.role_timeouts.*` wildcard handlers in `set_global_value`; add `set_backend_role_timeout` helper function |
| `src/daemon/refine.rs` | No change (uses `backend.execute()` which falls back to `timeout_seconds`) |
| `src/prd/pipeline.rs` | No change |
| `src/prd/quick.rs` | No change |
| `.ralph/ralph.toml` | No mandatory changes; `role_timeouts` defaults to all-`None` |
| `src/validate/tests_e2e_conformance.rs` | Update `backend_timeout_exhausted_fails_task` to match enriched error message; add new test for role-specific timeout override |
| `src/backend/mock.rs` | No change (default trait impl handles `execute_with_log_and_timeout`) |

## Testing Strategy

### Unit tests (`src/config/global.rs`)

- **`RoleTimeouts::for_role` returns correct value** for each of the 8 roles and `None` for unknown role strings
- **`RoleTimeouts::fill_from` merges partial overrides** with defaults (e.g., user sets `planner = 3600`, defaults has `qa = 1800` → merged result has both)
- **`BackendConfig::timeout_for_role` falls back to `timeout_seconds`** when role override is `None`
- **`BackendConfig::timeout_for_role` uses role override** when set (e.g., `role_timeouts.planner = 3600` with `timeout_seconds = 7200` → planner gets 3600)
- **TOML deserialization with `[backends.claude.role_timeouts]`** section parses correctly
- **TOML deserialization without `role_timeouts`** section defaults to all-`None` (backward compat)
- **Partial `role_timeouts`** (only some roles set) deserializes correctly, others remain `None`
- **`PartialBackendConfig::into_backend_config_with_defaults`** correctly merges role_timeouts

### Unit tests (`src/backend/mod.rs`)

- **`BackendRegistry::timeout_for_role`** resolves from config for known backends
- **`BackendRegistry::timeout_for_role`** returns 7200s default for unknown backends
- **`BackendRegistry::timeout_for_role`** parses backend specs like `claude(opus)` to resolve against base `claude` config
- **`CliBackend` timeout override test** (`cli_backend_timeout_kills_and_reaps_child_and_writes_footer` adapted): verify that `execute_with_log_and_timeout` uses the provided `Duration`, not `self.timeout`
- **`CliBackend` backward compat**: verify `execute` and `execute_with_log` still use `self.timeout`

### Unit tests (`src/backend/tmux_backend.rs`)

- **`execute_with_log_and_timeout` uses provided timeout**: mock tmux execution with a short caller-supplied timeout; verify `BackendTimeout` is raised at the expected time
- **`execute` still uses `self.inner.timeout()`**: verify backward compat for non-orchestrator callers

### Integration / E2E tests (`src/validate/tests_e2e_conformance.rs`)

- **Backward compat**: Existing `backend_timeout_exhausted_fails_task` passes with only `timeout_seconds` set (no `role_timeouts`); update stderr assertion to match enriched error format
- **Role-specific timeout override**: Set `backends.claude.role_timeouts.planner = 2` via `config set`, leave `timeout_seconds = 7200`; verify planner times out (stderr contains `BackendTimeoutExhausted` with `role=planner`) while a default-timeout role would not
- **Acceptance QA timeout resolution**: Set `backends.claude.role_timeouts.acceptance_qa = 2` (short) and `backends.claude.role_timeouts.qa = 7200` (long); verify acceptance QA uses the short timeout, not the QA timeout
- **Config CLI round-trip**: `config set backends.claude.role_timeouts.planner 3600` then `config get backends.claude.role_timeouts.planner` returns `3600`; `config set backends.claude.role_timeouts.planner null` clears it

### Error message tests

- **`BackendTimeoutExhausted` error string** includes role name and timeout value in expected format

## Out of Scope

- **Per-project role timeout overrides**: `ProjectConfig` / `ProjectDaemonOverrides` does not gain `role_timeouts` in this change. Project-level overrides can follow the existing override pattern in a follow-up.
- **CLI flags for role timeouts**: No `--planner-timeout` or similar CLI arguments. Configuration is TOML-only (plus `config set`).
- **Removing `timeout_seconds` entirely**: Retained as the universal fallback. Deprecation/removal is a separate decision.
- **Non-orchestrator role-based timeouts**: Daemon refinement, PRD pipelines, and quick-PRD pipelines continue using `BackendConfig.timeout_seconds` via `backend.execute()`. These paths do not have agent roles and intentionally use the fallback.
- **Dynamic timeout adjustment**: No runtime tuning based on prompt size or historical execution time.
- **`daemon_rebase_timeout_seconds`**: This is a workspace-level config for a different subsystem (git rebase operations, not backend agent execution) and remains unchanged.
- **Validation bounds on timeout values**: No minimum/maximum enforcement beyond `u64` range.
- **Enriching `BackendTimeout` (internal variant)**: Only `BackendTimeoutExhausted` (user-facing) gets `role` and `timeout_secs` enrichment. `BackendTimeout` remains a simple internal retry signal with just `backend: String`.