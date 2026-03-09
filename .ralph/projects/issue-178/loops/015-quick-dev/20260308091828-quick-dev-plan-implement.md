---
artifact: quick-dev-plan-implement
loop: 15
project: issue-178
backend: claude
role: implementer
created_at: 2026-03-08T09:18:28Z
---

# Implementation: Reviewer Amendments P1, P2, P3

## Amendment P1: Rollback Ceiling Provenance — Prevents Stale Checkpoint Resurrection

**Problem:** The ceiling guard `checkpoint_loop > ceiling && checkpoint_loop > max_artifact_loop` fails when a stale checkpoint has the same loop number as newly recreated artifacts (before any new checkpoint commit is written). The marker becomes inert prematurely, letting the stale checkpoint resurrect the pre-rollback position.

**Solution:** Persist checkpoint hash provenance in the `.rollback-ceiling` marker file.

### Changes

**`src/project/lifecycle.rs`:**
- `read_rollback_ceiling` now returns `Option<(u32, Option<String>)>` — ceiling number plus optional checkpoint hash recorded at rollback time (backward-compat: old markers without hash use `None`)
- Ceiling enforcement uses hash-based staleness: when `rollback_hash` is present and matches the latest checkpoint hash, the marker is still active (same stale checkpoint → cap). When hashes differ (new checkpoint committed), the marker is inert. When no rollback hash (backward compat), falls back to `checkpoint_loop > max_artifact_loop`.
- Two new unit tests:
  - `reconstruct_ceiling_stale_checkpoint_equals_max_artifact` — regression test for the exact P1 scenario (checkpoint_loop == max_artifact_loop > ceiling with matching hash → caps correctly)
  - `reconstruct_ceiling_inert_after_new_checkpoint` — verifies ceiling becomes inert when a new checkpoint (different hash) appears

**`src/cli/rollback.rs`:**
- Computes `latest_checkpoint_hash` before any git mutations (via `list_ralph_commits`)
- Writes enriched `.rollback-ceiling` marker with format `{ceiling}\n{checkpoint_hash}` for both soft rollback and hard rollback with push failure

### Marker file format (two lines, second optional):
```
1
abc123def456...
```

## Amendment P2: Remote Branch Probe Tri-State — Distinguishes Missing from Unreachable

**Problem:** `remote_branch_exists_on_remote` returns `false` on any non-zero `ls-remote` exit status, misclassifying connectivity/auth failures as "branch missing".

**Solution:** Return a tri-state result with distinct handling for each case.

### Changes

**`src/git/branch.rs`:**
- Added `RemoteBranchProbeResult` enum with `Exists`, `Missing`, `ProbeFailed(String)` variants
- Changed `remote_branch_exists_on_remote` to return `Result<RemoteBranchProbeResult>` using `Command::new("git")` directly (to capture stderr)
- Exit code 0 → `Exists`, exit code 2 → `Missing`, other → `ProbeFailed(stderr)`

**`src/cli/rollback.rs`:**
- Both dry-run and execution path callers now match on the tri-state:
  - `Exists` → proceed as before
  - `Missing` → `RalphError::Validation` (branch doesn't exist)
  - `ProbeFailed(stderr)` → `RalphError::Orchestration` with actionable stderr

## Amendment P3: Push-Failure Test Session Assertions — Documented as Structural

**Problem:** The `rollback_push_failure_continues` test's session invalidation assertions pass vacuously because no session records are created.

**Analysis:** `CliBackend.execute_streaming` (mod.rs ~line 710) normalizes output and discards `session_id` via `.map(|n| n.text)`. Even if a mock emitted NDJSON with session_id, the double-normalization pipeline would lose it. Making the test non-vacuous requires architectural changes to the backend normalization layer.

**Change:** Updated the test comment in `src/validate/tests_commands.rs` to precisely document the CliBackend limitation and defer the fix to a backend-layer follow-up. Real session invalidation logic is already verified by `state.rs::remove_loop_clears_session_records`.
