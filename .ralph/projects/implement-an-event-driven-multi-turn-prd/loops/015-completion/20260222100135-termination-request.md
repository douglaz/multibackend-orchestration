---
artifact: termination-request
loop: 15
project: implement-an-event-driven-multi-turn-prd
backend: claude(opus)
role: planner
created_at: 2026-02-22T10:01:35Z
---

I now have a comprehensive picture of the project. All amendments from all 3 rounds have been implemented:

- **PRD-AMEND-01** (bot-login retry): Bot login resolution is inside `get_or_fetch_bot_login()` chained with `.and_then()`, results go through `finish_transition()` for retry accounting. Tests present.
- **PRD-AMEND-02** (approval label ordering): `ralph:prd-done` added before `ralph:prd-active` removed (line 849-855). Tests present.
- **PRD-REV-001** (nix binary resolution): `ralph_bin_absolute()` uses `option_env!("CARGO_BIN_EXE_ralph")` + runtime env + multi-path search.
- **PRD-REV-002** (strict 6-section validation): `missing.is_empty()` checks throughout.
- **PRD-AMD-01** (terminal persistence safety): Save before label removal in both Done (line 857-874) and Failed (line 1425-1442) paths.
- **PRD-AMD-02** (bot-scoped markers): `find_bot_comment_with_marker()` and `post_bot_comment_with_marker()` scope by bot login.

All acceptance criteria are met:
- Config fields with defaults
- Config validation (exactly 2 backends, fail fast at startup)
- Label ensure at startup
- State machine with all transitions
- Idempotent markers
- Approval detection
- Error handling with retry exhaustion
- 56 unit tests, 26 integration tests, 33 conformance tests
- Runtime integration in `runtime.rs`
- Module export in `mod.rs`
- Error variant in `error.rs`

# Project Completion Request

## Rationale
All requirements from the master prompt are fully implemented and verified across 9 completed implementation loops (1-4, 6, 8, 10, 12, 14). Every acceptance criterion, hard constraint, amendment from all three review rounds, and testing requirement has been addressed:

**Core State Machine**: All five states (`Pending`, `AwaitingAnswers`, `AwaitingFeedback`, `Done`, `Failed`) with all specified transitions implemented in `src/daemon/interactive_prd.rs`.

**Lifecycle Labels**: `ensure_prd_labels_best_effort()` creates all 5 PRD labels idempotently at startup. `has_prd_label()` excludes PRD issues from the normal `ralph:ready` claim flow.

**Comment & Approval Rules**: Bot filtering by `author_login`, approval detection with code-block stripping and negative-phrase priority, word-boundary matching for `approved`/`lgtm`/`ship it`/`looks good`.

**Persistence**: Atomic JSON persistence per issue at `{data_dir}/{owner}/{repo}/.ralph/interactive-prd/{issue_number}.json` with all required fields.

**Configuration**: All 6 `daemon_prd_*` fields on `WorkspaceConfig` with correct defaults and validation (exactly 2 question backends, backend spec parsing, fail-fast at startup).

**All 6 Amendments Resolved**:
- PRD-AMEND-01: Bot-login failures routed through `finish_transition` retry accounting
- PRD-AMEND-02: `ralph:prd-done` added before `ralph:prd-active` removed
- PRD-REV-001: Layout-agnostic `ralph_bin_absolute()` with compile-time/runtime/search fallbacks
- PRD-REV-002: `missing.is_empty()` enforced for all draft/revision acceptance
- PRD-AMD-01: Terminal state persisted before poll-visible label removal (both Done and Failed paths)
- PRD-AMD-02: Bot-scoped marker idempotency via `find_bot_comment_with_marker()`

**Testing**: 56 unit tests, 26 integration tests, 33 conformance tests covering all specified scenarios including edge cases from amendments.

## Summary of Work
- **Loop 1**: State machine foundation — `InteractivePrdState`, `PrdWorkflowState`, serialization, persistence, label constants, error variant
- **Loop 2**: Pending→AwaitingAnswers transition — label swap, dual-backend question generation, synthesis, idempotent marker posting
- **Loop 3**: AwaitingAnswers→AwaitingFeedback — answer extraction, draft generation with writer/reviewer/section validation pipeline
- **Loop 4**: AwaitingFeedback transitions — revision loop, approval detection, Done and Failed terminal transitions
- **Loop 6**: Boundary-safe comment processing, restart hydration from persisted state, marker timestamp extraction
- **Loop 8**: Reliability hardening — retry accounting, error count persistence, graceful degradation
- **Loop 10**: Round-2 compliance — strict 6-section validation, nix-compatible binary resolution, section-incomplete failure paths
- **Loop 12**: Terminal persistence safety — save-before-label-remove pattern, bot-scoped marker idempotency, spoof resistance
- **Loop 14**: Pickup conformance alignment — bot login plumbing for question marker hydration

## Remaining Items
- None — all specified requirements and amendments are implemented and tested

---
