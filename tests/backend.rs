//! Unit tests for backend registry and alternation logic.

use std::fs;
use std::sync::{Mutex, MutexGuard};

use ralph::backend::{BackendRegistry, BackendRegistryTmuxConfig};
use ralph::config::global::GlobalConfig;
use ralph::error::RalphError;
use tempfile::TempDir;

fn test_config() -> GlobalConfig {
    GlobalConfig::default()
}

fn tmux_disabled() -> BackendRegistryTmuxConfig {
    BackendRegistryTmuxConfig {
        enabled: false,
        session_name: "ralph".to_owned(),
        window_keep_seconds: 0,
    }
}

#[test]
fn test_backend_registry_get() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert!(registry.get("claude").is_some());
    assert!(registry.get("codex").is_some());
    assert!(registry.get("unknown").is_none());
}

#[test]
fn test_backend_registry_default() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(registry.default_backend(), "claude");
}

#[test]
fn test_backend_opposite() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(registry.opposite("claude").unwrap(), "codex");
    assert_eq!(registry.opposite("codex").unwrap(), "claude");
}

#[test]
fn test_backend_opposite_unknown() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert!(registry.opposite("unknown").is_err());
}

#[test]
fn test_planner_for_loop_odd_starts_with_starting() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Odd loops use starting backend
    assert_eq!(registry.planner_for_loop(1, "claude").unwrap(), "claude");
    assert_eq!(registry.planner_for_loop(3, "claude").unwrap(), "claude");
    assert_eq!(registry.planner_for_loop(5, "claude").unwrap(), "claude");
}

#[test]
fn test_planner_for_loop_even_uses_opposite() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Even loops use opposite backend
    assert_eq!(registry.planner_for_loop(2, "claude").unwrap(), "codex");
    assert_eq!(registry.planner_for_loop(4, "claude").unwrap(), "codex");
    assert_eq!(registry.planner_for_loop(6, "claude").unwrap(), "codex");
}

#[test]
fn test_planner_for_loop_with_codex_start() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Starting with codex swaps the pattern
    assert_eq!(registry.planner_for_loop(1, "codex").unwrap(), "codex");
    assert_eq!(registry.planner_for_loop(2, "codex").unwrap(), "claude");
}

#[test]
fn test_assign_feature_backends() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let backends = registry.assign_feature_backends(1, "claude").unwrap();
    assert_eq!(backends.planner, "claude");
    assert_eq!(backends.implementer, "codex");
    assert_eq!(backends.reviewer, "claude");

    let backends = registry.assign_feature_backends(2, "claude").unwrap();
    assert_eq!(backends.planner, "codex");
    assert_eq!(backends.implementer, "claude");
    assert_eq!(backends.reviewer, "codex");
}

#[test]
fn test_assign_completion_backends() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let backends = registry.assign_completion_backends(1, "claude").unwrap();
    assert_eq!(backends.planner, "claude");
    assert_eq!(backends.completer, "codex");

    let backends = registry.assign_completion_backends(2, "claude").unwrap();
    assert_eq!(backends.planner, "codex");
    assert_eq!(backends.completer, "claude");
}

#[test]
fn test_backend_alternation_sequence() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Verify the documented alternation pattern
    // Loop 1: Claude planner, Codex implementer, Claude reviewer
    // Loop 2: Codex planner, Claude implementer, Codex reviewer
    // Loop 3: Claude planner, Codex implementer, Claude reviewer
    // etc.

    let expected = vec![
        (1, "claude", "codex", "claude"),
        (2, "codex", "claude", "codex"),
        (3, "claude", "codex", "claude"),
        (4, "codex", "claude", "codex"),
        (5, "claude", "codex", "claude"),
    ];

    for (loop_num, exp_planner, exp_impl, exp_reviewer) in expected {
        let backends = registry
            .assign_feature_backends(loop_num, "claude")
            .unwrap();
        assert_eq!(
            backends.planner, exp_planner,
            "Loop {loop_num} planner mismatch"
        );
        assert_eq!(
            backends.implementer, exp_impl,
            "Loop {loop_num} implementer mismatch"
        );
        assert_eq!(
            backends.reviewer, exp_reviewer,
            "Loop {loop_num} reviewer mismatch"
        );
    }
}

#[test]
fn test_completion_alternation_sequence() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Completer must always be opposite of Planner
    let expected = vec![
        (1, "claude", "codex"),
        (2, "codex", "claude"),
        (3, "claude", "codex"),
        (4, "codex", "claude"),
    ];

    for (loop_num, exp_planner, exp_completer) in expected {
        let backends = registry
            .assign_completion_backends(loop_num, "claude")
            .unwrap();
        assert_eq!(
            backends.planner, exp_planner,
            "Loop {loop_num} planner mismatch"
        );
        assert_eq!(
            backends.completer, exp_completer,
            "Loop {loop_num} completer mismatch"
        );
    }
}

static PATH_LOCK: Mutex<()> = Mutex::new(());

struct PathGuard {
    original: Option<String>,
}

impl PathGuard {
    fn set(path: &str) -> Self {
        let original = std::env::var("PATH").ok();
        std::env::set_var("PATH", path);
        Self { original }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        if let Some(value) = self.original.as_ref() {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }
}

fn lock_path() -> MutexGuard<'static, ()> {
    PATH_LOCK.lock().expect("path lock poisoned")
}

fn write_executable(path: &std::path::Path, body: &str) {
    fs::write(path, body).expect("write script");
    let mut perms = fs::metadata(path).expect("stat script").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(path, perms).expect("chmod script");
}

fn tmux_enabled(session_name: &str) -> BackendRegistryTmuxConfig {
    BackendRegistryTmuxConfig {
        enabled: true,
        session_name: session_name.to_owned(),
        window_keep_seconds: 0,
    }
}

#[tokio::test]
async fn registry_wraps_backends_with_tmux_when_enabled() {
    let _lock = lock_path();
    let temp = TempDir::new().expect("temp dir");
    let bin_dir = temp.path();
    let backend_script = bin_dir.join("mock-backend");
    let tmux_script = bin_dir.join("tmux");

    write_executable(
        &backend_script,
        r#"#!/usr/bin/env bash
set -euo pipefail
cat
"#,
    );
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == "has-session" ]]; then
  exit 0
fi
if [[ "$1" == "new-window" ]]; then
  echo "mock tmux failure" >&2
  exit 1
fi
if [[ "$1" == "kill-window" ]]; then
  exit 0
fi
exit 1
"#,
    );

    let base_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{base_path}", bin_dir.display());
    let _guard = PathGuard::set(&path);

    let mut config = test_config();
    let command = backend_script.to_string_lossy().to_string();
    config.backends.claude.command = command.clone();
    config.backends.codex.command = command;

    let registry = BackendRegistry::new(&config, tmux_enabled("ralph"));
    let backend = registry.get("claude").expect("claude backend");
    let result = backend.execute("hello from prompt").await;

    match result {
        Err(RalphError::BackendCommandFailed { backend, details }) => {
            assert_eq!(backend, "tmux");
            assert!(
                details.contains("mock tmux failure"),
                "unexpected tmux failure details: {details}"
            );
        }
        other => panic!("expected tmux command failure, got {other:?}"),
    }
}

#[tokio::test]
async fn registry_keeps_direct_cli_backends_when_tmux_disabled() {
    let _lock = lock_path();
    let temp = TempDir::new().expect("temp dir");
    let bin_dir = temp.path();
    let backend_script = bin_dir.join("mock-backend");

    write_executable(
        &backend_script,
        r#"#!/usr/bin/env bash
set -euo pipefail
cat
"#,
    );

    let base_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{base_path}", bin_dir.display());
    let _guard = PathGuard::set(&path);

    let mut config = test_config();
    let command = backend_script.to_string_lossy().to_string();
    config.backends.claude.command = command.clone();
    config.backends.codex.command = command;

    let registry = BackendRegistry::new(&config, tmux_disabled());
    let backend = registry.get("claude").expect("claude backend");
    let output = backend
        .execute("hello without tmux")
        .await
        .expect("direct cli backend should execute");
    assert_eq!(output, "hello without tmux");
}
