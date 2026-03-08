---
artifact: quick-dev-final-review
loop: 15
project: issue-184
backend: codex
role: reviewer
created_at: 2026-03-08T14:50:47Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] DAEMON-ARTIFACT-WATCHER-TEST-REGRESSION

### Problem
`runtime_artifact_comments_posted` no longer validates what its name and docstring claim.  
The test still says it verifies quick-prd and final-prompt comments ([tests_daemon.rs:1827](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:1827)), but current assertions only check that dispatch text appeared in stderr ([tests_daemon.rs:1951](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:1951)-[1959](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:1959)).  
This is a false-positive risk for artifact watcher regressions.

### Proposed Change
Restore assertions on actual posted comment content and idempotency markers for both phases (`quick-prd`, `final-prompt`) instead of dispatch-only checks. Keep it in-process, but make the test produce deterministic artifacts and verify comment payloads/markers directly.

### Affected Files
- [src/validate/tests_daemon.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs) - strengthen `runtime_artifact_comments_posted` assertions to validate watcher behavior, not just task dispatch.

## Amendment: [P2] RESUME-DISPATCH-TEST-NO-LONGER-PROVES-RESUME-PATH

### Problem
`dispatch_resume_uses_issue_project_prompt_file` currently asserts only `event=project_resume` in stderr ([tests_daemon.rs:2567](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:2567), [2628](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:2628)-[2632](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs:2632)).  
That does not prove it used the `run --project issue-<n>` resume path nor that it selected the issue project prompt file.

### Proposed Change
Add explicit assertions for resume dispatch semantics:
1. Assert the dispatch log shows resume via `run --project issue-501`.
2. Assert fresh-path indicators (`--project-id`/auto path) are absent.
3. Assert the resumed issue project prompt path is the one consumed in the worktree (or equivalent state evidence).

### Affected Files
- [src/validate/tests_daemon.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_daemon.rs) - make resume test prove project-id/prompt-file selection, not just generic resume logging.

## Amendment: [P1] PR-RUNTIME CONFORMANCE COVERAGE WAS REMOVED/WEAKENED

### Problem
Two critical PR-flow conformance checks were effectively dropped:
1. `e2e_draft_create_via_binary` was removed with no equivalent end-to-end replacement ([tests_pr_runtime.rs:29](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs:29)-[33](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs:33), [340](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs:340)-[344](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs:344)).
2. `pr_metadata_verification` now only checks dispatch + terminal label ([tests_e2e_conformance.rs:392](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:392)-[409](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:409)); prior argument/body parsing helpers are now dead-code suppressed ([tests_e2e_conformance.rs:583](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:583)-[603](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs:603)).

This leaves PR metadata and draft lifecycle behavior under-tested in conformance.

### Proposed Change
Reintroduce end-to-end PR-flow assertions for the in-process architecture:
1. Add an in-process E2E test that verifies actual `gh pr create` args/body fields (`--title`, `--head`, `--repo`, `--body-file`, issue closure/footer content).
2. Add/restore lifecycle ordering coverage (`create` before `ready`) under deterministic watcher timing.
3. Remove `#[allow(dead_code)]` from helper parsers once assertions consume them again.

### Affected Files
- [src/validate/tests_pr_runtime.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_pr_runtime.rs) - restore E2E draft lifecycle conformance coverage.
- [src/validate/tests_e2e_conformance.rs](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-184/src/validate/tests_e2e_conformance.rs) - restore PR metadata assertions and remove dead assertion helpers.

---
