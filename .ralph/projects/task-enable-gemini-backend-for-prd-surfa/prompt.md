### Feature
Enable `gemini` on PRD-only surfaces across daemon and CLI PRD entry points, while keeping `gemini` blocked on non-PRD `Required` surfaces.

### Goal
Implement and verify PRD-specific `gemini` support for:
- Daemon interactive PRD config surfaces:
`workspace.daemon_prd_reviewer_backend`, `workspace.daemon_prd_writer_backend`, `workspace.daemon_prd_question_backends[]`
- CLI PRD surfaces:
`ralph prd --backend`, `ralph quick-prd --reviewer-backend`, `ralph quick-prd --writer-backend`, `ralph auto --spec-reviewer`, `ralph auto --spec-writer`

Do not broaden `gemini` support beyond PRD surfaces.

### Required Behavior Matrix
| Surface | `gemini` | `gemini(<model>)` | `?gemini` | Disabled backend behavior |
|---|---|---|---|---|
| Daemon PRD config fields | Allowed | Allowed | Rejected at validation | Runtime failure (`BackendUnavailable`), daemon PRD failure path preserved |
| CLI PRD flags (`prd`, `quick-prd`, `auto`) | Allowed | Allowed | Parsed but treated as hard error when unavailable (no fallback) | Non-zero exit with backend-unavailable behavior |
| Non-PRD `Required` surfaces | Rejected | Rejected | Rejected | Unchanged |

### Implementation Requirements
1. `src/config/mod.rs`
- Add `ValidationSurface::Prd`.
- `allows_gemini()` must include `Prd`.
- `allows_optional()` must remain unchanged (only existing optional surfaces remain optional).
- Validate daemon PRD fields using `ValidationSurface::Prd` instead of `Required`.
- Update error message to state `gemini` is allowed only on panel surfaces and PRD surfaces.
- Add/adjust unit tests to cover:
`gemini` accepted on PRD reviewer/writer/question backends, `gemini(model)` accepted, `?gemini` rejected on PRD config, non-PRD `Required` still rejects `gemini`.

2. `src/daemon/interactive_prd.rs`
- Update `create_backend()` to support `"gemini"`.
- Keep signature with `cwd: Option<PathBuf>` and propagate `cwd` for all backends.
- Add enabled check equivalent to registry behavior: if `backends.gemini.enabled = false`, return `RalphError::BackendUnavailable`.
- Unknown backend continues returning validation error.

3. `src/backend/output_normalizer.rs`
- Fix preamble-before-NDJSON routing:
if output starts with non-JSON lines, scan for first NDJSON stream event (`type` in `STREAM_EVENT_TYPES`) and route stream portion to `normalize_claude_stream_json()`.
- Preserve existing behavior for plain text and multiline JSON extraction.
- Ensure extraction rules:
prefer `result` event text over concatenated `message` text when both exist, and retain `session_id` from `init`.

4. `src/validate/harness.rs`
- Add `setup_mock_backends_with_gemini(script)` that configures `claude`, `codex`, `gemini` and enables gemini.
- Add `setup_mock_backends_with_gemini_argv_capture(script)` that logs backend argv to file path in `RALPH_ARGV_CAPTURE` before executing script.

### Conformance Test Requirements
Add/extend validate tests to cover all PRD entry points and guardrails.

1. Daemon interactive PRD
- Reviewer success with bare `gemini`.
- Reviewer success with `gemini(model)` and argv-capture asserting `--model <model>` present.
- Reviewer success with bare `gemini` and argv-capture asserting `--model` absent.
- Reviewer disabled case verifies daemon PRD failure path (error increments; existing fail-after-3 behavior preserved).
- Question backend list including `gemini` succeeds.
- Writer backend `gemini` succeeds.

2. CLI `ralph prd`
- Bare `gemini` success.
- `gemini(model)` argv includes `--model`.
- Bare `gemini` argv omits `--model`.
- Disabled `gemini` fails non-zero.
- `?gemini` with unavailable backend fails non-zero (hard error).

3. CLI `quick-prd`
- Reviewer and writer each: bare `gemini` success.
- Reviewer and writer each: disabled `gemini` fails non-zero.
- Reviewer and writer each: `?gemini` unavailable fails non-zero.

4. CLI `auto`
- `--spec-reviewer gemini` and `--spec-writer gemini` succeed.
- Disabled `gemini` fails non-zero for both flags.
- `?gemini` unavailable fails non-zero for both flags.

5. Guardrails
- Non-PRD `Required` surfaces still reject `gemini`.
- Daemon PRD surfaces reject `?gemini`.

6. Output normalizer regression tests
- Long (2KB+) multi-section PRD plain output extraction.
- Stream JSON long PRD extraction from `result`.
- Preamble + NDJSON stream routing with session ID retention and `result`-text preference.

### Constraints
- Do not change default backend selection behavior outside this feature.
- Do not enable `gemini` for non-PRD `Required` surfaces.
- Do not change optional-syntax semantics globally.
- Do not change `workspace.daemon_prd_*` mutator policy unless required by tests; daemon tests may use load-mutate-save config helper.

### Acceptance Criteria
- All PRD surfaces listed above accept `gemini` and `gemini(<model>)`.
- PRD daemon config rejects `?gemini`.
- CLI PRD paths treat unavailable `?gemini` as hard error (non-zero, no fallback).
- `--model` injection occurs only when model is explicitly specified in backend spec.
- Non-PRD `Required` surfaces continue rejecting `gemini`.
- Output normalizer correctly handles preamble-before-NDJSON and long PRD responses.
- Build and test pass:
`nix develop -c cargo check`, `nix develop -c cargo test`, `nix build -L`, `./result/bin/ralph validate --bin ./result/bin/ralph`.