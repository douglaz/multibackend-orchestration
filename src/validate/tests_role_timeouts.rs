use super::*;

use crate::validate::assertions::{assert_json_field, assert_stdout_eq};
use crate::validate::harness::RalphHarness;
use serde_json::json;

pub fn tests() -> Vec<ConformanceTest> {
    vec![ConformanceTest {
        name: "role_timeouts::config_set_get_roundtrip_and_null_clear",
        func: config_set_get_roundtrip_and_null_clear,
    }]
}

fn config_set_get_roundtrip_and_null_clear(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init failed");

        h.ralph_ok([
            "config",
            "set",
            "backends.claude.role_timeouts.planner",
            "19",
        ])
        .expect("set claude planner role timeout failed");
        let planner = h
            .ralph(["config", "get", "backends.claude.role_timeouts.planner"])
            .expect("get claude planner role timeout should execute");
        assert_stdout_eq(&planner, "19");

        h.ralph_ok([
            "config",
            "set",
            "backends.codex.role_timeouts.prompt_reviewer",
            "27",
        ])
        .expect("set codex prompt_reviewer role timeout failed");
        let prompt_reviewer = h
            .ralph([
                "config",
                "get",
                "backends.codex.role_timeouts.prompt_reviewer",
            ])
            .expect("get codex prompt_reviewer role timeout should execute");
        assert_stdout_eq(&prompt_reviewer, "27");

        let shown = h
            .ralph_ok(["config", "show"])
            .expect("config show should succeed");
        let shown_json: serde_json::Value =
            serde_json::from_str(&shown).expect("config show output should be valid JSON");
        assert_json_field(
            &shown_json,
            "backends.claude.role_timeouts.planner",
            &json!(19),
        );
        assert_json_field(
            &shown_json,
            "backends.codex.role_timeouts.prompt_reviewer",
            &json!(27),
        );

        h.ralph_ok([
            "config",
            "set",
            "backends.claude.role_timeouts.planner",
            "null",
        ])
        .expect("clear claude planner role timeout failed");
        h.ralph_ok([
            "config",
            "set",
            "backends.codex.role_timeouts.prompt_reviewer",
            "null",
        ])
        .expect("clear codex prompt_reviewer role timeout failed");

        let planner_cleared = h
            .ralph(["config", "get", "backends.claude.role_timeouts.planner"])
            .expect("get cleared claude planner role timeout should execute");
        assert_stdout_eq(&planner_cleared, "null");

        let prompt_reviewer_cleared = h
            .ralph([
                "config",
                "get",
                "backends.codex.role_timeouts.prompt_reviewer",
            ])
            .expect("get cleared codex prompt_reviewer role timeout should execute");
        assert_stdout_eq(&prompt_reviewer_cleared, "null");
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
