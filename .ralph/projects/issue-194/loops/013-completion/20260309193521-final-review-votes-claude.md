---
artifact: final-review-votes
loop: 13
project: issue-194
backend: claude
role: final_reviewer
created_at: 2026-03-09T19:35:21Z
---

# Vote Results

## Amendment: STRAY-IMPL-NOTES-001

### Vote
ACCEPT

### Rationale
Confirmed: the file is tracked (`git ls-files --error-unmatch` succeeds), contains only ephemeral loop-12 implementation notes, and has no place in the shipped branch. This is a trivial, risk-free cleanup — `git rm` and done.

## Amendment: AMQ-UTF8-READ-001

### Vote
ACCEPT

### Rationale
The defect is real and the fix is clean. At line 492, `fs::read_to_string` maps invalid UTF-8 to `io::Error(ErrorKind::InvalidData)`, which falls into `ReadFailed` at line 494 and triggers `rollback_mid_drain` at line 246-248. The code's own documentation (lines 481, 485-488) explicitly distinguishes content failures (quarantine) from I/O failures (fatal), yet the implementation violates that contract for this specific case. Switching to `fs::read` + `serde_json::from_slice` is a minimal, correct fix that routes non-UTF-8 files through the `Malformed`→quarantine path where they belong. A single corrupt or binary file in the queue should not abort the entire drain and re-enqueue all previously processed amendments.

## Amendment: AMQ-STEM-RACE-002

### Vote
ACCEPT

### Rationale
The planner's step-by-step trace of the race is accurate. The critical issue is at lines 197-208: the drain deletes *any* `.json` whose stem matches a completed `.inflight`, regardless of whether that `.json` is the original crash-recovery leftover or a freshly enqueued file with different content. The enqueue collision check (lines 147-152) only tests `.json` existence, not `.inflight`, so the window is open. While the race requires same timestamp + same sanitized ID (making it narrow), the consequence — silent, undetectable loss of a legitimately enqueued amendment — is severe enough to warrant fixing. The existing test at line 750 uses identical content for both files, so it only validates crash recovery, not the concurrent-enqueue scenario. Comparing content or inodes before deletion is a sound approach.

## Amendment: AMQ-STRAY-FILE-003

### Vote
ACCEPT

### Rationale
Duplicate of STRAY-IMPL-NOTES-001. Both identify the same file (`20260309T190541Z-impl-notes.md`) and propose the same action. The file is confirmed tracked and non-source. Accepting both is harmless — they collapse to a single `git rm` during implementation.
