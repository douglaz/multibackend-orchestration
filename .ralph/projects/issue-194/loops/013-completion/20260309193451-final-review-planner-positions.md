---
artifact: final-review-planner-positions
loop: 13
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T19:34:51Z
---

# Planner Positions

## Amendment: STRAY-IMPL-NOTES-001

### Position
ACCEPT

### Rationale
Verified: the file `20260309T190541Z-impl-notes.md` exists at the repository root and is tracked by git (confirmed via `git ls-files --error-unmatch`). It was committed in `6885d06` (loop 12 implementing→reviewing). Its content is purely ephemeral implementation notes ("Decisions Made", "Spec Deviations", "Testing") from the loop-12 amendment round — not project source code. This is a legitimate cleanup item regardless of why the stray-artifact detection didn't catch it.

## Amendment: AMQ-UTF8-READ-001

### Position
ACCEPT

### Rationale
Verified at `src/project/amendments.rs:491-494`: `read_and_parse_inflight` uses `fs::read_to_string(path)`. In Rust, `fs::read_to_string` returns `std::io::Error` with `ErrorKind::InvalidData` when the file contains invalid UTF-8. This is an `io::Error`, so it follows the `ReadFailed` path (line 494), which at line 246-248 triggers `rollback_mid_drain` — aborting the entire drain and re-enqueuing all previously processed items.

This is a genuine correctness defect. Invalid UTF-8 is a *content* problem (the file was successfully read from disk), not a filesystem I/O failure. The `InflightReadOutcome` enum (line 480-488) explicitly documents this separation: "File was readable but content was invalid" → `Malformed` (quarantine), vs. "File could not be read due to an I/O error" → `ReadFailed` (fatal). A non-UTF-8 file should be quarantined via the `Malformed` path, not abort the entire drain. The proposed fix (use `fs::read` + `serde_json::from_slice`, classify decode failures as `Malformed`) correctly addresses this.

## Amendment: AMQ-STEM-RACE-002

### Position
ACCEPT

### Rationale
Verified the race condition by tracing the code paths:

1. **Drain** reads the directory listing at lines 180-190, creating a snapshot of filenames including `stem.json`.
2. **Drain** claims `stem.json` → `stem.inflight` via `claim_file_without_overwrite` (line 216), which hard-links then deletes the original. Now `stem.json` no longer exists on disk.
3. **Concurrent enqueue** (separate CLI process) tries to publish to the same stem path. Enqueue's collision check at lines 147-152 only tests whether the `.json` destination exists — it does NOT check for `.inflight`. Since `stem.json` was just deleted by drain, enqueue succeeds, creating a NEW `stem.json` with different content.
4. **Drain** processes `stem.inflight`, adds stem to `completed_inflight_stems` (line 254-255).
5. **Drain** reaches the `stem.json` entry from its original snapshot (line 197-208). Stem matches `completed_inflight_stems`, so it calls `remove_file_if_exists(&queued_path)` — which deletes the NEW file (content B), not the original.

Result: **silent data loss** of a legitimately enqueued amendment. The test at line 750 (`drain_processes_same_stem_json_and_inflight_only_once`) uses identical content for both files, masking this issue — it only validates the crash-recovery scenario, not the concurrent-enqueue scenario.

While the window requires same timestamp + same sanitized ID (making it narrow), the consequence is silent, undetectable data loss. The fix proposals (content/inode comparison before deletion, checking `.inflight` during enqueue collision detection) are sound.

## Amendment: AMQ-STRAY-FILE-003

### Position
ACCEPT

### Rationale
This is a duplicate of STRAY-IMPL-NOTES-001 — both identify the same file (`20260309T190541Z-impl-notes.md`) and propose the same action (delete from repo). As verified above, the file exists, is tracked, was committed in loop 12, and contains only ephemeral implementation notes. Valid cleanup.
