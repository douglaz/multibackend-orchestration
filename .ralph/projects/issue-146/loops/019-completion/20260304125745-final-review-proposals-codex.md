---
artifact: final-review-proposals
loop: 19
project: issue-146
backend: codex
role: final_reviewer
created_at: 2026-03-04T12:57:45Z
---

Reviewing the completed quick-dev implementation now. I’ll start by identifying the correct base branch, run the required `git diff <base>...HEAD -- . ':(exclude).ralph'`, then trace the changed code paths and tests for correctness and safety issues.
I confirmed the branch is `ralph/issue-146` ahead of `master`; next I’m running the exact diff against `master` (excluding `.ralph`) and then I’ll inspect each changed module and tests.
I have the full patch context; now I’m enumerating all changed files and drilling into high-risk areas first (`workflow`, `daemon`, `parser`, `state`, and conformance tests) to catch correctness/safety issues.
I’m now reading the new `quick_dev_orchestrator` end-to-end, since most correctness and crash-safety requirements concentrate there. I’ll verify phase transitions, persisted counters, guard logic, and backend/session isolation behavior.
I’ve validated the core loop and am continuing through helper functions and tests to verify parser contracts, checkpoint behavior, and whether any error paths can lose persisted state.
Next I’m reviewing config/template merge paths and daemon dispatch wiring, because subtle fallback mistakes there can make quick-dev route incorrectly or fail only in resumed runs.
I’m now checking whether final-review calls truly use fresh context in practice (not just sequentially), by tracing backend/registry behavior and session-reuse settings under the hood.
I’m moving through the integration and conformance tests now to ensure they verify the intended behavior rather than passing incidentally, especially around resume-phase assertions and daemon branching.
I’ve identified two concrete robustness bugs and one cleanup issue. I’m taking one more pass to lock exact line references for each amendment before delivering the final review.
# Final Review: AMENDMENTS

## Amendment: FR-QD-STATE-001

### Problem
Quick-dev state detection is too broad and can override non-quick reconstruction paths.

- In [`load_quick_dev_phase_from_state_json`]( /tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:471 ), `is_quick_dev_state` is inferred from presence of quick-dev fields, including counters ([`src/project/lifecycle.rs:475`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:475)-[`477`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:477)).
- That flag then overwrites `current_phase`/`phase_iteration` ([`src/project/lifecycle.rs:487`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:487)-[`493`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:493)) and can force `status=completed` ([`src/project/lifecycle.rs:500`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:500)-[`506`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs:506)).
- The conformance test claiming to protect this path uses an unrealistic fixture with only `status/current_phase` and omits quick-dev fields ([`src/validate/tests_quick_dev.rs:1240`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs:1240)-[`1247`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs:1247)), so it can pass while marker logic is still unsafe.

### Proposed Change
Tighten quick-dev marker detection to actual quick-dev evidence, not mere field presence.

- Treat `state.json` as quick-dev only when at least one strong signal exists:
1. `quick_dev_phase` is `Some`.
2. quick-dev counters are greater than zero.
3. quick-dev artifacts are present (`quick-dev-*` loop/project artifacts).
- Apply `current_phase`/`phase_iteration`/`status` overrides only under that stronger marker.
- Update/add tests to use realistic serialized shape (including zero-valued counters) and assert non-quick projects are not reclassified.

### Affected Files
- [`src/project/lifecycle.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/project/lifecycle.rs) - fix marker logic and quick-dev override gating.
- [`src/validate/tests_quick_dev.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs) - strengthen non-quick reclassification test fixture/assertions.

## Amendment: FR-QD-PREFLIGHT-002

### Problem
`quick-dev-auto` preflight claims fail-fast behavior but does not validate backend availability (enabled/usable), so side effects can occur before failure.

- Preflight currently validates distinctness and spec validity only ([`src/cli/quick_dev_auto.rs:129`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs:129)-[`158`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs:158)).
- `validate_required_backend_spec` does not check disabled backends ([`src/config/mod.rs:534`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/config/mod.rs:534)-[`559`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/config/mod.rs:559), [`566`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/config/mod.rs:566)-[`572`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/config/mod.rs:572)).
- The actual disabled-backend rejection happens later in orchestrator backend creation ([`src/workflow/quick_dev_orchestrator.rs:117`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:117)-[`118`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/workflow/quick_dev_orchestrator.rs:118), [`src/backend/mod.rs:983`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/backend/mod.rs:983)-[`990`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/backend/mod.rs:990)).
- By then `quick-prd` and project creation may already have run ([`src/cli/quick_dev_auto.rs:168`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs:168)-[`229`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs:229)).

### Proposed Change
Make preflight truly fail-fast for quick-dev backends before any quick-prd/project side effects.

- During preflight, verify selected implementer/reviewer are enabled/available (not just syntactically valid).
- Perform backend `health_check` for those two selected quick-dev roles before launching quick-prd.
- Add conformance coverage for disabled reviewer backend to assert failure happens before project creation.

### Affected Files
- [`src/cli/quick_dev_auto.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/cli/quick_dev_auto.rs) - strengthen preflight validation.
- [`src/config/mod.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/config/mod.rs) - add/adjust helper to validate availability (or use registry-based check).
- [`src/validate/tests_quick_dev.rs`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/src/validate/tests_quick_dev.rs) - add disabled-backend fail-fast test.

## Amendment: FR-CLEANUP-003

### Problem
A stray implementation artifact was committed at repo root and is outside product/runtime/test scope.

- [`20260304T103437-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md)

### Proposed Change
Remove the stray root file from the branch.

### Affected Files
- [`20260304T103437-impl-notes.md`](/tmp/ralph-daemon-data/douglaz/multibackend-orchestration/.ralph/daemon/worktrees/douglaz-multibackend-orchestration-146/20260304T103437-impl-notes.md) - delete file.

---
