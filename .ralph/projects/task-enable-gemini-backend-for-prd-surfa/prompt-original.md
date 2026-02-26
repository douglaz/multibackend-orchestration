## Summary

Enable the gemini backend for all PRD surfaces — daemon interactive PRD flow, `ralph prd`, `ralph quick-prd`, and `ralph auto --spec-reviewer/--spec-writer` — while keeping gemini blocked on non-PRD `Required` surfaces.

**Current state:** `ValidationSurface::Required` blocks gemini from PRD roles, and `create_backend()` in `interactive_prd.rs` only recognizes `claude` and `codex`.

**Target state:** A new `Prd` validation surface variant allows gemini on PRD config fields, `create_backend()` gains a `"gemini"` arm with an enabled-check and `cwd` propagation, the output normalizer handles preamble-before-NDJSON routing, and conformance tests cover all entry points including argv verification for model injection.

## Acceptance Criteria

- Users can set `workspace.daemon_prd_reviewer_backend`, `workspace.daemon_prd_writer_backend`, and `workspace.daemon_prd_question_backends[]` to `gemini` or `gemini(<model>)` — daemon validates and executes successfully
- CLI commands accept gemini: `ralph prd --backend gemini`, `ralph quick-prd --reviewer-backend gemini`, `ralph quick-prd --writer-backend gemini`, `ralph auto --spec-reviewer gemini`, `ralph auto --spec-writer gemini`
- Bare `gemini` (no model parenthetical) is valid for PRD surfaces — model resolution follows same rules as `claude` and `codex`
- When `gemini(<model>)` is specified, the `--model <model>` flag is injected into the backend command args — verified by argv-capture tests that assert `--model` presence for modeled specs and absence for bare `gemini`
- When gemini is configured but `backends.gemini.enabled` is `false` or the backend fails: CLI exits non-zero, daemon PRD fails with error increment (issue transitions to `Failed` state after 3 consecutive failures with `ralph:prd-failed` label, daemon continues)
- Gemini restriction relaxed **only** for PRD surfaces — `starting_backend`, `planner_backend`, `implementer_backend`, `reviewer_backend` and other `Required` surfaces remain unchanged
- Optional syntax (`?gemini`) rejected on daemon PRD config surfaces — `ValidationSurface::Prd` does not enable `allows_optional()`
- For CLI entry points (`ralph prd`, `quick-prd`, `auto`), `?gemini` with unavailable backend propagates as hard error (no fallback) — `BackendRegistry::get_or_create_inner()` returns `BackendUnavailable` without checking the `optional` flag, and CLI entry points propagate the error as non-zero exit
- Output normalizer handles gemini PRD responses — long-form multi-section specs, stream-json events, preamble-before-NDJSON routing, preamble/429-error edge cases
- Regression test with realistic multi-section PRD output (6+ headings, 2KB+) locks in parsing correctness

## Technical Approach

### 1. Add `Prd` variant to `ValidationSurface`

**File:** `src/config/mod.rs` (lines 20–38)

Add `Prd` variant. Update `allows_gemini()` to include it. Leave `allows_optional()` unchanged so `?gemini` is rejected on daemon PRD surfaces.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationSurface {
    Required,
    RequiredPanel,
    PanelList,
    Prd,
}

impl ValidationSurface {
    fn allows_optional(self) -> bool {
        matches!(self, ValidationSurface::PanelList)
    }

    fn allows_gemini(self) -> bool {
        matches!(
            self,
            ValidationSurface::PanelList | ValidationSurface::RequiredPanel | ValidationSurface::Prd
        )
    }
}
```

### 2. Use `ValidationSurface::Prd` for PRD config validation

**File:** `src/config/mod.rs` (lines 531–563)

Change three `validate_backend_spec()` calls from `ValidationSurface::Required` to `ValidationSurface::Prd`:

- `daemon_prd_question_backends` entries (line ~543)
- `daemon_prd_writer_backend` (line ~551)
- `daemon_prd_reviewer_backend` (line ~557)

### 3. Update validation error message

**File:** `src/config/mod.rs` (lines 522–526)

Update the gemini-rejection message to mention PRD surfaces:

```
"gemini backend is not supported for {label}; it may only be used in panel surfaces (final review, completion, prompt review) or PRD surfaces"
```

### 4. Add `"gemini"` arm to `create_backend()` in `interactive_prd.rs`

**File:** `src/daemon/interactive_prd.rs` (lines 612–627)

The current signature is `fn create_backend(backend_spec: &str, global_config: &GlobalConfig, cwd: Option<PathBuf>) -> Result<CliBackend>`. The `cwd` parameter is propagated to all `backend_from_config` calls and is passed as `Some(repo_cwd)` at all callsites (lines ~1538, ~1792, ~1938). The updated function preserves this signature and `cwd` propagation:

```rust
fn create_backend(
    backend_spec: &str,
    global_config: &GlobalConfig,
    cwd: Option<PathBuf>,
) -> Result<CliBackend> {
    let spec = parse_backend_spec(backend_spec)?;
    // Enforce enabled check (mirrors BackendRegistry::get_or_create_inner at src/backend/mod.rs:874-882)
    if global_config
        .backend_config(&spec.name)
        .is_some_and(|cfg| cfg.enabled == BackendEnabled::Disabled)
    {
        return Err(RalphError::BackendUnavailable {
            backend: backend_spec.to_owned(),
        });
    }
    let model = spec.model.as_deref();
    match spec.name.as_str() {
        "claude" => Ok(claude::backend_from_config(global_config, model, None, cwd)),
        "codex" => Ok(codex::backend_from_config(global_config, model, None, cwd)),
        "gemini" => Ok(gemini::backend_from_config(global_config, model, None, cwd)),
        _ => Err(RalphError::Validation(format!(
            "unknown PRD backend: {backend_spec}"
        ))),
    }
}
```

Add imports at the top of `interactive_prd.rs`:
- Add `gemini` to the existing backend import: `use crate::backend::{claude, codex, gemini, parse_backend_spec, Backend, CliBackend};` (line 17)
- Add `use crate::config::global::BackendEnabled;` (new line after line 18)

**Note:** `gemini::backend_from_config` has the same signature as `claude::backend_from_config` and `codex::backend_from_config` — all take `(config, model, role, cwd)` and return `CliBackend`. The `cwd` is chained via `.with_cwd(cwd)` in all three implementations.

### 5. CLI entry points — no changes needed

`ralph prd`, `quick-prd`, and `auto` already use `BackendRegistry::get_or_create_for_spec()` which checks `backends.gemini.enabled` and routes gemini via `create_cli_backend_for_spec()`. CLI validation (`cli::backend_spec::validate_backend_spec()` at `src/cli/backend_spec.rs:11–20`) only checks backend name is known — no surface-type filtering.

**Optional syntax on CLI:** `parse_backend_spec()` accepts `?gemini` syntactically, but `BackendRegistry::get_or_create_inner()` (src/backend/mod.rs:884–892) returns `BackendUnavailable` if disabled **without checking `parsed.optional`**. CLI entry points propagate error as non-zero exit. No CLI validation change needed.

### 6. Output normalizer: fix preamble-before-NDJSON routing gap

**File:** `src/backend/output_normalizer.rs`

**Gap:** When gemini emits status lines (e.g., "YOLO mode is enabled…", "Loaded cached credentials.") before NDJSON stream events (`{"type":"init",...}`), `normalize_output()` at line 48 sees a non-`{` first content line, calls `try_extract_multiline_json_after_preamble()`, which attempts to parse a single multiline JSON object from each `{`-starting line downward. NDJSON lines are individual JSON objects per line, so joining them produces invalid JSON and `try_extract_multiline_json_after_preamble` returns `None`. The output is then returned as raw text — losing structured extraction (session_id, text fields).

**Fix:** In `normalize_output()`, after the `try_extract_multiline_json_after_preamble` check at line 51 but before returning raw text at line 67, add a fallback that scans for the first line starting with `{` that parses as a stream event (has a `type` field matching `STREAM_EVENT_TYPES`). If found, extract from that line onward and route to `normalize_claude_stream_json()`:

```rust
// Check for preamble followed by NDJSON stream events.
// Gemini CLI may output status lines before {"type":"init",...} events.
if let Some(stream_start) = raw.lines().position(|l| {
    let t = l.trim();
    t.starts_with('{')
        && serde_json::from_str::<Value>(t)
            .ok()
            .and_then(|v| v.get("type")?.as_str().map(str::to_owned))
            .is_some_and(|t| STREAM_EVENT_TYPES.contains(&t.as_str()))
}) {
    let stream_portion: String = raw.lines().skip(stream_start).collect::<Vec<_>>().join("\n");
    tracing::debug!(
        path = "preamble_stream",
        preamble_lines = stream_start,
        "normalize_output: detected preamble before NDJSON stream events"
    );
    return normalize_claude_stream_json(&stream_portion);
}
```

**Note:** `normalize_claude_stream_json` already skips non-JSON lines (line 123: `let Ok(event) = serde_json::from_str::<Value>(trimmed) else { continue; }`), so passing just the stream portion is sufficient. We skip preamble lines explicitly to avoid the edge case where preamble text coincidentally starts with `{`.

**Investigate PRD-length responses additionally:**
- Verify multi-KB `response` fields in `result` events parse correctly via `extract_result_event_text()`
- Verify `check_spec_sections()` (`src/prd/quick.rs`) operates correctly on gemini-produced plain text
- Add regression tests with realistic multi-section PRD output; fix any additional parsing gaps discovered

### 7. Test harness helper

**File:** `src/validate/harness.rs`

Add `setup_mock_backends_with_gemini()` — extends `setup_mock_backends_stable` pattern to also configure and enable the gemini backend:

```rust
/// Configure mock backends for claude, codex, gemini and enable gemini.
/// Use for tests exercising gemini as PRD backend.
pub fn setup_mock_backends_with_gemini<P: AsRef<Path>>(&self, script: P) -> Result<()> {
    let script = script.as_ref().to_string_lossy().into_owned();
    let wrapper_content = format!("#!/bin/sh\nexec bash \"{script}\"\n");
    let wrapper = self.write_mock_script("mock-wrapper.sh", &wrapper_content)?;
    let wrapper_str = wrapper.to_string_lossy().into_owned();

    for backend in &["claude", "codex", "gemini"] {
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.command"),
            wrapper_str.clone(),
            "--global".to_owned(),
        ])?;
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.args"),
            "[]".to_owned(),
            "--global".to_owned(),
        ])?;
    }
    self.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.gemini.enabled".to_owned(),
        "true".to_owned(),
        "--global".to_owned(),
    ])?;
    Ok(())
}
```

**Argv-capture variant:** Add `setup_mock_backends_with_gemini_argv_capture()` that uses a wrapper script which logs all positional arguments to a file before executing the real script. This is needed for tests that verify `--model` flag injection:

```rust
/// Like `setup_mock_backends_with_gemini` but the wrapper logs all argv to a
/// capture file before executing the script. Set `RALPH_ARGV_CAPTURE` env var
/// on the backend to point to the log file.
pub fn setup_mock_backends_with_gemini_argv_capture<P: AsRef<Path>>(
    &self,
    script: P,
) -> Result<()> {
    let script = script.as_ref().to_string_lossy().into_owned();
    // Wrapper captures "$@" to RALPH_ARGV_CAPTURE before exec'ing the script.
    let wrapper_content = format!(
        "#!/bin/sh\n\
         if [ -n \"${{RALPH_ARGV_CAPTURE:-}}\" ]; then\n\
           printf '%s\\n' \"$@\" >> \"$RALPH_ARGV_CAPTURE\"\n\
         fi\n\
         exec bash \"{script}\"\n"
    );
    let wrapper = self.write_mock_script("mock-wrapper-argv.sh", &wrapper_content)?;
    let wrapper_str = wrapper.to_string_lossy().into_owned();

    for backend in &["claude", "codex", "gemini"] {
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.command"),
            wrapper_str.clone(),
            "--global".to_owned(),
        ])?;
        self.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.args"),
            "[]".to_owned(),
            "--global".to_owned(),
        ])?;
    }
    self.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.gemini.enabled".to_owned(),
        "true".to_owned(),
        "--global".to_owned(),
    ])?;
    Ok(())
}
```

**Daemon PRD config helper:** Use load-mutate-save via `GlobalConfig::load()`/`save()` because `set_global_config_value` rejects `workspace.daemon_prd_*` keys by design (see `shared_mutator_rejects_daemon_prd_keys` test at line 2956 of `global.rs`):

```rust
fn set_daemon_prd_backend(h: &RalphHarness, field: &str, value: &str) {
    let toml_path = h.repo_root.join(".ralph").join("ralph.toml");
    let mut config = GlobalConfig::load(&toml_path).expect("load config");
    match field {
        "daemon_prd_reviewer_backend" => {
            config.workspace.daemon_prd_reviewer_backend = value.to_owned();
        }
        "daemon_prd_writer_backend" => {
            config.workspace.daemon_prd_writer_backend = value.to_owned();
        }
        "daemon_prd_question_backends" => {
            config.workspace.daemon_prd_question_backends =
                serde_json::from_str(value).expect("parse question backends JSON array");
        }
        _ => panic!("unknown daemon PRD field: {field}"),
    }
    config.save(&toml_path).expect("save patched config");
}
```

### 8. Update existing test for gemini rejection

**File:** `src/config/mod.rs` (line 1149)

**Replace** `validate_prd_config_rejects_gemini_backend_specs` with acceptance and rejection tests (see Testing Strategy section 1–6).

### 9. Test configuration strategy for daemon PRD

CLI tests pass gemini via command-line flags and don't need config file manipulation. Daemon tests use the load-mutate-save helper from section 7.

## Files & Modules

| File | Change |
|------|--------|
| `src/config/mod.rs` | Add `Prd` variant to `ValidationSurface`; update `allows_gemini()`; change PRD validation calls to `ValidationSurface::Prd`; update error message; replace gemini-rejection test with acceptance + optional-syntax rejection tests |
| `src/daemon/interactive_prd.rs` | Add `gemini` to backend import, add `BackendEnabled` import; add `"gemini"` match arm + `BackendEnabled::Disabled` check to `create_backend()` preserving existing `cwd: Option<PathBuf>` parameter |
| `src/backend/output_normalizer.rs` | Add preamble-before-NDJSON routing fallback in `normalize_output()`; add regression tests with realistic multi-section PRD responses (pipe-mode, stream-json, preamble + stream-json) |
| `src/validate/harness.rs` | Add `setup_mock_backends_with_gemini()` and `setup_mock_backends_with_gemini_argv_capture()` helpers |
| `src/validate/tests_interactive_prd.rs` | Add daemon PRD conformance tests for gemini (reviewer success, modeled success with argv verification, disabled-fails, question backend, writer) |
| `src/validate/tests_gemini_backend.rs` | Add conformance for gemini acceptance on PRD surfaces, `?gemini` rejection, non-PRD `Required` still rejects |
| `src/validate/tests_prd.rs` | Add conformance for `ralph prd`, `quick-prd`, `auto` with gemini (success, disabled-fail, modeled with argv verification, optional-syntax hard error on all three CLI paths) |
| `src/validate/mock_scripts.rs` | Add mock script variant or env-var support for argv capture if needed by argv-capture tests |

## Testing Strategy

### Unit tests (`src/config/mod.rs`)

1. `validate_prd_config_accepts_gemini_reviewer` — bare `gemini` accepted as reviewer
2. `validate_prd_config_accepts_gemini_with_model` — `gemini(gemini-3-pro-preview)` accepted as question backend
3. `validate_prd_config_accepts_gemini_question_backend` — bare `gemini` accepted as question backend
4. `validate_prd_config_accepts_gemini_writer` — bare `gemini` accepted as writer
5. `validate_non_prd_required_still_rejects_gemini` — `starting_backend = "gemini"` still rejected
6. `validate_prd_config_rejects_optional_gemini` — `?gemini` on daemon PRD surfaces rejected

### Daemon interactive PRD conformance (`src/validate/tests_interactive_prd.rs`)

7. `gemini_reviewer_success` — bare `gemini` reviewer using `setup_mock_backends_with_gemini()`; state progresses through `AwaitingAnswers` → `AwaitingFeedback` → `Done`; output passes `check_spec_sections()`
8. `gemini_reviewer_with_model_argv_verified` — `gemini(gemini-3-pro-preview)` reviewer using `setup_mock_backends_with_gemini_argv_capture()`; assert state progresses to `Done`; read argv capture file and assert it contains `--model` followed by `gemini-3-pro-preview`
9. `gemini_reviewer_bare_no_model_argv_verified` — bare `gemini` reviewer using `setup_mock_backends_with_gemini_argv_capture()`; assert state progresses to `Done`; read argv capture file and assert `--model` does NOT appear
10. `gemini_reviewer_disabled_fails` — gemini reviewer with `setup_mock_backends_stable()` (disabled); assert `error_count` increments, issue → `Failed` after 3 failures
11. `gemini_question_backend_success` — `daemon_prd_question_backends = ["claude", "gemini"]`; assert question generation completes
12. `gemini_writer_success` — gemini writer; assert draft generation completes

### CLI conformance — `ralph prd` (`src/validate/tests_prd.rs`)

13. `prd_gemini_backend_success` — `ralph prd --backend gemini --idea "test" --non-interactive` with `setup_mock_backends_with_gemini()`; assert exit 0
14. `prd_gemini_backend_with_model_argv_verified` — `ralph prd --backend "gemini(gemini-3-pro-preview)" ...` with `setup_mock_backends_with_gemini_argv_capture()` + `RALPH_ARGV_CAPTURE` env on gemini backend; assert exit 0; read capture file and assert `--model` + `gemini-3-pro-preview` present
15. `prd_gemini_backend_bare_no_model_argv_verified` — `ralph prd --backend gemini ...` with argv capture; assert exit 0; read capture file and assert `--model` absent
16. `prd_gemini_backend_disabled_fails` — `ralph prd --backend gemini ...` with gemini disabled; assert non-zero exit, stderr mentions "gemini"
17. `prd_optional_gemini_disabled_fails` — `ralph prd --backend "?gemini" ...` with gemini disabled; assert non-zero exit (hard error, no fallback)

### CLI conformance — `quick-prd` (`src/validate/tests_prd.rs`)

18. `quick_prd_gemini_reviewer_success` — `ralph quick-prd --reviewer-backend gemini ...`; assert exit 0
19. `quick_prd_gemini_reviewer_disabled_fails` — reviewer gemini disabled; assert non-zero exit
20. `quick_prd_gemini_writer_success` — `ralph quick-prd --writer-backend gemini ...`; assert exit 0
21. `quick_prd_gemini_writer_disabled_fails` — writer gemini disabled; assert non-zero exit
22. `quick_prd_optional_gemini_reviewer_disabled_fails` — `ralph quick-prd --reviewer-backend "?gemini" ...` with gemini disabled; assert non-zero exit (hard error)
23. `quick_prd_optional_gemini_writer_disabled_fails` — `ralph quick-prd --writer-backend "?gemini" ...` with gemini disabled; assert non-zero exit (hard error)

### CLI conformance — `auto` (`src/validate/tests_prd.rs`)

24. `auto_spec_reviewer_gemini_success` — `ralph auto --spec-reviewer gemini --idea "test" --dry-run`; assert exit 0
25. `auto_spec_reviewer_gemini_disabled_fails` — spec-reviewer gemini disabled; assert non-zero exit
26. `auto_spec_writer_gemini_success` — `ralph auto --spec-writer gemini --idea "test" --dry-run`; assert exit 0
27. `auto_spec_writer_gemini_disabled_fails` — spec-writer gemini disabled; assert non-zero exit
28. `auto_optional_gemini_spec_reviewer_disabled_fails` — `ralph auto --spec-reviewer "?gemini" ...` with gemini disabled; assert non-zero exit (hard error)
29. `auto_optional_gemini_spec_writer_disabled_fails` — `ralph auto --spec-writer "?gemini" ...` with gemini disabled; assert non-zero exit (hard error)

### Gemini guardrails (`src/validate/tests_gemini_backend.rs`)

30. `prd_surface_accepts_gemini` — validate gemini config passes for daemon PRD surfaces
31. `prd_surface_rejects_optional_gemini` — validate `?gemini` rejected on daemon PRD surfaces
32. `guardrails_still_reject_non_prd_required` — non-PRD `Required` surfaces still reject gemini (update `guardrails_reject_disallowed_surfaces` if needed)

### Output normalizer (`src/backend/output_normalizer.rs`)

33. `normalize_output_gemini_long_prd_response` — realistic gemini pipe-mode output with multi-section PRD review (6 headings: Summary, Acceptance Criteria, Technical Approach, Files & Modules, Testing Strategy, Out of Scope — 2KB+); verify full text + session ID extraction
34. `normalize_output_gemini_stream_long_prd_response` — gemini stream-json with long multi-section `result` event (2KB+ `response` field); verify full text extraction and session_id
35. `normalize_output_gemini_preamble_before_stream_json` — preamble status lines ("YOLO mode is enabled.", "Loaded cached credentials.") followed by NDJSON stream events (`{"type":"init",...}`, `{"type":"message",...}`, `{"type":"result",...}`); verify that text is extracted from stream events (not returned as raw text), session_id is retained from `init` event, and `result` event text is preferred over concatenated `message` text

## Out of Scope

- Enabling gemini for non-PRD `ValidationSurface::Required` surfaces (separate follow-up)
- Changing default PRD backends to include gemini — defaults remain `claude`/`codex`; gemini is opt-in
- Changing "exactly 2 question backends" constraint — stays as-is, gemini can be one of two
- Adding `backends.gemini.models.reviewer` default model config — bare `gemini` uses CLI's built-in default
- Gemini support in `BackendRegistry::opposite()` (currently claude↔codex) — not needed for PRD
- Performance benchmarking gemini vs claude/codex for PRD
- Role-based model injection for gemini in PRD contexts — PRD backends created without role parameter
- Adding `workspace.daemon_prd_*` keys to `set_global_config_value()` — intentionally excluded by design
- Adding CLI-level surface validation for optional syntax on PRD CLI paths — current behavior (hard error via `BackendRegistry`) is correct; `?` optional syntax is only meaningful in daemon config panel lists