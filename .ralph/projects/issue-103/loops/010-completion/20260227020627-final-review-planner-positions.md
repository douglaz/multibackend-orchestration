---
artifact: final-review-planner-positions
loop: 10
project: issue-103
backend: codex(gpt-5.3-codex-xhigh)
role: planner
created_at: 2026-02-27T02:06:27Z
---

I’ll verify both amendments directly against the code and tests, starting with `tests_interactive_prd_logging.rs` and the corresponding logging paths in `interactive_prd.rs` to confirm whether each claimed gap is real.
I’ve located the exact test and production paths mentioned. Next I’m opening the surrounding blocks to confirm the current assertions and how log entries are emitted on parse failures vs backend failures.
I confirmed the production logging behavior: retry parse events log `raw_output` with `error = null`, while backend/runtime failures log `raw_output = null`, `error = ...`, `validation = not_checked`. I’m now checking whether conformance tests already assert that error-path schema.
I’ve confirmed the logging test file has no explicit assertions for `raw_output`/`error` in the timing-guarantee case. I’ll quickly inspect `run_review_with_retry` to verify that a backend exit on attempt 2 still emits a labeled fallback log, which would let the current test pass spuriously.
I confirmed there’s no validate/conformance assertion today for the backend-failure log schema (`raw_output: null`, `error: non-null`) in interactive PRD logging. I’m doing one more pass over in-file unit tests to ensure this isn’t already covered elsewhere before finalizing positions.
I’ve verified both amendments against the source. I’m now preparing the final positions with file/line citations showing whether each claimed gap is real.
# Planner Positions

## Amendment: FR-LOG-001

### Position
ACCEPT

### Rationale
The problem is real. The timing-guarantee test currently only asserts presence/count of labels ([`tests_interactive_prd_logging.rs#L548`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L548), [`tests_interactive_prd_logging.rs#L563`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L563)), while the mock backend guard exits non-zero when prior log is missing ([`tests_interactive_prd_logging.rs#L382`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L382)-[`tests_interactive_prd_logging.rs#L384`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L384)).  
On backend error, production still emits a labeled failed-attempt log with `raw_output = None`, `error = Some(...)`, `validation = not_checked` ([`interactive_prd.rs#L2244`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs#L2244)-[`interactive_prd.rs#L2256`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs#L2256)). So label-only assertions can pass for the wrong reason.

## Amendment: FR-LOG-002

### Position
ACCEPT

### Rationale
This coverage gap is real. The current conformance list in this module has no explicit backend transport/runtime failure-schema test ([`tests_interactive_prd_logging.rs#L19`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L19)-[`tests_interactive_prd_logging.rs#L53`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L53)); existing schema assertions cover only parse-failure semantics (`raw_output` present, `error` null, `review_parse_failed`) ([`tests_interactive_prd_logging.rs#L343`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L343)-[`tests_interactive_prd_logging.rs#L353`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/validate\/tests_interactive_prd_logging.rs#L353)).  
Production explicitly defines the failure schema for runtime/transport errors (`raw_output = None`, `error = Some`, `validation = not_checked`) in both generic backend calls and review retry failure paths ([`interactive_prd.rs#L2444`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs#L2444)-[`interactive_prd.rs#L2452`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs#L2452), [`interactive_prd.rs#L2249`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs#L2249)-[`interactive_prd.rs#L2256`](\/tmp\/ralph-daemon-data\/douglaz\/multibackend-orchestration\/.ralph\/daemon\/worktrees\/douglaz-multibackend-orchestration-103\/src\/daemon\/interactive_prd.rs#L2256)). Adding conformance coverage is technically justified.
