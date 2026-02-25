---
artifact: final-review-proposals
loop: 8
project: task-analysis-this-is-a-comprehensive-gi
backend: codex
role: final_reviewer
created_at: 2026-02-23T01:28:29Z
---

# Final Review: AMENDMENTS

## Amendment: FR-PR-ALIAS-OPTIONAL-001

### Problem
`?backend` is currently accepted through the singular prompt-review alias surface, which violates the constraint that optional syntax is invalid on single-backend required surfaces.

The singular alias is accepted by config set (`src/cli/config.rs:671`, `src/cli/config.rs:990`), then promoted into the panel list (`src/config/mod.rs:186`). During validation, the spec is canonicalized before validation (`src/config/mod.rs:647`), and canonicalization strips the optional marker (`src/config/mod.rs:817`), so the optional-surface check in `validate_backend_spec` (`src/config/mod.rs:497`) is bypassed for this path.

I confirmed this behavior via CLI: setting `workflow.prompt_review_backend` to `?gemini` succeeds and resolves as `["?gemini"]`.

### Proposed Change
Reject optional syntax on the singular alias before alias synthesis/canonicalization. Concretely:
- Validate `workflow.prompt_review_backend` as a required surface (`ValidationSurface::Required`) when present.
- In list normalization, validate the raw spec first (to preserve `optional`), then canonicalize for dedupe/model normalization.
- Add tests that `workflow.prompt_review_backend = "?gemini"` fails for both global/project config paths.

### Affected Files
- `src/config/mod.rs` - enforce required-surface validation for singular alias; adjust normalization order.
- `src/cli/config.rs` - ensure singular-key set path rejects optional syntax consistently.
- `src/validate/tests_prompt_review_panel.rs` - add conformance coverage for singular-alias optional rejection.

## Amendment: FR-DAEMON-REFINEMENT-GUARDRAIL-002

### Problem
Gemini is not fully guardrailed for daemon refinement when configured via project override.

Project-level override exists (`src/config/project.rs:95`) and can be set without guardrail rejection (`src/cli/config.rs:811`). The merged daemon config uses that override (`src/config/mod.rs:439`), but daemon startup validates only workspace-level refinement backend (`src/cli/daemon.rs:167` calling `src/config/mod.rs:552`). So project override can bypass the intended Gemini rejection for daemon refinement. Failure is deferred to runtime refine backend creation (`src/daemon/refine.rs:53`, `src/daemon/refine.rs:56`) as "unknown refinement backend," not explicit guardrail validation.

### Proposed Change
Validate the effective daemon refinement backend (post-merge) as a required non-Gemini surface:
- Add validation against `daemon_cfg.refinement_backend` after `resolve_daemon_config` in daemon startup.
- Centralize this in config validation (effective daemon config validator) so all call paths are consistent.
- Add tests for project override rejection (`daemon.refinement_backend = gemini(...)`) in daemon startup/config validation.

### Affected Files
- `src/cli/daemon.rs` - validate merged daemon refinement backend, not only workspace raw config.
- `src/config/mod.rs` - add effective daemon refinement validation helper and tests.
- `src/validate/tests_gemini_backend.rs` - add conformance case for project-level daemon refinement guardrail.
