use super::*;

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::validate::assertions::{
    assert_artifact_timestamp_naming, assert_exit_code, assert_file_exists, assert_json_array_len,
    assert_json_field, assert_no_loop_artifacts, assert_no_uncommitted_ralph_files,
    parse_yaml_frontmatter,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{
    always_reject_review_script, mock_tmux_script, prompt_mutating_mock_script,
    review_feedback_once_then_approve_script, standard_mock_script,
};
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "run::single_feature_loop",
            func: single_feature_loop,
        },
        ConformanceTest {
            name: "run::artifact_naming",
            func: artifact_naming,
        },
        ConformanceTest {
            name: "run::agent_output_artifacts",
            func: agent_output_artifacts,
        },
        ConformanceTest {
            name: "run::planner_no_agent_output",
            func: planner_no_agent_output,
        },
        ConformanceTest {
            name: "run::artifact_frontmatter",
            func: artifact_frontmatter,
        },
        ConformanceTest {
            name: "run::state_after_loop",
            func: state_after_loop,
        },
        ConformanceTest {
            name: "run::git_tag_format",
            func: git_tag_format,
        },
        ConformanceTest {
            name: "run::two_loops_alternation",
            func: two_loops_alternation,
        },
        ConformanceTest {
            name: "run::completion_flow",
            func: completion_flow,
        },
        ConformanceTest {
            name: "run::review_limit_fails",
            func: review_limit_fails,
        },
        ConformanceTest {
            name: "run::dry_run",
            func: dry_run,
        },
        ConformanceTest {
            name: "run::until_review",
            func: until_review,
        },
        ConformanceTest {
            name: "run::resume_after_interrupt",
            func: resume_after_interrupt,
        },
        ConformanceTest {
            name: "run::dirty_tree_rejected",
            func: dirty_tree_rejected,
        },
        ConformanceTest {
            name: "run::skip_commit",
            func: skip_commit,
        },
        ConformanceTest {
            name: "run::loops_flag",
            func: loops_flag,
        },
        ConformanceTest {
            name: "run::template_fallback_when_file_missing",
            func: template_fallback_when_file_missing,
        },
        ConformanceTest {
            name: "run::completion_artifacts_committed",
            func: completion_artifacts_committed,
        },
        ConformanceTest {
            name: "run::impl_response_artifact_on_review_feedback",
            func: impl_response_artifact_on_review_feedback,
        },
        ConformanceTest {
            name: "run::tmux_enabled_no_loop_dir_logs",
            func: tmux_enabled_no_loop_dir_logs,
        },
        ConformanceTest {
            name: "run::on_prompt_change_flag_accepted",
            func: on_prompt_change_flag_accepted,
        },
        ConformanceTest {
            name: "run::on_prompt_change_abort_triggers",
            func: on_prompt_change_abort_triggers,
        },
        ConformanceTest {
            name: "run::workspace_root_uses_alternate_path",
            func: workspace_root_uses_alternate_path,
        },
    ]
}

fn single_feature_loop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-201";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];
        let project_dir = h.project_dir(project_id);

        let spec_rel = loop_state["artifacts"]["spec"]
            .as_str()
            .expect("spec artifact should exist");
        let impl_notes_rel = loop_state["artifacts"]["impl_notes"]
            .as_str()
            .expect("impl-notes artifact should exist");
        let approval_rel = loop_state["artifacts"]["approval"]
            .as_str()
            .expect("review-approved artifact should exist");

        assert_file_exists(&project_dir.join(spec_rel));
        assert_file_exists(&project_dir.join(impl_notes_rel));
        assert_file_exists(&project_dir.join(approval_rel));

        assert!(
            loop_state["commit"].as_str().is_some(),
            "expected loop to have a commit hash"
        );
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn artifact_naming(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-202";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts should succeed");
        assert!(!artifacts.is_empty(), "expected at least one loop artifact");

        for artifact in &artifacts {
            assert_artifact_timestamp_naming(artifact);
        }

        let names = artifacts
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("artifact filename should be valid UTF-8")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert!(
            names.iter().any(|name| name.ends_with("-spec.md")),
            "expected spec artifact in loop directory"
        );
        assert!(
            names.iter().any(|name| name.ends_with("-impl-notes.md")),
            "expected impl-notes artifact in loop directory"
        );
        assert!(
            names
                .iter()
                .any(|name| name.ends_with("-review-approved.md")),
            "expected review-approved artifact in loop directory"
        );
    })
}

fn artifact_frontmatter(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-203";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        let loop_state = loops.first().expect("expected one loop entry");
        let spec_rel = loop_state["artifacts"]["spec"]
            .as_str()
            .expect("spec artifact should exist");
        let spec_path = h.project_dir(project_id).join(spec_rel);

        let fm = parse_yaml_frontmatter(&spec_path);
        assert!(
            fm.get("artifact").is_some(),
            "missing 'artifact' frontmatter"
        );
        assert!(fm.get("loop").is_some(), "missing 'loop' frontmatter");
        assert!(fm.get("project").is_some(), "missing 'project' frontmatter");
        assert!(fm.get("backend").is_some(), "missing 'backend' frontmatter");
        assert!(fm.get("role").is_some(), "missing 'role' frontmatter");
    })
}

fn agent_output_artifacts(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-204";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        // Agent output logs are routed to .ralph/tmp/logs (not loop directories)
        let tmp_log_dir = h.tmp_log_dir();
        let log_files: Vec<String> = fs::read_dir(&tmp_log_dir)
            .expect("read tmp log dir")
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(project_id) && name.ends_with(".log") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !log_files.is_empty(),
            "expected at least one agent-output log in tmp/logs; found: {log_files:?}"
        );
        assert!(
            log_files.iter().any(|name| name.contains("-implementer")),
            "expected implementer log in tmp/logs; found: {log_files:?}"
        );
        assert!(
            log_files.iter().any(|name| name.contains("-reviewer")),
            "expected reviewer log in tmp/logs; found: {log_files:?}"
        );

        // Verify no agent-output logs exist in loop directories
        let artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts should succeed");
        let loop_logs: Vec<_> = artifacts
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains("agent-output-") && n.ends_with(".log"))
            })
            .collect();
        assert!(
            loop_logs.is_empty(),
            "agent-output logs should NOT exist in loop directory; found: {loop_logs:?}"
        );
    })
}

fn planner_no_agent_output(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-205";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        // With tmp-log routing, planner logs go to .ralph/tmp/logs but should
        // NOT produce timestamped artifacts in loop directories.
        let artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts should succeed");
        let names = artifacts
            .iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("artifact filename should be valid UTF-8")
                    .to_owned()
            })
            .collect::<Vec<_>>();

        assert!(
            !names
                .iter()
                .any(|name| name.contains("-agent-output-planner-")),
            "planner should not write agent-output artifacts in loop dir"
        );
    })
}

fn state_after_loop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-206";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "current_loop", &json!(1));
        assert_json_field(&state, "current_phase", &json!("planning"));
        assert_json_field(&state, "status", &json!("in_progress"));
    })
}

fn git_tag_format(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-207";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        let subject = git_log_subject(&h.repo_root, "HEAD");
        assert!(
            subject.starts_with(&format!("ralph({project_id}): loop 1 ")),
            "expected structured Ralph checkpoint commit subject, got '{subject}'"
        );
        let body = git_log_body(&h.repo_root, "HEAD");
        assert!(
            body.contains(&format!("Ralph-Project: {project_id}")),
            "expected Ralph-Project trailer in checkpoint commit body"
        );
        assert!(
            body.contains("Ralph-Loop: 1"),
            "expected Ralph-Loop trailer in checkpoint commit body"
        );
    })
}

fn two_loops_alternation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        use crate::validate::assertions::normalize_backend;

        let project_id = "issue-208";
        h.init_workspace().expect("init failed");

        let claude_script = h
            .write_mock_script("claude-mock.sh", &standard_mock_script())
            .expect("failed to write claude mock script");
        let codex_script = h
            .write_mock_script("codex-mock.sh", &standard_mock_script())
            .expect("failed to write codex mock script");
        h.setup_separate_mock_backends(&claude_script, &codex_script)
            .expect("setup_separate_mock_backends failed");

        h.create_project(
            project_id,
            "Alternation Run Project",
            "Run alternation prompt",
        )
        .expect("create_project failed");
        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 2, "expected two loop entries");

        let project_dir = h.project_dir(project_id);
        let loop1_spec = project_dir.join(
            loops[0]["artifacts"]["spec"]
                .as_str()
                .expect("loop 1 spec artifact should exist"),
        );
        let loop2_spec = project_dir.join(
            loops[1]["artifacts"]["spec"]
                .as_str()
                .expect("loop 2 spec artifact should exist"),
        );

        let loop1_frontmatter = parse_yaml_frontmatter(&loop1_spec);
        let loop1_backend_raw = loop1_frontmatter["backend"]
            .as_str()
            .expect("loop 1 spec frontmatter backend should be a string");
        let loop2_frontmatter = parse_yaml_frontmatter(&loop2_spec);
        let loop2_backend_raw = loop2_frontmatter["backend"]
            .as_str()
            .expect("loop 2 spec frontmatter backend should be a string");

        // Normalize backend strings (strip model suffixes like "claude(sonnet-4)" → "claude")
        let loop1_backend = normalize_backend(loop1_backend_raw);
        let loop2_backend = normalize_backend(loop2_backend_raw);

        // Per spec contract: loop 1 planner=claude, loop 2 planner=codex
        assert_eq!(
            loop1_backend, "claude",
            "expected loop 1 planner backend to be 'claude', got '{loop1_backend}' (raw: '{loop1_backend_raw}')"
        );
        assert_eq!(
            loop2_backend, "codex",
            "expected loop 2 planner backend to be 'codex', got '{loop2_backend}' (raw: '{loop2_backend_raw}')"
        );
    })
}

fn completion_flow(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-209";
        setup_with_standard_mock(h, project_id);

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run with env should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        assert_no_uncommitted_ralph_files(&h.repo_root);
    })
}

fn review_limit_fails(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-210";
        h.init_workspace().expect("init failed");

        let script = h
            .write_mock_script("reject-review.sh", &always_reject_review_script())
            .expect("failed to write reject-review script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(project_id, "Review Limit Project", "Review limit prompt")
            .expect("create_project failed");

        h.ralph_ok(["config", "set", "workflow.max_review_iterations", "1"])
            .expect("config set workflow.max_review_iterations failed");

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert!(
            !output.status.success(),
            "expected run to fail after review iteration limit"
        );

        // With state.json removed, failed orchestration results in a rolled-back
        // project directory with no loops.  Reconstruction derives "pending" status
        // since there are no artifacts on disk.  The non-zero exit code above is the
        // authoritative failure signal (the daemon maps this to ralph:failed label).
        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("pending"));
        assert_json_array_len(&state, "loops", 0);
        // No-checkpoint baseline: current_loop=1, current_phase=planning
        assert_json_field(&state, "current_loop", &json!(1));
        assert_json_field(&state, "current_phase", &json!("planning"));
        assert_no_loop_artifacts(&h.project_dir(project_id));
    })
}

fn dry_run(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-211";
        setup_with_standard_mock(h, project_id);

        let prompt_path = h.project_dir(project_id).join("prompt.md");
        let before_prompt =
            fs::read_to_string(&prompt_path).expect("failed to read prompt before dry-run");

        h.ralph_ok(["run", "--dry-run", "--loops", "1"])
            .expect("ralph run --dry-run should succeed");

        let after_prompt =
            fs::read_to_string(&prompt_path).expect("failed to read prompt after dry-run");
        assert_eq!(
            before_prompt, after_prompt,
            "prompt should not change during dry-run"
        );
        assert_no_loop_artifacts(&h.project_dir(project_id));

        // Dry-run produces no checkpoint commits; reconstruction defaults
        // to the no-checkpoint baseline: loop 1, phase planning.
        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_array_len(&state, "loops", 0);
        assert_json_field(&state, "current_loop", &json!(1));
        assert_json_field(&state, "current_phase", &json!("planning"));
    })
}

fn until_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-212";
        setup_with_standard_mock(h, project_id);

        // With structured checkpoint commits at phase boundaries, HEAD advances
        // during --until-review (checkpoint commits are created for each phase
        // transition).  We verify state properties instead of HEAD stability.
        h.ralph_ok(["run", "--until-review"])
            .expect("ralph run --until-review should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one in-progress loop");
        let loop_state = &loops[0];
        let project_dir = h.project_dir(project_id);

        assert_json_field(&state, "current_phase", &json!("committing"));
        assert_json_field(&state, "status", &json!("in_progress"));

        let spec_rel = loop_state["artifacts"]["spec"]
            .as_str()
            .expect("spec artifact should exist");
        let impl_notes_rel = loop_state["artifacts"]["impl_notes"]
            .as_str()
            .expect("impl-notes artifact should exist");
        let approval_rel = loop_state["artifacts"]["approval"]
            .as_str()
            .expect("review-approved artifact should exist");

        assert_file_exists(&project_dir.join(spec_rel));
        assert_file_exists(&project_dir.join(impl_notes_rel));
        assert_file_exists(&project_dir.join(approval_rel));
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn resume_after_interrupt(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-213";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--until-review"])
            .expect("ralph run --until-review should succeed");
        h.ralph_ok(["run"])
            .expect("second ralph run should resume and succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        // With checkpoint commits at phase boundaries, the resume from --until-review
        // may produce 1 or 2 loop entries depending on how reconstruction interprets
        // the checkpoint state. At minimum, the first loop must be completed.
        assert!(
            !loops.is_empty(),
            "expected at least one loop entry after resume"
        );
        let loop_state = &loops[0];

        assert_eq!(loop_state["status"], json!("completed"));
        assert!(
            loop_state["commit"].as_str().is_some(),
            "commit hash should be populated after resume"
        );
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn dirty_tree_rejected(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-214";
        setup_with_standard_mock(h, project_id);

        fs::write(h.repo_root.join("dirty.txt"), "uncommitted change")
            .expect("failed to write dirty file");
        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run should execute");
        assert!(
            !output.status.success(),
            "expected ralph run to fail with dirty working tree"
        );
    })
}

fn skip_commit(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-215";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--skip-commit", "--loops", "1"])
            .expect("ralph run --skip-commit should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop entry");
        let loop_state = &loops[0];
        assert_eq!(loop_state["status"], json!("completed"));
        assert!(
            loop_state["commit"].as_str().is_some(),
            "loop commit should still be set via structured phase checkpointing"
        );
        assert_has_ralph_checkpoint_commit(&h.repo_root, project_id);
    })
}

fn loops_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-216";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "2"])
            .expect("ralph run --loops 2 should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_array_len(&state, "loops", 2);

        let loops = state["loops"].as_array().expect("loops should be an array");
        for loop_state in loops {
            assert_eq!(
                loop_state["status"],
                json!("completed"),
                "all loop entries should be completed"
            );
        }
    })
}

fn template_fallback_when_file_missing(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-217";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["config", "set", "workflow.qa_enabled", "true"])
            .expect("config set workflow.qa_enabled true failed");

        let qa_template = h.repo_root.join(".ralph").join("templates").join("qa.md");
        if qa_template.exists() {
            fs::remove_file(&qa_template).expect("failed to remove workspace qa template");
        }

        let output = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop entry");

        let qa_results = loops[0]["artifacts"]["qa_results"]
            .as_array()
            .expect("artifacts.qa_results should be an array");
        assert!(
            !qa_results.is_empty(),
            "expected qa_results to be non-empty when QA is enabled"
        );
        assert_eq!(
            loops[0]["status"],
            json!("completed"),
            "expected loop status to be completed"
        );
    })
}

fn completion_artifacts_committed(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-218";
        setup_with_standard_mock(h, project_id);

        let head_before = git_head(&h.repo_root);

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run with RALPH_COMPLETE should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));

        // HEAD should have advanced (completion commit created)
        let head_after = git_head(&h.repo_root);
        assert_ne!(
            head_before, head_after,
            "HEAD should advance after completion commit"
        );

        // No uncommitted .ralph/ files should remain
        assert_no_uncommitted_ralph_files(&h.repo_root);

        // The completion commit message should reference completion artifacts
        let log_output = Command::new("git")
            .args(["log", "-1", "--format=%s"])
            .current_dir(&h.repo_root)
            .output()
            .expect("git log should execute");
        let commit_msg = String::from_utf8_lossy(&log_output.stdout);
        assert!(
            commit_msg.contains("completing"),
            "completion commit message should contain 'completing', got: {}",
            commit_msg.trim()
        );
    })
}

fn impl_response_artifact_on_review_feedback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-219";
        let review_counter = h.temp_dir.path().join("review-counter.txt");
        let script = review_feedback_once_then_approve_script(&review_counter);

        h.init_workspace().expect("init failed");
        let script_path = h
            .write_mock_script("feedback-then-approve.sh", &script)
            .expect("failed to write feedback-then-approve script");
        h.setup_mock_backends_stable(&script_path)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Impl Response Conformance Project",
            "Impl response test prompt",
        )
        .expect("create_project failed");

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed with review feedback cycle");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected exactly one completed loop");
        let loop_state = &loops[0];

        // Assert at least one review exchange occurred
        let reviews = loop_state["artifacts"]["reviews"]
            .as_array()
            .expect("reviews should be an array");
        assert!(
            !reviews.is_empty(),
            "expected at least one review exchange from feedback cycle"
        );

        // Find the impl-response-001 artifact
        let artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts should succeed");
        let impl_response_artifacts: Vec<_> = artifacts
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with("-impl-response-001.md"))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            impl_response_artifacts.len(),
            1,
            "expected exactly one *-impl-response-001.md artifact, got {}",
            impl_response_artifacts.len()
        );

        let impl_response_path = impl_response_artifacts[0];

        // Assert YAML frontmatter includes required keys
        let fm = parse_yaml_frontmatter(impl_response_path);
        assert!(
            fm.get("artifact").is_some(),
            "impl-response frontmatter missing 'artifact' key"
        );
        assert!(
            fm.get("iteration").is_some(),
            "impl-response frontmatter missing 'iteration' key"
        );
        assert!(
            fm.get("role").is_some(),
            "impl-response frontmatter missing 'role' key"
        );

        // Assert body contains expected implementer response content
        let content = fs::read_to_string(impl_response_path).unwrap_or_else(|err| {
            panic!(
                "failed to read {}: {err}",
                impl_response_path.to_string_lossy()
            )
        });
        assert!(
            content.contains("# Implementation Response"),
            "impl-response artifact should contain '# Implementation Response' heading"
        );
        assert!(
            content.contains("## Changes Made"),
            "impl-response artifact should contain '## Changes Made' section"
        );
        assert!(
            content.contains("Addressed reviewer feedback"),
            "impl-response artifact body should contain expected feedback response content"
        );
    })
}

fn tmux_enabled_no_loop_dir_logs(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-tmux-log";
        setup_with_standard_mock(h, project_id);

        // Write mock tmux script and place it on PATH
        let mock_tmux_content = mock_tmux_script();
        let mock_tmux_bin_dir = h.temp_dir.path().join("mock-bin");
        fs::create_dir_all(&mock_tmux_bin_dir).expect("create mock-bin dir");
        let mock_tmux_path = mock_tmux_bin_dir.join("tmux");
        fs::write(&mock_tmux_path, mock_tmux_content).expect("write mock tmux");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&mock_tmux_path)
                .expect("metadata")
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&mock_tmux_path, perms).expect("set permissions");
        }

        // Run with --tmux CLI flag and mock tmux on PATH
        let output = h
            .ralph_with_path(["run", "--tmux", "--loops", "1"], &[&mock_tmux_bin_dir])
            .expect("ralph run --tmux --loops 1 with tmux should execute");
        assert!(
            output.status.success(),
            "ralph run --tmux --loops 1 should succeed; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Verify agent output logs exist in .ralph/tmp/logs
        let tmp_log_dir = h.tmp_log_dir();
        assert!(
            tmp_log_dir.exists(),
            "tmp log dir should exist at {}",
            tmp_log_dir.display()
        );
        let log_files: Vec<String> = fs::read_dir(&tmp_log_dir)
            .expect("read tmp log dir")
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(project_id) && name.ends_with(".log") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !log_files.is_empty(),
            "expected at least one agent-output log in tmp/logs for tmux run; found: {log_files:?}"
        );

        // Verify NO agent-output-*.log files in loop directories
        let artifacts = h
            .list_artifacts(project_id, 1)
            .expect("list_artifacts should succeed");
        let loop_logs: Vec<_> = artifacts
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains("agent-output-") && n.ends_with(".log"))
            })
            .collect();
        assert!(
            loop_logs.is_empty(),
            "agent-output logs should NOT exist in loop directory for tmux run; found: {loop_logs:?}"
        );
    })
}

fn on_prompt_change_flag_accepted(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-opc-accept";
        setup_with_standard_mock(h, project_id);

        // --on-prompt-change abort should be accepted as a valid flag
        // and succeed with stable mocks (no actual prompt mutation)
        let output = h
            .ralph(["run", "--on-prompt-change", "abort", "--loops", "1"])
            .expect("ralph run --on-prompt-change abort should execute");
        assert!(
            output.status.success(),
            "ralph run --on-prompt-change abort --loops 1 should succeed; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    })
}

fn on_prompt_change_abort_triggers(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-opc-abort";
        h.init_workspace().expect("init failed");

        let prompt_path = h
            .repo_root
            .join(".ralph")
            .join("projects")
            .join(project_id)
            .join("prompt.md");

        // We need to create the project first so the prompt path exists
        let script = h
            .write_mock_script("standard-for-opc.sh", &standard_mock_script())
            .expect("failed to write standard mock script");
        h.setup_mock_backends_stable(&script)
            .expect("setup_mock_backends_stable failed");
        h.create_project(
            project_id,
            "Prompt Change Abort Project",
            "Prompt change test prompt",
        )
        .expect("create_project failed");

        // Now replace mock with prompt-mutating mock
        let mutating_script = prompt_mutating_mock_script(&prompt_path);
        let mutating_path = h
            .write_stable_mock_script("prompt-mutating-mock.sh", &mutating_script)
            .expect("failed to write prompt-mutating mock script");
        h.setup_mock_backends_stable(&mutating_path)
            .expect("setup_mock_backends_stable failed");

        let head_before = git_head(&h.repo_root);

        let output = h
            .ralph(["run", "--on-prompt-change", "abort", "--loops", "1"])
            .expect("ralph run should execute");
        assert!(
            !output.status.success(),
            "expected non-zero exit when prompt changes with abort mode"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("prompt changed"),
            "expected stderr to contain 'prompt changed', got: {stderr}"
        );

        // Failure-mode invariant: no new completed loop recorded
        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        let completed_loops: Vec<_> = loops
            .iter()
            .filter(|l| l["status"].as_str() == Some("completed"))
            .collect();
        assert!(
            completed_loops.is_empty(),
            "expected no completed loops after prompt-change abort"
        );

        // HEAD should remain at pre-run commit
        let head_after = git_head(&h.repo_root);
        assert_eq!(
            head_before, head_after,
            "HEAD should remain unchanged after prompt-change abort"
        );
    })
}

fn workspace_root_uses_alternate_path(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "issue-wsr";

        // Initialize workspace and project normally at h.repo_root
        setup_with_standard_mock(h, project_id);

        // Move .ralph to an alternate location so discovery from repo_root fails
        let alt_root = h.temp_dir.path().join("alt-workspace");
        fs::create_dir_all(&alt_root).expect("create alt-workspace dir");
        let original_ralph = h.repo_root.join(".ralph");
        let alt_ralph = alt_root.join(".ralph");
        fs::rename(&original_ralph, &alt_ralph).expect("move .ralph to alt location");

        // Non-vacuous invariant: without --workspace-root, discovery fails
        let output_no_flag = h
            .ralph(["run", "--loops", "1"])
            .expect("ralph run without workspace-root should execute");
        assert!(
            !output_no_flag.status.success(),
            "expected ralph run to fail without workspace at repo root and no --workspace-root flag"
        );

        // With --workspace-root pointing to the alternate location, command succeeds
        let output = h
            .ralph([
                "run",
                "--workspace-root",
                &alt_root.to_string_lossy(),
                "--loops",
                "1",
            ])
            .expect("ralph run with --workspace-root should execute");
        assert!(
            output.status.success(),
            "ralph run --workspace-root should succeed; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    })
}

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends_stable(&script)
        .expect("setup_mock_backends_stable failed");
    h.create_project(
        project_id,
        "Run Conformance Project",
        "Run suite test prompt",
    )
    .expect("create_project failed");
}

fn git_head(repo_root: &Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .expect("git rev-parse should execute");
    assert!(
        output.status.success(),
        "git rev-parse HEAD failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn git_log_subject(repo_root: &Path, rev: &str) -> String {
    let output = Command::new("git")
        .args(["show", "-s", "--format=%s", rev])
        .current_dir(repo_root)
        .output()
        .expect("git show should execute");
    assert!(
        output.status.success(),
        "git show subject failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn git_log_body(repo_root: &Path, rev: &str) -> String {
    let output = Command::new("git")
        .args(["show", "-s", "--format=%b", rev])
        .current_dir(repo_root)
        .output()
        .expect("git show should execute");
    assert!(
        output.status.success(),
        "git show body failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn assert_has_ralph_checkpoint_commit(repo_root: &Path, project_id: &str) {
    let output = Command::new("git")
        .args(["log", "--format=%s"])
        .current_dir(repo_root)
        .output()
        .expect("git log should execute");
    assert!(
        output.status.success(),
        "git log failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let subjects = String::from_utf8_lossy(&output.stdout);
    assert!(
        subjects
            .lines()
            .any(|line| line.starts_with(&format!("ralph({project_id}):"))),
        "expected at least one Ralph checkpoint commit for project '{project_id}'"
    );
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
