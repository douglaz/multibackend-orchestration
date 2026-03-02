use super::*;

use crate::validate::assertions::{assert_exit_code, assert_file_contains, assert_file_exists};
use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "prd::preset_default_is_full_with_ask_max_3",
            func: preset_default_is_full_with_ask_max_3,
        },
        ConformanceTest {
            name: "prd::preset_overrides_default_ask_max",
            func: preset_overrides_default_ask_max,
        },
        ConformanceTest {
            name: "prd::preset_fast_has_zero_ask_max",
            func: preset_fast_has_zero_ask_max,
        },
        ConformanceTest {
            name: "prd::explicit_ask_max_preempts_preset",
            func: explicit_ask_max_preempts_preset,
        },
    ]
}

fn preset_default_is_full_with_ask_max_3(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph(["prd", "--idea", "Add one-click onboarding"])
            .expect("prd default preset command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: full"),
            "expected full preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 3"),
            "expected default ask max 3"
        );

        let prd = h.repo_root.join("PRD.md");
        assert_file_exists(&prd);
        assert_file_contains(&prd, "## Executive Summary");
    })
}

fn preset_overrides_default_ask_max(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph(["prd", "--idea", "Add quick notes", "--preset", "discuss"])
            .expect("prd discuss preset command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: discuss"),
            "expected discuss preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 1"),
            "expected discuss preset to resolve to ask max 1"
        );
    })
}

fn preset_fast_has_zero_ask_max(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph(["prd", "--idea", "Add quick exports", "--preset", "fast"])
            .expect("prd fast preset command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: fast"),
            "expected fast preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 0"),
            "expected fast preset to resolve to ask max 0"
        );
    })
}

fn explicit_ask_max_preempts_preset(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_prd_mock(h);
        let output = h
            .ralph([
                "prd",
                "--idea",
                "Add bulk edits",
                "--preset",
                "fast",
                "--ask-max",
                "2",
            ])
            .expect("prd preset+ask-max command should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ask preset: fast"),
            "expected fast preset output"
        );
        assert!(
            stdout.contains("ask max rounds: 2"),
            "expected explicit ask max to override preset"
        );
    })
}

fn setup_prd_mock(h: &RalphHarness) {
    h.init_workspace().expect("init_workspace failed");
    let script = h
        .write_mock_script("prd-mock.sh", &prd_mock_script())
        .expect("failed to write PRD mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends failed");
}

fn prd_mock_script() -> String {
    r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a product ideation specialist" <<< "$INPUT"; then
  cat <<'EOF'
## Core Concept
Clear idea framing for the proposed product.

## Target Users
- Internal team

## Key Problems Solved
- Reduced manual burden

## Proposed Features
- A focused set of starter features

## Success Metrics
- Faster task completion

## Constraints & Assumptions
- Basic implementation assumptions
EOF
elif grep -q "You are a technical research analyst" <<< "$INPUT"; then
  cat <<'EOF'
## Market Context
- Market overview

## Technical Landscape
- Existing approaches

## Comparable Solutions
- Baseline alternatives

## Technical Feasibility
- Feasible with current stack

## Risk Assessment
- Low risk scope
EOF
elif grep -q "You are a product strategist" <<< "$INPUT"; then
  cat <<'EOF'
## Product Vision
- Vision summary

## User Stories
- User story example

## Feature Prioritization
- P0: core features

## Architecture Overview
- High-level architecture notes

## MVP Scope
- Core scope only

## Open Questions
- None
EOF
elif grep -q "You are a technical product manager" <<< "$INPUT"; then
  cat <<'EOF'
## Executive Summary
- Summary

## Goals & Non-Goals
- Goals and boundaries

## User Stories
- User story

## Functional Requirements
- Feature requirements

## Non-Functional Requirements
- Performance and reliability

## Technical Architecture
- Architecture outline

## Data Model
- Data structures

## API Design
- API boundaries

## Security Considerations
- Secure auth

## Testing Strategy
- Unit and integration tests

## Rollout Plan
- Phased rollout

## Success Metrics
- Uptake and retention

## Open Questions
- None
EOF
elif grep -q "You are a requirements analyst" <<< "$INPUT"; then
  cat <<'EOF'
```json
{
  "missing_fields": [],
  "ambiguities": [],
  "questions": [],
  "suggested_defaults": []
}
```
EOF
elif grep -q "You are a PRD reviewer." <<< "$INPUT"; then
  cat <<'EOF'
```json
{
  "valid": true,
  "issues": []
}
```
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###
    .to_owned()
}

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
