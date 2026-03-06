use super::*;

use std::fs;
use std::path::PathBuf;

use crate::validate::assertions::{assert_exit_code, assert_file_exists, assert_stdout_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::prd_mock_response_body;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "quick_prd::non_interactive_happy_path",
            func: non_interactive_happy_path,
        },
        ConformanceTest {
            name: "quick_prd::dry_run_no_artifact",
            func: dry_run_no_artifact,
        },
        ConformanceTest {
            name: "quick_prd::backend_override_proof",
            func: backend_override_proof,
        },
    ]
}

fn non_interactive_happy_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_quick_prd_mock(h);

        let output = h
            .ralph(["quick-prd", "--idea", "test idea", "--non-interactive"])
            .expect("quick-prd --non-interactive should execute");
        assert_exit_code(&output, 0);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let spec_path = PathBuf::from(stdout.trim());
        assert!(
            !stdout.trim().is_empty(),
            "expected non-interactive quick-prd to print spec path"
        );
        assert_file_exists(&spec_path);
        assert!(
            spec_path.starts_with(h.repo_root.join(".ralph").join("quick-prd")),
            "expected spec artifact in workspace quick-prd directory, got: {}",
            spec_path.display()
        );
    })
}

fn dry_run_no_artifact(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        let before_paths = quick_prd_spec_paths(h);
        let output = h
            .ralph(["quick-prd", "--idea", "test idea", "--dry-run"])
            .expect("quick-prd --dry-run should execute");
        assert_exit_code(&output, 0);
        assert_stdout_contains(&output, "test idea");

        let after_paths = quick_prd_spec_paths(h);
        assert_eq!(
            before_paths.len(),
            after_paths.len(),
            "quick-prd --dry-run should not write a spec artifact"
        );
    })
}

fn backend_override_proof(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_quick_prd_mock(h);

        // Poison default codex backend path so default quick-prd routing fails.
        let poisoned_path = h.temp_dir.path().join("missing-codex-binary");
        assert!(!poisoned_path.exists(), "poisoned binary path must not exist");
        let poisoned = poisoned_path.to_string_lossy().into_owned();
        h.ralph_ok(vec![
            "config".to_owned(),
            "set".to_owned(),
            "backends.codex.command".to_owned(),
            poisoned,
        ])
        .expect("set poisoned codex backend command");

        let default_output = h
            .ralph(["quick-prd", "--idea", "test idea", "--non-interactive"])
            .expect("quick-prd with default backends should execute");
        assert!(
            !default_output.status.success(),
            "expected quick-prd with poisoned default codex backend to fail"
        );
        let default_stderr = String::from_utf8_lossy(&default_output.stderr).to_lowercase();
        assert!(
            default_stderr.contains("codex") || default_stderr.contains("unavailable"),
            "expected poisoned default path to fail with codex/unavailable signal, got:\n{default_stderr}"
        );

        let override_output = h
            .ralph([
                "quick-prd",
                "--idea",
                "test idea",
                "--writer-backend",
                "claude",
                "--reviewer-backend",
                "claude",
                "--non-interactive",
            ])
            .expect("quick-prd with backend overrides should execute");
        assert_exit_code(&override_output, 0);

        let spec_path = PathBuf::from(String::from_utf8_lossy(&override_output.stdout).trim());
        assert_file_exists(&spec_path);
    })
}

fn quick_prd_spec_paths(h: &RalphHarness) -> Vec<PathBuf> {
    let root = h.repo_root.join(".ralph").join("quick-prd");
    let mut specs = Vec::new();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return specs,
        Err(err) => panic!("failed to read {}: {err}", root.display()),
    };

    for entry in entries {
        let entry = entry.expect("read quick-prd entry");
        let ty = entry.file_type().expect("quick-prd entry file_type");
        if !ty.is_dir() {
            continue;
        }
        let spec = entry.path().join("SPEC.md");
        if spec.is_file() {
            specs.push(spec);
        }
    }

    specs.sort();
    specs
}

fn setup_quick_prd_mock(h: &RalphHarness) {
    h.init_workspace().expect("init failed");

    let script = h
        .write_stable_mock_script("quick-prd-mock.sh", &quick_prd_mock_script())
        .expect("failed to write quick-prd mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
}

fn quick_prd_mock_script() -> String {
    let response_body = prd_mock_response_body();
    format!(
        r###"set -euo pipefail

INPUT="$(cat)"

if grep -q "You are a senior software engineer writing a focused engineering specification." <<< "$INPUT"; then
  cat <<'EOF'
## Summary
- Quick PRD mock summary for conformance coverage.

## Acceptance Criteria
- [ ] Mock acceptance criteria is recorded.

## Technical Approach
- Reuse existing quick-prd pipeline behavior.

## Files & Modules
- src/validate/tests_quick_prd.rs

## Testing Strategy
- Validate command behavior with mock backends.

## Out of Scope
- Production code changes.
EOF
elif grep -q "You are a senior engineer reviewing an engineering specification for completeness and feasibility." <<< "$INPUT"; then
  cat <<'EOF'
```json
{{"approved": true, "issues": []}}
```
EOF
else
{response_body}
fi
"###
    )
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
