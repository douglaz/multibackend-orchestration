# Add QA Conformance Tests to Validate Suite

## Goal

Add black-box conformance tests to `src/validate/` that exercise the QA phase feature through the ralph binary. Currently the 40 conformance tests have zero QA coverage — all QA testing is done via unit/integration tests in `tests/`.

## Background

The validate suite tests ralph as a black box: it runs the compiled binary, inspects state.json/artifacts/git state, and asserts correctness. Tests use `RalphHarness` (in `src/validate/harness.rs`) which provides an isolated temp workspace per test with mock backends.

Mock scripts detect which role is being invoked by grepping the prompt for role-identifying phrases:
- Planner: `"You are a software architect planning features for a project."`
- Implementer: `"You are a software developer implementing a feature specification."`
- Reviewer: `"You are a code reviewer ensuring implementations match specifications."`
- Completer: `"You are a project completion validator."`
- **QA: `"You are a QA engineer validating an implementation against its specification."`**
- **Acceptance QA: `"You are a QA engineer validating overall project acceptance."`**

QA output must follow the parser contract:
- Pass: H1 `# QA: PASS`, required H2s `## Tests Run`, `## Verification Summary`
- Fail: H1 `# QA: FAIL`, required H2s `## Failures`, `## Suggested Fixes`

## Config Keys

- `workflow.qa_enabled` (bool, default: false)
- `workflow.qa_backend` (optional string)
- `workflow.max_qa_iterations` (u32, default: 3)

## Tests to Add

All tests go in a new file `src/validate/tests_qa.rs`. Register them in `src/validate/mod.rs` alongside the existing test modules.

### 1. `qa::disabled_skips_phase`

**Setup:** Standard mock, `workflow.qa_enabled = false` (default).
**Action:** `ralph run --loops 1`
**Assert:**
- Exit 0, loop completes
- `state.json` loop has empty `qa_results` array
- No QA artifacts in the loop directory (no files matching `*qa*`)
- Loop completed and committed normally

### 2. `qa::enabled_pass_proceeds_to_review`

**Setup:** Standard mock + QA mock that returns `# QA: PASS` with required sections. Set `workflow.qa_enabled = true`.
**Action:** `ralph run --loops 1`
**Assert:**
- Exit 0, loop completes
- `state.json` loop has `qa_results` array with 1 entry: `passed: true`
- QA pass artifact exists in loop directory
- Loop completed and committed normally
- `backends.qa` field is populated in loop state

### 3. `qa::fail_retries_then_passes`

**Setup:** QA mock that fails on first call, passes on second (use a counter file). Set `workflow.qa_enabled = true`.
**Action:** `ralph run --loops 1`
**Assert:**
- Exit 0, loop completes
- `state.json` loop has `qa_results` array with 2 entries: first `passed: false`, second `passed: true`
- Both QA artifacts exist (fail + pass)
- Implementer QA response artifact exists (the implementer's fix response)
- Loop completed and committed

### 4. `qa::iteration_limit_rolls_back`

**Setup:** QA mock that always returns `# QA: FAIL`. Set `workflow.qa_enabled = true`, `workflow.max_qa_iterations = 1`.
**Action:** `ralph run --loops 1`
**Assert:**
- Exit code is non-zero (QaIterationLimitExceeded)
- `state.json` has no completed loops (loop was rolled back)
- No loop artifacts remain (rollback cleaned up)
- No git tag for loop 1

### 5. `qa::acceptance_gate_pass`

**Setup:** Mock that returns COMPLETE on completion check. QA/acceptance mock returns PASS. Set `workflow.qa_enabled = true`. Use `RALPH_COMPLETE=yes` env var.
**Action:** `ralph run` with RALPH_COMPLETE env
**Assert:**
- Exit 0
- `state.json` status is `"completed"`
- Acceptance pass artifact exists in completion loop directory
- Completion attempt has `acceptance_passed: true`

### 6. `qa::acceptance_gate_fail_forces_continue`

**Setup:** Mock where completer returns COMPLETE but acceptance QA returns FAIL. Then on the forced-continue second planning loop, planner returns another feature (and eventually completes on a second completion attempt with acceptance PASS). Set `workflow.qa_enabled = true`.
**Action:** `ralph run --until-complete`
**Assert:**
- Exit 0 eventually (project completes on retry)
- First completion attempt has `acceptance_passed: false`
- Acceptance fail artifact exists
- At least 2 loops were executed (the forced continue caused another feature loop)
- Final status is `"completed"`

### 7. `qa::config_get_set`

**Setup:** Init workspace only.
**Action:** Test config get/set for QA keys:
- `ralph config get workflow.qa_enabled` → `false`
- `ralph config set workflow.qa_enabled true` → success
- `ralph config get workflow.qa_enabled` → `true`
- `ralph config set workflow.max_qa_iterations 5` → success
- `ralph config get workflow.max_qa_iterations` → `5`
- `ralph config set workflow.qa_backend "claude(opus)"` → success
- `ralph config get workflow.qa_backend` → `claude(opus)`
- `ralph config set qa_backend "codex"` → success (alias test)
- `ralph config get workflow.qa_backend` → `codex`

### 8. `qa::history_verbose_shows_qa`

**Setup:** Run a loop with QA enabled and passing.
**Action:** `ralph history --verbose`
**Assert:**
- Output contains `QA:` line with attempt count and verdict
- Output contains `qa=` in the backends line

### 9. `qa::status_shows_qa_info`

**Setup:** Run a loop with QA enabled and passing, then start (but don't finish) a second loop that's in QA phase.
**Action:** `ralph status`
**Assert:**
- Output contains QA-related information (phase label, QA verdict)

## Mock Script Design

The QA mock needs to handle both feature QA and acceptance QA:

```bash
# Inside the mock script's role detection:
if echo "$INPUT" | grep -q "You are a QA engineer validating an implementation"; then
  # Feature QA
  if echo "$INPUT" | grep -q "overall project acceptance"; then
    # Acceptance QA
    cat <<'ACCEPTANCE'
# QA: PASS

## Tests Run
- acceptance check: passed

## Verification Summary
All project-level acceptance criteria verified.
ACCEPTANCE
  else
    # Feature QA
    cat <<'FEATUREQA'
# QA: PASS

## Tests Run
- cargo check: ok
- cargo test: 10 passed, 0 failed

## Verification Summary
All acceptance criteria from the spec have been verified.
FEATUREQA
  fi
```

For the fail-then-pass test, use a counter file:
```bash
COUNTER_FILE="$HOME/.qa_counter"
COUNT=$(cat "$COUNTER_FILE" 2>/dev/null || echo 0)
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNTER_FILE"
if [ "$COUNT" -le 1 ]; then
  # First call: FAIL
  cat <<'FAIL'
# QA: FAIL

## Failures
1. cargo test failed: 2 tests failing

## Suggested Fixes
1. Fix the failing assertions in test_feature_x
FAIL
else
  # Subsequent calls: PASS
  cat <<'PASS'
# QA: PASS

## Tests Run
- cargo test: all passed

## Verification Summary
All tests passing after fixes.
PASS
fi
```

## Files to Create/Modify

| File | Change |
|------|--------|
| `src/validate/tests_qa.rs` | **New** — all QA conformance tests |
| `src/validate/mod.rs` | Register `tests_qa` module and add tests to runner |

## Implementation Notes

- Follow existing patterns in `tests_run.rs` exactly for test structure
- Each test function signature: `fn(h: &RalphHarness) -> TestResult`
- Use `run_case(|| { ... })` wrapper
- Test names are prefixed with `qa::` (e.g., `qa::disabled_skips_phase`)
- Use `h.setup_separate_mock_backends()` when QA needs different behavior from other roles
- The mock script must handle ALL roles (planner/implementer/reviewer/completer/qa) — not just QA
- Tests 6 (acceptance gate fail) is the most complex — it needs careful mock script design to control behavior across multiple loops

## Acceptance Criteria

1. All new tests pass with `nix build`
2. Existing 40 tests still pass unchanged
3. QA config keys are exercised end-to-end
4. QA pass, fail, retry, rollback, and acceptance gate paths are all covered
5. History/status output includes QA information when QA is enabled
