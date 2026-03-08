---
artifact: quick-dev-final-review
loop: 15
project: issue-178
backend: codex
role: reviewer
created_at: 2026-03-08T06:45:01Z
---

# Final Review: AMENDMENTS

## Amendment: [P1] Persist rollback ceiling before fallible cleanup steps

### Problem
In hard rollback, destructive git actions happen before several fallible cleanup steps, but `.rollback-ceiling` is only written at the end.  
If push fails and any later step errors (`remove_dir_all`, config load, etc.), the function exits before writing the marker, so reconstruction can resurrect stale checkpoint state.

Key paths:
- hard reset/push path in [`rollback.rs` lines ~158-182](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:158)
- fallible cleanup before marker write in [`rollback.rs` lines ~198-233](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:198)
- marker handling only at end in [`rollback.rs` lines ~249-282](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:249)

### Proposed Change
Write/retain `.rollback-ceiling` immediately after determining rollback mode and push outcome (before later fallible cleanup).  
Then run artifact/session cleanup, collecting errors as warnings or aggregated errors, but do not lose the marker durability guarantee for soft rollback and hard-push-failure cases.

### Affected Files
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - reorder marker persistence relative to fallible cleanup; harden error-path behavior.

## Amendment: [P2] Differentiate “branch missing” from remote query failures

### Problem
`remote_branch_exists_on_remote` treats any non-zero `git ls-remote --exit-code` as `false`, so transport/auth failures are misreported as “branch does not exist locally or on origin.”  
This produces misleading validation errors and hides actionable remote failures.

Key paths:
- helper in [`branch.rs` lines ~79-89](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs:79)
- error messages in [`rollback.rs` lines ~89-92](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:89) and [`rollback.rs` lines ~140-143](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs:140)

### Proposed Change
Make remote branch probing tri-state:
- `exists`
- `missing`
- `query_failed` (with stderr context)

Return an error on query failure instead of converting it to “missing branch.”  
Add a validate case with an intentionally broken/unreachable `origin` URL to assert correct error surfacing and non-destructive behavior.

### Affected Files
- [`src/git/branch.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/git/branch.rs) - return richer remote branch probe result.
- [`src/cli/rollback.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/cli/rollback.rs) - handle probe failures distinctly from true missing branches.
- [`src/validate/tests_commands.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-178/src/validate/tests_commands.rs) - add conformance coverage for remote query failure path.

---
