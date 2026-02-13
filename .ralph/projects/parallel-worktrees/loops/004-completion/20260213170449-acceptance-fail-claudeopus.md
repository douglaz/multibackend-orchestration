---
artifact: acceptance-fail
loop: 4
project: parallel-worktrees
backend: claude(opus)
role: qa
created_at: 2026-02-13T17:04:49Z
---

# QA: FAIL
## Failures
- Output did not include the required top-level H1 as the first line.
- Required H2 sections were missing.

## Suggested Fixes
- Start the response with exactly one valid H1: `# QA: PASS` or `# QA: FAIL`.
- If using `# QA: PASS`, include:
  - `## Tests Run`
  - `## Verification Summary`
- If using `# QA: FAIL`, include:
  - `## Failures`
  - `## Suggested Fixes`
