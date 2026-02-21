use std::fs;

use super::*;

use crate::daemon::rebase_agent::{parse_rebase_agent_backend, RebaseAgentBackend};
use crate::validate::harness::RalphHarness;

pub fn tests() -> Vec<ConformanceTest> {
    vec![
        ConformanceTest {
            name: "daemon_rebase::config_default_value",
            func: config_default_value,
        },
        ConformanceTest {
            name: "daemon_rebase::config_backward_compat_missing_key",
            func: config_backward_compat_missing_key,
        },
        ConformanceTest {
            name: "daemon_rebase::agent_disabled_parses_none",
            func: agent_disabled_parses_none,
        },
    ]
}

fn config_default_value(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");

        let value = h
            .ralph_ok(["config", "get", "workspace.daemon_rebase_agent_backend"])
            .expect("config get daemon_rebase_agent_backend");

        assert_eq!(value.trim(), "claude(opus)");
    })
}

fn config_backward_compat_missing_key(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");

        let global_config = h.repo_root.join(".ralph").join("ralph.toml");
        let raw = fs::read_to_string(&global_config).expect("read global config");
        let filtered = raw
            .lines()
            .filter(|line| !line.contains("daemon_rebase_agent_backend"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&global_config, format!("{filtered}\n")).expect("write config without new key");

        let value = h
            .ralph_ok(["config", "get", "workspace.daemon_rebase_agent_backend"])
            .expect("config get should still resolve default");

        assert_eq!(value.trim(), "claude(opus)");
    })
}

fn agent_disabled_parses_none(h: &RalphHarness) -> TestResult {
    run_case(|| {
        h.init_workspace().expect("init workspace");
        h.create_project("daemon-rebase", "Daemon Rebase", "Prompt")
            .expect("create project");

        h.ralph_ok(["config", "set", "daemon.rebase_agent_backend", "none"])
            .expect("set project daemon.rebase_agent_backend to none");

        let resolved = h
            .ralph_ok(["config", "get", "daemon.rebase_agent_backend"])
            .expect("get resolved daemon.rebase_agent_backend");
        let parsed = parse_rebase_agent_backend(resolved.trim())
            .expect("parse resolved daemon.rebase_agent_backend");

        assert_eq!(parsed, RebaseAgentBackend::None);
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
