---
artifact: termination-request
loop: 3
project: daemon-prompt-refinement
backend: claude(opus)
role: planner
created_at: 2026-02-13T23:12:03Z
---

All 9 acceptance criteria from the master prompt have been fully implemented and tested across both completed loops. Here is the completion response:

# Project Completion Request

## Rationale
All 9 acceptance criteria from the master prompt are fully satisfied by the work completed in loops 1 and 2:

1. **Issue body fetched (AC 1)**: `GhIssue` and `RawGhIssue` have `body: Option<String>`. `poll_issues()` requests `"number,title,labels,body"`. `fetch_issue_body()` exists for restart recovery.
2. **Refinement produces a structured prompt (AC 2)**: `src/daemon/refine.rs` implements `refine_prompt()` with `REFINEMENT_SYSTEM_PROMPT`, backend parsing, and output validation (< 20 chars → error).
3. **Refined prompt posted as comment (AC 3)**: `dispatch_task()` calls `post_idempotent_comment(..., "refined-prompt", &idea)` best-effort — failures log a warning and never abort dispatch.
4. **Refined prompt used as --idea (AC 4)**: `build_ralph_auto_command()` uses `["auto", "--idea", idea]`.
5. **Graceful fallback (AC 5)**: Refinement errors are caught in `dispatch_task()`, logged as warnings, and the raw idea is used instead.
6. **Configuration (AC 6)**: `daemon_refinement_enabled` and `daemon_refinement_backend` are plumbed through `WorkspaceConfig` → `ProjectDaemonOverrides` → `EffectiveDaemonConfig` → `DaemonRuntimeConfig` with correct defaults (`true`, `"claude(sonnet)"`).
7. **Timing & ordering (AC 7)**: `dispatch_task()` enforces `create_worktree` → `refine_prompt` → `post comment` → `spawn_ralph_auto` → CAS update.
8. **spawn_ralph_auto argv correctness (AC 8)**: Uses `["auto", "--idea", idea]` with a unit test asserting exact argv structure.
9. **Restart with missing raw_idea (AC 9)**: `adopt_pending_tasks()` calls `fetch_issue_body()` for legacy tasks with `raw_idea == None`. Never falls back to `"Implement task {task_id}"`.

## Summary of Work

**Loop 1 — Data Layer, Configuration, and Argv Foundation:**
- Extended `GhIssue`/`RawGhIssue` with `body: Option<String>` and added `fetch_issue_body()` for restart recovery
- Added `raw_idea: Option<String>` to `DaemonTask` with serde backwards compatibility
- Fixed `spawn_ralph_auto()` argv from positional to `["auto", "--idea", idea]`
- Plumbed `daemon_refinement_enabled` and `daemon_refinement_backend` through the three-tier config system
- Added `global_config` to `DaemonRuntimeConfig`
- Unit tests for deserialization, config defaults, argv correctness, and backwards compatibility

**Loop 2 — Prompt Refinement Dispatch Integration:**
- Created `src/daemon/refine.rs` with `refine_prompt()`, system prompt constant, and `Handle::current().block_on()` async bridge
- Integrated refinement + fallback + best-effort comment posting into `dispatch_task()` with strict ordering
- Added 8 conformance tests: strict ordering, happy path, refinement failure fallback, comment failure non-blocking, refinement disabled, restart with legacy task, retry idempotency, and metadata fallback

## Remaining Items
- None
