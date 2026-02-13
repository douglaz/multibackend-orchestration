---
artifact: acceptance-fail
loop: 6
project: parallel-worktrees
backend: claude(opus)
role: qa
created_at: 2026-02-13T17:21:45Z
---

# QA: FAIL
## Failures
- Previous output was empty and did not include the required top-level H1.
- Required H2 sections were missing.

## Suggested Fixes
1. Start the response with exactly `# QA: PASS` or `# QA: FAIL` as the first line.
2. If using PASS, include:
   - `## Tests Run`
   - `## Verification Summary`
3. If using FAIL, include:
   - `## Failures`
   - `## Suggested Fixes`
4. Do not include preamble text or YAML frontmatter.
