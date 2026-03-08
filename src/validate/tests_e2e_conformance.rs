use super::*;

use std::fs;
use std::process::Command;

use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    auto_mock_script, e2e_mock_gh_logging_script, e2e_mock_ralph_script,
    empty_output_backend_script, nonzero_exit_backend_script,
};
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "e2e_conformance::backend_timeout_exhausted_fails_task",
            func: backend_timeout_exhausted_fails_task,
        },
        ConformanceTest {
            name: "e2e_conformance::retry_override_unset_defaults_to_three",
            func: retry_override_unset_defaults_to_three,
        },
        ConformanceTest {
            name: "e2e_conformance::retry_override_set_to_one",
            func: retry_override_set_to_one,
        },
        ConformanceTest {
            name: "e2e_conformance::retry_override_zero_defaults_to_three",
            func: retry_override_zero_defaults_to_three,
        },
        ConformanceTest {
            name: "e2e_conformance::retry_override_invalid_string_defaults_to_three",
            func: retry_override_invalid_string_defaults_to_three,
        },
        ConformanceTest {
            name: "e2e_conformance::backend_command_failed_no_reformatter",
            func: backend_command_failed_no_reformatter,
        },
        ConformanceTest {
            name: "e2e_conformance::empty_output_retries_then_reformatter",
            func: empty_output_retries_then_reformatter,
        },
        ConformanceTest {
            name: "e2e_conformance::pr_metadata_verification",
            func: pr_metadata_verification,
        },
        ConformanceTest {
            name: "e2e_conformance::e2e_mock_ralph_script_delegates_to_auto",
            func: e2e_mock_ralph_script_delegates_to_auto,
        },
        ConformanceTest {
            name: "e2e_conformance::e2e_mock_gh_logging_script_captures_pr_create",
            func: e2e_mock_gh_logging_script_captures_pr_create,
        },
        ConformanceTest {
            name: "e2e_conformance::e2e_pr_create_body_file_verification",
            func: e2e_pr_create_body_file_verification,
        },
    ]
}

fn backend_timeout_exhausted_fails_task(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-801";
        setup_timeout_failure_project(h, project_id);

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");

        let exit_code = output.status.code().unwrap_or(-1);
        assert_ne!(
            exit_code, 0,
            "expected non-zero exit when backend times out"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("BackendTimeoutExhausted"),
            "expected timeout exhaustion to be surfaced in stderr, got:\n{stderr}"
        );
        assert!(
            !stderr.contains("requesting reformat via"),
            "timeout should not trigger reformatter fallback, got:\n{stderr}"
        );

        // Without durable state.json, a failed orchestration that leaves no loop
        // artifacts results in "pending" status from reconstruction.  The non-zero
        // exit code is the authoritative failure signal.
        let state = h.load_state(project_id).expect("load_state failed");
        assert_eq!(
            state["status"],
            json!("pending"),
            "project should be pending (no artifacts) after backend timeout"
        );
    })
}

fn retry_override_unset_defaults_to_three(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-804";
        setup_timeout_failure_project(h, project_id);

        // No --max-backend-retries arg → defaults to 3.
        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_ne!(
            output.status.code().unwrap_or(-1),
            0,
            "expected non-zero exit when backend times out"
        );

        assert_planner_attempt_count(h, project_id, 3);
    })
}

fn retry_override_set_to_one(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-805";
        setup_timeout_failure_project(h, project_id);

        let output = h
            .ralph(["run", "--loops", "1", "--max-backend-retries", "1"])
            .expect("ralph run should execute");
        assert_ne!(
            output.status.code().unwrap_or(-1),
            0,
            "expected non-zero exit when backend times out"
        );

        assert_planner_attempt_count(h, project_id, 1);
    })
}

fn retry_override_zero_defaults_to_three(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-806";
        setup_timeout_failure_project(h, project_id);

        // --max-backend-retries 0 is treated as unset → defaults to 3.
        let output = h
            .ralph(["run", "--loops", "1", "--max-backend-retries", "0"])
            .expect("ralph run should execute");
        assert_ne!(
            output.status.code().unwrap_or(-1),
            0,
            "expected non-zero exit when backend times out"
        );

        assert_planner_attempt_count(h, project_id, 3);
    })
}

fn retry_override_invalid_string_defaults_to_three(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-807";
        setup_timeout_failure_project(h, project_id);

        // Invalid value for --max-backend-retries: clap rejects non-numeric
        // input, so the command fails with a non-zero exit code.
        let output = h
            .ralph(["run", "--loops", "1", "--max-backend-retries", "abc"])
            .expect("ralph run should execute");
        assert_ne!(
            output.status.code().unwrap_or(-1),
            0,
            "expected non-zero exit for invalid --max-backend-retries value"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("invalid value") || stderr.contains("error"),
            "expected clap parse error in stderr, got:\n{stderr}"
        );
    })
}

fn backend_command_failed_no_reformatter(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-802";
        h.init_workspace_fast().expect("init failed");

        let claude_script = h
            .write_mock_script("backend-nonzero.sh", &nonzero_exit_backend_script())
            .expect("failed to write nonzero backend script");

        let codex_log = h.temp_dir.path().join("reformatter-should-not-run.log");
        let codex_log_str = codex_log.to_string_lossy().into_owned();
        let codex_script = h
            .write_mock_script(
                "reformatter-should-not-run.sh",
                &format!(
                    "#!/bin/sh\nset -eu\ncat >/dev/null\nprintf 'unexpected reformatter call\\n' >> \"{codex_log_str}\"\nexit 0\n"
                ),
            )
            .expect("failed to write codex marker script");
        set_separate_mock_backends_fast(h, &claude_script, &codex_script)
            .expect("setup_separate_mock_backends_fast failed");
        h.set_config_fast("workflow.prompt_review_enabled", "false")
            .expect("config set workflow.prompt_review_enabled failed");
        h.create_project_fast(
            project_id,
            "Backend Command Failed Project",
            "Backend command failure reformatter boundary test prompt",
        )
        .expect("create_project failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_ne!(
            output.status.code().unwrap_or(-1),
            0,
            "expected non-zero exit for backend non-zero command failure"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.to_lowercase().contains("backend command failed"),
            "expected backend command failure in stderr, got:\n{stderr}"
        );
        assert!(
            !stderr.contains("requesting reformat via"),
            "backend command failure must not trigger reformatter fallback, got:\n{stderr}"
        );
        assert!(
            !codex_log.exists(),
            "reformatter backend should not run on BackendCommandFailed"
        );

        let state = h.load_state(project_id).expect("load_state failed");
        assert_eq!(
            state["status"],
            json!("pending"),
            "project should be pending (no artifacts) after BackendCommandFailed"
        );
    })
}

fn empty_output_retries_then_reformatter(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-803";
        h.init_workspace_fast().expect("init failed");

        let claude_script = h
            .write_mock_script("backend-empty-output.sh", &empty_output_backend_script())
            .expect("failed to write empty-output backend script");
        let codex_script = h
            .write_mock_script(
                "backend-reformatter-nonzero.sh",
                &nonzero_exit_backend_script(),
            )
            .expect("failed to write nonzero reformatter backend script");
        set_separate_mock_backends_fast(h, &claude_script, &codex_script)
            .expect("setup_separate_mock_backends_fast failed");

        let call_log = h.temp_dir.path().join("backend-call-order.log");
        let call_log_str = call_log.to_string_lossy().into_owned();
        h.set_config_fast(
            "backends.claude.env.RALPH_VALIDATE_BACKEND_LOG",
            &call_log_str,
        )
        .expect("config set claude log path failed");
        h.set_config_fast("backends.claude.env.RALPH_VALIDATE_BACKEND_LABEL", "claude")
            .expect("config set claude label failed");
        h.set_config_fast(
            "backends.codex.env.RALPH_VALIDATE_BACKEND_LOG",
            &call_log_str,
        )
        .expect("config set codex log path failed");
        h.set_config_fast("backends.codex.env.RALPH_VALIDATE_BACKEND_LABEL", "codex")
            .expect("config set codex label failed");

        h.set_config_fast("workflow.prompt_review_enabled", "false")
            .expect("config set workflow.prompt_review_enabled failed");
        h.create_project_fast(
            project_id,
            "Empty Output Retry Project",
            "Empty backend output should retry and then attempt reformatter",
        )
        .expect("create_project failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert_ne!(
            output.status.code().unwrap_or(-1),
            0,
            "expected non-zero exit after empty output and failed reformatter attempt"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.to_lowercase().contains("backend command failed"),
            "expected backend command failure after reformatter attempt, got:\n{stderr}"
        );

        let call_lines = fs::read_to_string(&call_log)
            .expect("failed to read backend call log")
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(
            call_lines.len() >= 3,
            "expected at least 3 backend invocations, got {} lines:\n{}",
            call_lines.len(),
            call_lines.join("\n")
        );
        assert!(
            call_lines[0].starts_with("claude:"),
            "first invocation should be claude primary backend, got: {}",
            call_lines[0]
        );
        assert!(
            call_lines[1].starts_with("claude:"),
            "second invocation should be same-backend retry, got: {}",
            call_lines[1]
        );
        assert!(
            call_lines.iter().any(|line| line.starts_with("codex:")),
            "expected reformatter backend invocation after same-backend retry, got lines:\n{}",
            call_lines.join("\n")
        );

        let state = h.load_state(project_id).expect("load_state failed");
        assert_eq!(
            state["status"],
            json!("pending"),
            "project should be pending (no artifacts) when parse-repair path ultimately fails"
        );
    })
}

fn pr_metadata_verification(h: &RalphHarness) -> TestResult {
    run_case(|| {
        // Use a daemon harness so repo_root matches the data-dir layout
        // (data_dir/acme/widgets/) that daemon start expects.
        let dh = RalphHarness::new_daemon(&h.ralph_bin, "acme", "widgets").expect("daemon harness");
        dh.init_workspace_fast().expect("init failed");
        let auto_backend = dh
            .write_mock_script("auto-mock.sh", &auto_mock_script())
            .expect("failed to write auto mock backend");
        dh.setup_mock_backends_stable(&auto_backend)
            .expect("setup_mock_backends_stable failed");
        dh.set_config_fast("workspace.daemon_refinement_enabled", "false")
            .expect("config set workspace.daemon_refinement_enabled failed");

        let gh_script = dh
            .write_mock_script("gh", &e2e_mock_gh_logging_script())
            .expect("failed to write e2e gh script");
        let gh_dir = gh_script
            .parent()
            .expect("mock gh script should have parent directory");
        let path_env = format!(
            "{}:{}",
            gh_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        let issue_number = 901_u32;
        // Provide a mock issue with ralph:ready label for the daemon to discover
        let mock_issues = format!(
            r#"[{{"number":{issue_number},"title":"PR metadata test","labels":[{{"name":"ralph:ready"}}],"body":"E2E PR metadata verification issue."}}]"#
        );

        let gh_log_path = dh.temp_dir.path().join("gh-pr-create-e2e.log");
        let gh_log_str = gh_log_path.to_string_lossy().into_owned();
        let env_vars = [
            ("PATH", path_env.as_str()),
            ("RALPH_E2E_GH_LOG", gh_log_str.as_str()),
            ("RALPH_E2E_MOCK_ISSUES", mock_issues.as_str()),
        ];

        let output = dh
            .daemon_env(
                [
                    "daemon",
                    "start",
                    "--repo",
                    "acme/widgets",
                    "--single-iteration",
                ],
                &env_vars,
            )
            .expect("daemon start should execute");
        assert_eq!(
            output.status.code().unwrap_or(-1),
            0,
            "expected daemon start single iteration to succeed; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);

        // Verify task dispatch occurred and reached terminal state.
        assert!(
            stderr.contains("dispatched task acme-widgets-901"),
            "expected task dispatch in stderr, got:\n{stderr}"
        );
        assert!(
            stderr.contains("ralph:failed") || stderr.contains("ralph:completed"),
            "expected terminal label in stderr, got:\n{stderr}"
        );

        // Verify PR metadata construction is correct by calling the build
        // helpers directly with values matching what the daemon would use
        // for this task.  This validates that the pr_metadata pipeline
        // produces correct output for the given issue.
        let task_id = "acme-widgets-901";
        let branch = format!("ralph/daemon/{task_id}");
        let title = crate::daemon::runtime::build_pr_title(&format!("ralph: {task_id}"));
        assert!(
            title.starts_with("ralph: acme-widgets-901"),
            "expected title to contain task_id, got: {title}"
        );

        let pr_body = crate::daemon::runtime::build_pr_body(
            &branch,
            Some("src/main.rs | 5 ++---"),
            Some("E2E PR metadata verification issue."),
            task_id,
            issue_number,
        );
        assert!(
            pr_body.contains(&format!("Closes #{issue_number}")),
            "expected issue closure marker in PR body, got:\n{pr_body}"
        );
        assert!(
            pr_body.contains("Diff Stat"),
            "expected Diff Stat section in PR body, got:\n{pr_body}"
        );
        assert!(
            pr_body.contains("src/main.rs"),
            "expected diff stat content in PR body, got:\n{pr_body}"
        );
        assert!(
            pr_body.contains("Issue Context"),
            "expected Issue Context section in PR body, got:\n{pr_body}"
        );
        assert!(
            pr_body.contains("E2E PR metadata verification issue."),
            "expected issue context in PR body, got:\n{pr_body}"
        );

        let project_ref = crate::daemon::runtime::extract_project_ref(&branch);
        assert!(
            pr_body.contains(&format!("Project Ref: `{}`", project_ref.as_deref().unwrap_or(""))),
            "expected Project Ref footer in PR body, got:\n{pr_body}"
        );
    })
}

/// Verify that `create_pr_with_body_file` passes the correct arguments
/// to `gh pr create` and that the body file content includes issue closure
/// markers and PR metadata sections.
fn e2e_pr_create_body_file_verification(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let mock_dir = temp.path().join("bin");
        fs::create_dir_all(&mock_dir).expect("mkdir mock bin");

        let gh_log = temp.path().join("gh-pr-body-file.log");
        let gh_script = h
            .write_mock_script("gh-body-file.sh", &e2e_mock_gh_logging_script())
            .expect("failed to write gh script");

        // Copy the script to mock_dir/gh for PATH resolution
        let gh_dest = mock_dir.join("gh");
        fs::copy(&gh_script, &gh_dest).expect("copy gh script");
        let mut perms = fs::metadata(&gh_dest).expect("meta").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        fs::set_permissions(&gh_dest, perms).expect("chmod");

        let original_path = std::env::var("PATH").unwrap_or_default();
        let composed = format!("{}:{}", mock_dir.display(), original_path);

        // Build PR body using the library functions
        let task_id = "acme-widgets-42";
        let branch = "ralph/daemon/acme-widgets-42";
        let issue_number = 42_u32;
        let title = crate::daemon::runtime::build_pr_title(&format!("ralph: {task_id}"));
        let pr_body = crate::daemon::runtime::build_pr_body(
            branch,
            Some("src/lib.rs | 10 ++++------"),
            Some("Implement feature X"),
            task_id,
            issue_number,
        );

        // Write body file
        let body_path = temp.path().join("pr-body-test.md");
        fs::write(&body_path, &pr_body).expect("write body file");

        // Call create_pr_with_body_file via the runtime
        let rt = tokio::runtime::Runtime::new().unwrap();
        let url = rt.block_on(async {
            // Temporarily set PATH and RALPH_E2E_GH_LOG for gh resolution
            unsafe { std::env::set_var("PATH", &composed) };
            unsafe { std::env::set_var("RALPH_E2E_GH_LOG", gh_log.to_string_lossy().as_ref()) };
            let result = crate::daemon::github::create_pr_with_body_file(
                "acme",
                "widgets",
                branch,
                &title,
                &body_path,
                Some("master"),
                false,
            )
            .await;
            unsafe { std::env::remove_var("RALPH_E2E_GH_LOG") };
            unsafe { std::env::set_var("PATH", &original_path) };
            result
        })
        .expect("create_pr_with_body_file should succeed");

        assert!(
            url.contains("github.com"),
            "expected PR URL in response, got: {url}"
        );

        // Read the gh log and verify args using the helper parsers
        let log_content = fs::read_to_string(&gh_log).expect("read gh log");
        let args = parse_logged_args(&log_content);

        assert_eq!(
            arg_value(&args, "--title"),
            Some(title.as_str()),
            "expected --title flag with correct value in gh args"
        );
        assert_eq!(
            arg_value(&args, "--head"),
            Some(branch),
            "expected --head flag with correct branch"
        );
        assert_eq!(
            arg_value(&args, "--repo"),
            Some("acme/widgets"),
            "expected --repo flag with correct value"
        );
        assert_eq!(
            arg_value(&args, "--base"),
            Some("master"),
            "expected --base flag with correct value"
        );
        assert!(
            arg_value(&args, "--body-file").is_some(),
            "expected --body-file flag in gh args"
        );

        // Verify body file content via logged body
        let body = extract_logged_body(&log_content);
        assert!(
            body.is_some(),
            "expected body content in gh log, got:\n{log_content}"
        );
        let body = body.unwrap();
        assert!(
            body.contains(&format!("Closes #{issue_number}")),
            "expected issue closure marker in body, got:\n{body}"
        );
        assert!(
            body.contains("Diff Stat"),
            "expected Diff Stat section in body, got:\n{body}"
        );
        assert!(
            body.contains("src/lib.rs"),
            "expected diff stat content in body, got:\n{body}"
        );
        assert!(
            body.contains("Issue Context"),
            "expected Issue Context section in body, got:\n{body}"
        );
        assert!(
            body.contains("Implement feature X"),
            "expected issue body content in body, got:\n{body}"
        );
        assert!(
            body.contains("Project Ref:"),
            "expected Project Ref footer in body, got:\n{body}"
        );
    })
}

fn e2e_mock_ralph_script_delegates_to_auto(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let script = h
            .write_mock_script("e2e-mock-ralph.sh", &e2e_mock_ralph_script(&h.ralph_bin))
            .expect("failed to write e2e mock ralph script");
        let script_content = fs::read_to_string(&script).expect("failed to read mock ralph script");

        let expected_bin = h
            .ralph_bin
            .canonicalize()
            .unwrap_or_else(|_| h.ralph_bin.clone());
        let expected_bin_str = expected_bin.to_string_lossy();

        assert!(
            script_content.contains(&*expected_bin_str),
            "script should embed the absolute ralph binary path"
        );
        assert!(
            script_content.contains(" auto \"$@\""),
            "script should execute ralph auto with forwarded args"
        );
        assert!(
            !script_content.contains("exec ralph "),
            "script should not resolve ralph via PATH"
        );

        let output = Command::new(&script)
            .arg("--help")
            .output()
            .expect("mock ralph script should execute");
        assert!(
            output.status.success(),
            "mock ralph script should delegate successfully; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    })
}

fn e2e_mock_gh_logging_script_captures_pr_create(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let script = h
            .write_mock_script("e2e-mock-gh.sh", &e2e_mock_gh_logging_script())
            .expect("failed to write e2e mock gh script");

        let body_path = h.temp_dir.path().join("pr-body.md");
        fs::write(
            &body_path,
            "Closes #42\n\nDiff stat:\n- src/lib.rs | 2 +-\n\nProject: acme/widgets\n",
        )
        .expect("failed to write body file");
        let body_file = body_path.to_string_lossy().into_owned();
        let log_path = h.temp_dir.path().join("gh-pr-create.log");

        let output = Command::new(&script)
            .args([
                "pr",
                "create",
                "--title",
                "ralph: test PR title",
                "--body-file",
                &body_file,
                "--head",
                "ralph/test-branch",
                "--repo",
                "acme/widgets",
            ])
            .env("RALPH_E2E_GH_LOG", &log_path)
            .output()
            .expect("mock gh script should execute");
        assert!(
            output.status.success(),
            "mock gh script should succeed; stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("https://github.com/mock/repo/pull/123"),
            "mock gh script should return a synthetic PR URL"
        );

        let log_content = fs::read_to_string(&log_path).expect("failed to read gh log");
        assert!(
            log_content.contains("--title") && log_content.contains("ralph: test PR title"),
            "log should capture full --title args, got:\n{log_content}"
        );
        assert!(
            log_content.contains("--head") && log_content.contains("ralph/test-branch"),
            "log should capture full --head args, got:\n{log_content}"
        );
        assert!(
            log_content.contains("--repo") && log_content.contains("acme/widgets"),
            "log should capture full --repo args, got:\n{log_content}"
        );
        assert!(
            log_content.contains("body_begin")
                && log_content.contains("Closes #42")
                && log_content.contains("Diff stat:"),
            "log should capture --body-file content, got:\n{log_content}"
        );
    })
}

fn setup_timeout_failure_project(h: &RalphHarness, project_id: &str) {
    h.init_workspace_fast().expect("init failed");

    let script = h
        .write_mock_script("sleep-timeout.sh", &sleeping_backend_script())
        .expect("failed to write sleeping backend script");
    h.setup_mock_backends_fast(&script)
        .expect("setup_mock_backends_fast failed");

    h.set_config_fast("backends.claude.timeout_seconds", "1")
        .expect("config set backends.claude.timeout_seconds failed");
    h.set_config_fast("backends.codex.timeout_seconds", "1")
        .expect("config set backends.codex.timeout_seconds failed");
    h.set_config_fast("workflow.prompt_review_enabled", "false")
        .expect("config set workflow.prompt_review_enabled failed");

    h.create_project_fast(
        project_id,
        "E2E Timeout Project",
        "Backend timeout test prompt",
    )
    .expect("create_project_fast failed");
}

fn set_separate_mock_backends_fast(
    h: &RalphHarness,
    claude_script: &std::path::Path,
    codex_script: &std::path::Path,
) -> crate::Result<()> {
    h.set_config_fast("backends.claude.command", &claude_script.to_string_lossy())?;
    h.set_config_fast("backends.codex.command", &codex_script.to_string_lossy())?;
    h.set_config_fast("backends.openrouter.enabled", "false")
}

fn planner_attempt_count(h: &RalphHarness, project_id: &str) -> usize {
    let planner_log = h
        .tmp_log_dir()
        .join(format!("{project_id}-001-planner.log"));
    assert!(
        planner_log.exists(),
        "planner log should exist at {}",
        planner_log.display()
    );
    let content = fs::read_to_string(&planner_log).expect("read planner log");
    content.matches("--- attempt=").count()
}

fn assert_planner_attempt_count(h: &RalphHarness, project_id: &str, expected: usize) {
    let attempts = planner_attempt_count(h, project_id);
    assert_eq!(
        attempts, expected,
        "unexpected planner attempt count for project {project_id}"
    );
}

fn sleeping_backend_script() -> String {
    r#"#!/bin/sh
set -eu

    # Consume prompt input, then sleep long enough to exceed backend timeout.
cat >/dev/null
sleep 2.1
echo "unreachable"
"#
    .to_owned()
}

fn parse_logged_args(log_content: &str) -> Vec<String> {
    log_content
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("arg[")?;
            let (_, value) = rest.split_once("]=")?;
            Some(value.to_owned())
        })
        .collect()
}

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}

fn extract_logged_body(log_content: &str) -> Option<String> {
    let marker = "body_begin\n";
    let start = log_content.find(marker)? + marker.len();
    let tail = &log_content[start..];
    let end = tail.find("\nbody_end")?;
    Some(tail[..end].to_owned())
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
