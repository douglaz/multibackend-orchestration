use super::*;

use crate::validate::assertions::{assert_exit_code, assert_stderr_contains};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::backend_exec_echo_script;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "backend_exec::happy_path_echo",
            func: happy_path_echo,
        },
        ConformanceTest {
            name: "backend_exec::unknown_backend",
            func: unknown_backend,
        },
        ConformanceTest {
            name: "backend_exec::raw_suppresses_metrics",
            func: raw_suppresses_metrics,
        },
        ConformanceTest {
            name: "backend_exec::prompt_from_file",
            func: prompt_from_file,
        },
    ]
}

fn setup_echo_backend(h: &RalphHarness) {
    h.init_workspace().expect("init workspace");
    let echo_script = backend_exec_echo_script();
    let mock_path = h
        .write_mock_script("echo-backend.sh", &echo_script)
        .expect("write echo mock");
    let mock_str = mock_path.to_string_lossy().into_owned();
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.claude.command".to_owned(),
        mock_str.clone(),
        "--global".to_owned(),
    ])
    .expect("set claude command");
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.claude.args".to_owned(),
        "[]".to_owned(),
        "--global".to_owned(),
    ])
    .expect("set claude args");
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.codex.command".to_owned(),
        mock_str,
        "--global".to_owned(),
    ])
    .expect("set codex command");
    h.ralph_ok(vec![
        "config".to_owned(),
        "set".to_owned(),
        "backends.codex.args".to_owned(),
        "[]".to_owned(),
        "--global".to_owned(),
    ])
    .expect("set codex args");
}

fn happy_path_echo(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_echo_backend(h);
        let sentinel = "SENTINEL_BACKEND_EXEC_HAPPY";
        let output = h
            .ralph_with_stdin(["backend", "exec", "claude"], sentinel)
            .expect("backend exec");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(sentinel),
            "expected stdout to contain sentinel '{}', got:\n{}",
            sentinel,
            stdout
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("tokens_in"),
            "expected stderr to contain 'tokens_in' (metrics block), got:\n{}",
            stderr
        );
    })
}

fn unknown_backend(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        let output = h
            .ralph_with_stdin(
                ["backend", "exec", "nonexistent_backend_xyz"],
                "some prompt",
            )
            .expect("backend exec unknown");
        assert!(
            !output.status.success(),
            "expected non-zero exit for unknown backend, got: {:?}",
            output.status
        );
        assert_stderr_contains(&output, "unknown");

        // Failure-mode invariant: unknown backend must not create loop artifacts
        let projects_dir = h.repo_root.join(".ralph").join("projects");
        if projects_dir.exists() {
            let entries: Vec<_> = std::fs::read_dir(&projects_dir)
                .expect("read projects dir")
                .collect();
            assert!(
                entries.is_empty(),
                "unknown backend should not create project artifacts"
            );
        }
    })
}

fn raw_suppresses_metrics(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_echo_backend(h);
        let sentinel = "SENTINEL_RAW_MODE";
        let output = h
            .ralph_with_stdin(["backend", "exec", "claude", "--raw"], sentinel)
            .expect("backend exec --raw");
        assert_exit_code(&output, 0);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("tokens_in"),
            "expected stderr NOT to contain 'tokens_in' in raw mode, got:\n{}",
            stderr
        );
    })
}

fn prompt_from_file(h: &RalphHarness) -> TestResult {
    run_case(|| {
        setup_echo_backend(h);
        let prompt_content = "SENTINEL_PROMPT_FILE_CONTENT";
        let prompt_path = h.temp_dir.path().join("test-prompt.txt");
        std::fs::write(&prompt_path, prompt_content).expect("write prompt file");

        let output = h
            .ralph(vec![
                "backend",
                "exec",
                "claude",
                "--prompt",
                &prompt_path.to_string_lossy(),
            ])
            .expect("backend exec --prompt");
        assert_exit_code(&output, 0);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(prompt_content),
            "expected stdout to contain prompt file content '{}', got:\n{}",
            prompt_content,
            stdout
        );
    })
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
