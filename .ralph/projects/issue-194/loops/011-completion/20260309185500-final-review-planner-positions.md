---
artifact: final-review-planner-positions
loop: 11
project: issue-194
backend: claude(opus)
role: planner
created_at: 2026-03-09T18:55:00Z
---

Now I have all the information I need. Let me verify one more detail about the stray file names vs the timestamp parser.

The stray files are named `20260309T165947Z-impl-notes.md`. The `parse_artifact_filename_timestamp` function splits on the first `-` and checks that the prefix is exactly 14 ASCII digits. The prefix `20260309T165947Z` is 16 characters and contains non-digit chars (`T`, `Z`), so the function returns `None` and `is_stray_impl_artifact` returns `false` — confirming FR-20260309-002's cleanup gap claim.

# Planner Positions

## Amendment: STRAY-ROOT-FILES-001

### Position
ACCEPT

### Rationale
Both files exist at the repository root (confirmed via `ls`). They are internal implementation artifacts (`impl-notes.md`, `impl-response-001.md`) and are not part of the deliverable source. The cleanup function `remove_stray_impl_artifacts` in `src/git/commit.rs:277-293` relies on `parse_artifact_filename_timestamp` (`src/project/artifacts.rs:253-259`), which requires a 14-digit all-numeric prefix. The stray files use ISO-basic format (`20260309T165947Z`), which is 16 characters and contains non-digit characters, so they are not detected. Removing them is straightforward and correct.

## Amendment: FR-20260309-001

### Position
ACCEPT

### Rationale
Verified in `src/project/amendments.rs`. The function `parse_inflight_request` (lines 477-481) calls `fs::read_to_string(path)?` followed by `serde_json::from_str(&content)?` and `req.validate()?`. All three error types are propagated as a single `Result::Err`. In the drain loop (lines 229-245), **every** `Err` from `parse_inflight_request` triggers quarantining and `continue` — there is no distinction between I/O errors and parse/validation errors. A transient I/O failure (e.g., `PermissionDenied`) on a perfectly valid amendment file would silently quarantine it, effectively losing the amendment without returning an error to the caller. This is a real correctness defect: I/O read failures should be treated as fatal drain errors (triggering the existing `rollback_mid_drain` path), not as malformed content.

## Amendment: FR-20260309-002

### Position
ACCEPT

### Rationale
This amendment correctly identifies two issues:

1. **Stray files exist** — confirmed both `20260309T165947Z-impl-notes.md` and `20260309T165947Z-impl-response-001.md` are present at the repo root.

2. **Cleanup gap is real** — `parse_artifact_filename_timestamp` in `src/project/artifacts.rs:253-259` splits on the first `-`, then checks `prefix.len() == 14 && prefix.chars().all(|c| c.is_ascii_digit())`. The ISO-basic timestamp prefix `20260309T165947Z` is 16 chars with non-digit characters, so it fails both checks. This means `is_stray_impl_artifact` (`src/git/commit.rs:277-293`) will never match files using this timestamp format, and `remove_stray_impl_artifacts` will skip them. Hardening the detection to also match ISO-basic variants (`YYYYMMDDTHHMMSSz-*`) is a valid robustness improvement that prevents recurrence.

Note: This amendment overlaps with STRAY-ROOT-FILES-001 on file deletion but additionally proposes hardening the detection regex, which is a distinct and valuable addition.
