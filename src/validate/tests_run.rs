use super::*;

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::validate::assertions::{
    assert_artifact_timestamp_naming, assert_exit_code, assert_file_exists, assert_git_tag_exists,
    assert_git_tag_not_exists, assert_json_array_len, assert_json_field, assert_no_loop_artifacts,
    parse_yaml_frontmatter,
};
use crate::validate::harness::RalphHarness;
use crate::validate::mock_scripts::{always_reject_review_script, standard_mock_script};
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
            name: "run::review_limit_rollback",
            func: review_limit_rollback,
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
    ]
}

fn single_feature_loop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-single";
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
        assert_git_tag_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn artifact_naming(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-artifacts";
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
        let project_id = "run-frontmatter";
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

fn state_after_loop(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-state";
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
        let project_id = "run-tag-format";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--loops", "1"])
            .expect("ralph run --loops 1 should succeed");

        assert_git_tag_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn two_loops_alternation(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-alternation";
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
        let loop1_backend = loop1_frontmatter["backend"]
            .as_str()
            .expect("loop 1 spec frontmatter backend should be a string");
        let loop2_frontmatter = parse_yaml_frontmatter(&loop2_spec);
        let loop2_backend = loop2_frontmatter["backend"]
            .as_str()
            .expect("loop 2 spec frontmatter backend should be a string");
        assert_ne!(
            loop1_backend, loop2_backend,
            "planner backend should alternate between loops"
        );
    })
}

fn completion_flow(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-completion";
        setup_with_standard_mock(h, project_id);

        let output = h
            .ralph_env(["run"], &[("RALPH_COMPLETE", "yes")])
            .expect("ralph run with env should execute");
        assert_exit_code(&output, 0);

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_field(&state, "status", &json!("completed"));
    })
}

fn review_limit_rollback(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-review-limit";
        h.init_workspace().expect("init failed");

        let script = h
            .write_mock_script("reject-review.sh", &always_reject_review_script())
            .expect("failed to write reject-review script");
        h.setup_mock_backends(&script)
            .expect("setup_mock_backends failed");
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

        let state = h.load_state(project_id).expect("load_state failed");
        assert_json_array_len(&state, "loops", 0);
        assert_no_loop_artifacts(&h.project_dir(project_id));
    })
}

fn dry_run(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-dry-run";
        setup_with_standard_mock(h, project_id);

        let state_path = h.project_dir(project_id).join("state.json");
        let before = fs::read_to_string(&state_path).expect("failed to read state before dry-run");

        h.ralph_ok(["run", "--dry-run", "--loops", "1"])
            .expect("ralph run --dry-run should succeed");

        let after = fs::read_to_string(&state_path).expect("failed to read state after dry-run");
        assert_eq!(before, after, "state.json should not change during dry-run");
        assert_no_loop_artifacts(&h.project_dir(project_id));
    })
}

fn until_review(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-until-review";
        setup_with_standard_mock(h, project_id);

        let head_before = git_head(&h.repo_root);
        h.ralph_ok(["run", "--until-review"])
            .expect("ralph run --until-review should succeed");
        let head_after = git_head(&h.repo_root);
        assert_eq!(
            head_before, head_after,
            "HEAD should not change before commit phase"
        );

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one in-progress loop");
        let loop_state = &loops[0];
        let project_dir = h.project_dir(project_id);

        assert_json_field(&state, "current_phase", &json!("committing"));
        assert_json_field(&state, "status", &json!("in_progress"));
        assert_eq!(loop_state["status"], json!("in_progress"));
        assert!(
            loop_state["commit"].is_null(),
            "commit hash should be null before commit phase"
        );

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
        assert_git_tag_not_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn resume_after_interrupt(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-resume";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--until-review"])
            .expect("ralph run --until-review should succeed");
        h.ralph_ok(["run"])
            .expect("second ralph run should resume and succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop entry");
        let loop_state = &loops[0];

        assert_eq!(loop_state["status"], json!("completed"));
        assert!(
            loop_state["commit"].as_str().is_some(),
            "commit hash should be populated after resume"
        );
        assert_git_tag_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn dirty_tree_rejected(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-dirty-tree";
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
        let project_id = "run-skip-commit";
        setup_with_standard_mock(h, project_id);

        h.ralph_ok(["run", "--skip-commit", "--loops", "1"])
            .expect("ralph run --skip-commit should succeed");

        let state = h.load_state(project_id).expect("load_state failed");
        let loops = state["loops"].as_array().expect("loops should be an array");
        assert_eq!(loops.len(), 1, "expected one loop entry");
        let loop_state = &loops[0];
        assert_eq!(loop_state["status"], json!("completed"));
        assert!(
            loop_state["commit"].is_null(),
            "loop commit should be null when --skip-commit is used"
        );
        assert_git_tag_not_exists(&h.repo_root, &format!("ralph/{project_id}/loop-1"));
    })
}

fn loops_flag(h: &RalphHarness) -> TestResult {
    run_case(|| {
        let project_id = "run-loops-flag";
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

fn setup_with_standard_mock(h: &RalphHarness, project_id: &str) {
    h.init_workspace().expect("init failed");
    let script = h
        .write_mock_script("standard-mock.sh", &standard_mock_script())
        .expect("failed to write standard mock script");
    h.setup_mock_backends(&script)
        .expect("setup_mock_backends failed");
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

fn run_case<F>(f: F) -> TestResult
where
    F: FnOnce(),
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(()) => TestResult::Pass,
        Err(e) => TestResult::Fail(panic_message(e)),
    }
}
