use std::process::Command;

use super::panic_message;
use crate::validate::assertions::{assert_exit_code, assert_file_exists, assert_path_not_exists};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    quick_dev_implementer_with_stray_files_script, quick_dev_reviewer_mock_script,
    quick_dev_reviewer_reject_once_script, standard_mock_with_stray_files_script,
};
use crate::validate::runner::{ConformanceTest, TestResult};

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "stray_cleanup::quick_dev_cleanup",
            func: quick_dev_cleanup,
        },
        ConformanceTest {
            name: "stray_cleanup::user_files_preserved",
            func: user_files_preserved,
        },
        ConformanceTest {
            name: "stray_cleanup::multi_iteration_cleanup",
            func: multi_iteration_cleanup,
        },
        ConformanceTest {
            name: "stray_cleanup::regular_implementing_to_reviewing",
            func: regular_implementing_to_reviewing,
        },
        ConformanceTest {
            name: "stray_cleanup::regular_user_files_preserved",
            func: regular_user_files_preserved,
        },
    ]
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

fn setup_quick_dev(h: &RalphHarness, project_id: &str, impl_script: &str, rev_script: &str) {
    h.init_workspace().expect("init failed");

    let impl_path = h
        .write_mock_script("qd-implementer.sh", impl_script)
        .expect("write implementer mock");
    let rev_path = h
        .write_mock_script("qd-reviewer.sh", rev_script)
        .expect("write reviewer mock");

    // Create /bin/sh wrappers for Nix compatibility
    let impl_wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", impl_path.display());
    let rev_wrapper_content = format!("#!/bin/sh\nexec bash \"{}\"\n", rev_path.display());
    let impl_wrapper = h
        .write_mock_script("qd-impl-wrapper.sh", &impl_wrapper_content)
        .expect("write impl wrapper");
    let rev_wrapper = h
        .write_mock_script("qd-rev-wrapper.sh", &rev_wrapper_content)
        .expect("write rev wrapper");

    let impl_wrapper_str = impl_wrapper.to_string_lossy().into_owned();
    let rev_wrapper_str = rev_wrapper.to_string_lossy().into_owned();

    for (backend, wrapper) in &[("claude", &impl_wrapper_str), ("codex", &rev_wrapper_str)] {
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.command"),
            wrapper.to_string(),
            "--global".to_owned(),
        ])
        .unwrap_or_else(|e| panic!("set {backend} command failed: {e}"));
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            format!("backends.{backend}.args"),
            "[]".to_owned(),
            "--global".to_owned(),
        ])
        .unwrap_or_else(|e| panic!("set {backend} args failed: {e}"));
    }

    // Disable openrouter
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.openrouter.enabled".to_owned(),
        "false".to_owned(),
        "--global".to_owned(),
    ])
    .expect("disable openrouter");

    h.create_project(
        project_id,
        "Stray Cleanup Test Project",
        "Test prompt for stray cleanup",
    )
    .expect("create_project failed");
}

fn setup_regular_with_stray_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("stray-mock.sh", &standard_mock_with_stray_files_script())
        .expect("failed to write stray mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
    h.create_project(
        project_id,
        "Stray Cleanup Regular Project",
        "Test prompt for regular orchestrator stray cleanup",
    )
    .expect("create_project failed");
}

fn git_output(repo: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command should execute");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

// ---------------------------------------------------------------------------
// Quick-dev test cases
// ---------------------------------------------------------------------------

/// Happy path: implementer creates stray impl-notes and impl-response files
/// at the worktree root. After the PlanAndImplement → CodexReview transition
/// the stray files should be absent from the worktree.
fn quick_dev_cleanup(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "sc-qd-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_with_stray_files_script(),
            &quick_dev_reviewer_mock_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);

        // Stray files should have been cleaned up
        let repo = &h.repo_root;
        assert_path_not_exists(&repo.join("20260304120000-impl-notes.md"));
        assert_path_not_exists(&repo.join("20260304120000-impl-response-001.md"));

        // The legitimate implementation file should still exist
        assert_file_exists(&repo.join("mock_file.txt"));
    })
}

/// Implementer creates both stray artifacts and a non-matching `impl-notes.md`
/// (no timestamp) at the worktree root. Verify only the timestamped artifacts
/// are removed; user files survive.
fn user_files_preserved(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "sc-user-001";

        // Create a modified implementer script that also writes user-like files
        let impl_script = r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "quick-dev plan-and-implement phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created quick-dev mock implementation.

## Spec Deviations
- None

## Testing
- Mock script only
EOF
  echo "quick-dev-implemented" > mock_file.txt
  git add mock_file.txt
  # Stray artifact (should be removed)
  echo "stray" > 20260304120000-impl-notes.md
  # User file without timestamp (should survive)
  echo "user notes" > impl-notes.md
  # User file with different name (should survive)
  echo "my notes" > my-notes.md
elif grep -q "quick-dev apply-fixes phase" <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Response (Iteration 1)

## Changes Made
1. Applied reviewer-requested fixes.

## Could Not Address
- None
EOF
  echo "quick-dev-fixed" >> mock_file.txt
  git add mock_file.txt
elif grep -q "final reviewer auditing" <<< "$INPUT"; then
  result="${QUICK_DEV_FINAL_REVIEW_RESULT:-NO_AMENDMENTS}"
  if [ "$result" = "AMENDMENTS" ]; then
    cat <<'EOF'
# Final Review: AMENDMENTS

## Issues
- Mock issue.
EOF
  else
    cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
All requirements met.
EOF
  fi
else
  echo "quick-dev-implementer: unrecognized prompt" >&2
  exit 1
fi
"###;

        setup_quick_dev(
            h,
            project_id,
            impl_script,
            &quick_dev_reviewer_mock_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);

        let repo = &h.repo_root;
        // Stray artifact should be gone
        assert_path_not_exists(&repo.join("20260304120000-impl-notes.md"));
        // User files should survive
        assert_file_exists(&repo.join("impl-notes.md"));
        assert_file_exists(&repo.join("my-notes.md"));
        assert_file_exists(&repo.join("mock_file.txt"));
    })
}

/// Two iterations: reviewer rejects once, implementer applies fixes (creating
/// new stray files each iteration). Verify cleanup occurs at each transition.
fn multi_iteration_cleanup(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "sc-multi-001";
        setup_quick_dev(
            h,
            project_id,
            &quick_dev_implementer_with_stray_files_script(),
            &quick_dev_reviewer_reject_once_script(),
        );

        let output = h
            .ralph([
                "quick-dev-run",
                "--project",
                project_id,
                "--implementer-backend",
                "claude",
                "--reviewer-backend",
                "codex",
                "--skip-commit",
            ])
            .expect("quick-dev-run should execute");

        assert_exit_code(&output, 0);

        let repo = &h.repo_root;
        // All stray files from both iterations should be gone
        assert_path_not_exists(&repo.join("20260304120000-impl-notes.md"));
        assert_path_not_exists(&repo.join("20260304120000-impl-response-001.md"));
        assert_path_not_exists(&repo.join("20260304130000-impl-notes.md"));
        assert_path_not_exists(&repo.join("20260304130000-impl-response-002.md"));

        // The legitimate implementation file should still exist
        assert_file_exists(&repo.join("mock_file.txt"));

        // Verify stray files are not in the git log either
        let log = git_output(repo, &["log", "--all", "--name-only", "--pretty=format:"]);
        assert!(
            !log.contains("20260304120000-impl-notes.md"),
            "stray notes should not appear in any commit"
        );
        assert!(
            !log.contains("20260304120000-impl-response-001.md"),
            "stray response should not appear in any commit"
        );
    })
}

// ---------------------------------------------------------------------------
// Regular orchestrator test cases
// ---------------------------------------------------------------------------

/// Regular orchestrator: implementer creates stray impl-notes and
/// impl-response files at the worktree root.  After the implementing→reviewing
/// transition the stray files should be absent from the worktree and the
/// committed tree.
fn regular_implementing_to_reviewing(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "sc-reg-001";
        setup_regular_with_stray_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let repo = &h.repo_root;
        // Stray files should have been cleaned up
        assert_path_not_exists(&repo.join("20260304120000-impl-notes.md"));
        assert_path_not_exists(&repo.join("20260304120000-impl-response-001.md"));

        // The legitimate implementation file should still exist
        assert_file_exists(&repo.join("mock_file.txt"));

        // Verify stray files never appear in any commit
        let log = git_output(repo, &["log", "--all", "--name-only", "--pretty=format:"]);
        assert!(
            !log.contains("20260304120000-impl-notes.md"),
            "stray notes should not appear in any commit"
        );
        assert!(
            !log.contains("20260304120000-impl-response-001.md"),
            "stray response should not appear in any commit"
        );
    })
}

/// Regular orchestrator: implementer creates stray artifacts alongside
/// non-matching user files (`impl-notes.md` without timestamp, `my-notes.md`).
/// Verify only the timestamped artifacts are removed; user files survive.
fn regular_user_files_preserved(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "sc-reg-002";

        // Use a custom mock that also writes user-like files
        let script_content = r###"#!/usr/bin/env bash
set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a software architect planning features for a project." <<< "$INPUT"; then
  cat <<'EOF'
# Feature: Demo Feature

## Description
Mock feature used by validate tests.

## Acceptance Criteria
- [ ] Mock implementation file is created

## Files to Modify/Create
- `mock_file.txt` - file created by the mock implementer

## Dependencies
- Requires: none
- Blocks: none
EOF
elif grep -q "You are a software developer implementing a feature specification." <<< "$INPUT"; then
  cat <<'EOF'
# Implementation Notes

## Decisions Made
- Created a mock implementation artifact.

## Spec Deviations
- None

## Testing
- Mock script execution only
EOF
  echo "implemented" > mock_file.txt
  git add mock_file.txt
  # Stray artifact (should be removed)
  echo "stray notes" > 20260304120000-impl-notes.md
  # User files (should survive)
  echo "user notes" > impl-notes.md
  echo "my notes" > my-notes.md
elif grep -q "You are a code reviewer ensuring implementations match specifications." <<< "$INPUT"; then
  cat <<'EOF'
# Review: APPROVED

## Acceptance Criteria Checklist
- [x] Mock implementation file is created

## Notes
Looks good.

## Commit Message
feat: apply mock implementation
EOF
elif grep -q "You are a project completion validator." <<< "$INPUT"; then
  cat <<'EOF'
# Verdict: CONTINUE

## Missing Requirements
1. Additional feature remains.

## Recommended Next Features
1. Implement another mock feature.
EOF
elif grep -q "You are a final reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Final Review: NO AMENDMENTS

## Summary
The project is complete and requires no further amendments.
EOF
elif grep -q "You are a prompt reviewer" <<< "$INPUT"; then
  cat <<'EOF'
# Prompt Review

## Issues Found
- Mock issue for testing

## Refined Prompt
This is the refined prompt from the mock reviewer.
EOF
elif grep -q "You are a QA engineer" <<< "$INPUT"; then
  cat <<'EOF'
# QA: PASS

## Manual Testing
- mock manual check: passed

## Automated Tests
- mock test suite: passed

## Acceptance Criteria Verification
All acceptance criteria verified by mock QA.
EOF
else
  echo "unrecognized prompt" >&2
  exit 1
fi
"###;

        h.init_workspace().expect("init failed");
        let script = h
            .write_mock_script("stray-user-mock.sh", script_content)
            .expect("failed to write mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Stray Cleanup User Files Project",
            "Test prompt for regular orchestrator stray cleanup with user files",
        )
        .expect("create_project failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let repo = &h.repo_root;
        // Stray artifact should be gone
        assert_path_not_exists(&repo.join("20260304120000-impl-notes.md"));
        // User files should survive
        assert_file_exists(&repo.join("impl-notes.md"));
        assert_file_exists(&repo.join("my-notes.md"));
        assert_file_exists(&repo.join("mock_file.txt"));
    })
}
