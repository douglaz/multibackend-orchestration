---
artifact: prompt-review
project: summary-replace-the-current-backend-leve
backend: codex(gpt-5.3-codex-xhigh)
role: prompt_reviewer
created_at: 2026-02-16T19:46:30Z
---

# Prompt Review

## Issues Found
- The spec is internally contradictory about errors: acceptance criteria require adding `role` and `timeout_secs` to both `BackendTimeout` and `BackendTimeoutExhausted`, but the technical approach later says to enrich only `BackendTimeoutExhausted`. This creates incompatible implementations.
- Acceptance QA timeout semantics are ambiguous: the call site uses role `"qa"` for parser/log context, but timeout resolution must use `"acceptance_qa"`. Without an explicit rule, acceptance QA may incorrectly use the normal QA timeout.
- Timeout propagation is not fully specified end-to-end: role-specific timeout resolution is described, but the execution API contract and retry flow do not consistently define where timeout is chosen and where it is consumed.
- Requirements rely on fragile line numbers and a fixed “9 call sites” count. That is brittle under refactors and causes false failures.
- Test requirements are broad but not operationally precise: they do not clearly require coverage for `config set ...role_timeouts.<role>`, `null` clearing, unknown-role validation, and conformance test registration strategy.
- Fallback behavior for unknown backend specs is underspecified (hardcoded `7200` vs centralized default source), which can lead to drift.

## Refined Prompt
### Feature
Implement per-role backend timeout overrides for orchestrator executions.

### Objective
Add timeout configuration per `(backend, role)` while preserving existing `timeout_seconds` behavior as the fallback default.

### Scope
1. Add role timeout config fields to global backend config.
2. Resolve timeout at execution time per `(backend_spec, timeout_role)`.
3. Thread resolved timeout through orchestrator retry/execution path.
4. Add backend API support for caller-supplied timeout.
5. Expose role timeout keys via `config set/get/show`.
6. Add unit + validate conformance coverage.

### Explicit Non-Goals
1. No project-level role timeout overrides in this change.
2. No new CLI flags like `--planner-timeout`.
3. No role-based timeout behavior changes for daemon refinement / PRD / quick-PRD paths.
4. Do not remove `timeout_seconds`.

### Role Set
Supported timeout roles are exactly:
`planner`, `implementer`, `reviewer`, `qa`, `completer`, `acceptance_qa`, `reformatter`, `prompt_reviewer`.

### Required Behavior

#### 1) Config schema (`src/config/global.rs`)
1. Add:
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
```
2. Implement:
- `RoleTimeouts::for_role(&self, role: &str) -> Option<u64>`
- `RoleTimeouts::fill_from(&mut self, defaults: &RoleTimeouts)`
3. Add `#[serde(default)] pub role_timeouts: RoleTimeouts` to `BackendConfig`.
4. Add `role_timeouts: Option<RoleTimeouts>` to `PartialBackendConfig`.
5. Merge `role_timeouts` in `into_backend_config_with_defaults` using `fill_from`, mirroring `models` merge behavior.
6. Add `BackendConfig::timeout_for_role(&self, role: &str) -> Duration`:
- return role override when present
- otherwise return `timeout_seconds`
7. Preserve backwards compatibility: configs with only `timeout_seconds` must deserialize and behave unchanged.
8. Default for all roles remains 7200s via fallback behavior.

#### 2) Backend registry resolution (`src/backend/mod.rs`)
1. Add:
```rust
pub fn timeout_for_role(&self, backend_spec: &str, role: &str) -> Duration
```
2. Behavior:
- Parse `backend_spec` with `parse_backend_spec`.
- Resolve against base backend name (`claude`, `codex`).
- Delegate to `BackendConfig::timeout_for_role`.
- If parsing/config lookup fails, fallback to 7200 seconds.

#### 3) Backend trait API (`src/backend/mod.rs`)
1. Add trait method:
```rust
async fn execute_with_log_and_timeout(
    &self,
    prompt: &str,
    log_writer: Option<&mut LogWriter>,
    timeout: Duration,
) -> Result<String>
```
2. Default implementation delegates to `execute_with_log` (ignores `timeout`) for compatibility with `MockBackend` and future backends.

#### 4) CLI backend timeout override (`src/backend/mod.rs`)
1. Refactor `CliBackend::execute_streaming` to accept `timeout: Duration`.
2. Keep existing behavior for non-orchestrator callers:
- `execute` and `execute_with_log` pass `self.timeout`.
3. Implement `execute_with_log_and_timeout` to pass caller-provided timeout.
4. Timeout errors remain `RalphError::BackendTimeout { backend }`.

#### 5) Tmux backend timeout override (`src/backend/tmux_backend.rs`)
1. Implement `execute_with_log_and_timeout` with caller-provided timeout.
2. Use that timeout in `tmux::wait_for_exit`.
3. Keep `execute` / `execute_with_log` behavior unchanged (still use backend default timeout for non-orchestrator paths).
4. Prefer shared internal helper to avoid logic duplication.

#### 6) Orchestrator timeout plumbing (`src/workflow/orchestrator.rs`)
1. Update `execute_with_timeout_retries` signature to accept `timeout: Duration`.
2. Inside it, call `backend.execute_with_log_and_timeout(...)`.
3. Update `execute_with_parse_retries` to accept `timeout: Duration` and pass through.
4. At each orchestrator call site, resolve timeout before calling `execute_with_parse_retries`:
- prompt reviewer -> timeout role `prompt_reviewer`
- planner -> `planner`
- implementer (all implementer invocations) -> `implementer`
- reviewer -> `reviewer`
- qa -> `qa`
- completer -> `completer`
- acceptance QA -> timeout role `acceptance_qa`
5. Acceptance QA nuance:
- keep parse/log role as `"qa"` if needed for output format consistency
- resolve timeout with `"acceptance_qa"` explicitly
6. Reformatter retry path must resolve timeout with timeout role `reformatter`.

#### 7) Error model (`src/error.rs`, call sites)
Use one consistent model:
1. Keep `BackendTimeout` unchanged:
```rust
BackendTimeout { backend: String }
```
2. Enrich only `BackendTimeoutExhausted`:
```rust
BackendTimeoutExhausted {
    backend: String,
    phase: String,
    role: String,
    timeout_secs: u64,
    attempts: u8,
}
```
3. Update all constructors and match arms accordingly (`orchestrator.rs`, `cli/run.rs`, terminal-error checks).

#### 8) Config CLI surface (`src/cli/config.rs`)
1. Add `config set` support for:
- `backends.claude.role_timeouts.<role>`
- `backends.codex.role_timeouts.<role>`
2. Add helper similar to `set_backend_model`:
- accepts `u64` or `null` (to clear)
- validates role against the 8-role set
- returns validation error for unknown role
3. `config get` / `config show` must expose these values through existing serialization output.

#### 9) Non-orchestrator paths
No behavior change for:
- `src/daemon/refine.rs`
- `src/prd/pipeline.rs`
- `src/prd/quick.rs`
They continue using `backend.execute()` and therefore `timeout_seconds` fallback.

### Acceptance Criteria
- [ ] `RoleTimeouts` exists with all 8 roles and serde defaults.
- [ ] `BackendConfig.role_timeouts` exists and defaults to all `None`.
- [ ] `BackendConfig::timeout_for_role` returns override or `timeout_seconds`.
- [ ] Partial config merge supports `role_timeouts` like `models`.
- [ ] `BackendRegistry::timeout_for_role` resolves by base backend name from backend spec.
- [ ] `Backend` trait has `execute_with_log_and_timeout` with compatible default implementation.
- [ ] `CliBackend` and `TmuxBackend` honor caller-supplied timeout through new method.
- [ ] Orchestrator retry path uses supplied timeout, not backend-construction timeout.
- [ ] Acceptance QA resolves timeout via `acceptance_qa` role.
- [ ] Reformatter retry resolves timeout via `reformatter` role.
- [ ] `BackendTimeoutExhausted` includes `role` and `timeout_secs`.
- [ ] `config set/get/show` support role timeout keys and `null` clearing.
- [ ] Existing configs using only `timeout_seconds` remain valid and unchanged in behavior.
- [ ] Default effective timeout remains 7200s when no role override is configured.

### Required Tests

#### Unit tests
1. `src/config/global.rs`
- `RoleTimeouts::for_role` for all roles + unknown role
- `RoleTimeouts::fill_from` merge behavior
- `BackendConfig::timeout_for_role` override + fallback
- TOML deserialize with and without `[backends.<name>.role_timeouts]`
- Partial backend merge of role timeouts

2. `src/backend/mod.rs`
- `BackendRegistry::timeout_for_role` with bare and modeled specs (e.g. `claude(opus)`)
- Unknown backend spec fallback to 7200
- `CliBackend::execute_with_log_and_timeout` uses provided timeout
- `execute` / `execute_with_log` still use backend default timeout

3. `src/backend/tmux_backend.rs`
- `execute_with_log_and_timeout` uses provided timeout path
- `execute` remains backward-compatible with inner timeout

#### Validate conformance tests
1. Update existing timeout failure test to assert enriched `BackendTimeoutExhausted` details (`role`, `timeout`).
2. Add conformance test: planner-specific timeout override causes planner timeout while fallback roles remain unaffected.
3. Add conformance test: acceptance QA uses `acceptance_qa` timeout, not `qa`.
4. Add conformance test: `config set/get` roundtrip for `backends.<backend>.role_timeouts.<role>` and `null` clearing.
5. Register any new validate module in `src/validate/mod.rs`.

### Implementation Notes
1. Avoid line-number-based requirements; target semantic call sites.
2. Keep `timeout_seconds` as canonical fallback and do not mark deprecated.
3. Preserve existing orchestrator logging and parse error context semantics unless required for timeout correctness.

### Verification Commands
1. `nix develop -c cargo check`
2. `nix develop -c cargo test`
3. `nix build -L`
4. `./result/bin/ralph validate --bin ./result/bin/ralph --filter timeout`
5. `./result/bin/ralph validate --bin ./result/bin/ralph`
