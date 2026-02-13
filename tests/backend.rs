//! Unit tests for backend registry and alternation logic.

use std::fs;
use std::sync::{Arc, Mutex, MutexGuard};

use ralph::backend::{BackendRegistry, BackendRegistryTmuxConfig, RoleOverrides};
use ralph::config::global::{BackendRoleModels, GlobalConfig};
use ralph::error::RalphError;
use tempfile::TempDir;

fn test_config() -> GlobalConfig {
    GlobalConfig::default()
}

/// Config with no role models configured — preserves bare backend names.
fn test_config_no_models() -> GlobalConfig {
    let mut config = GlobalConfig::default();
    config.backends.claude.models = BackendRoleModels::default();
    config.backends.codex.models = BackendRoleModels::default();
    config
}

fn tmux_disabled() -> BackendRegistryTmuxConfig {
    BackendRegistryTmuxConfig {
        enabled: false,
        session_name: "ralph".to_owned(),
        window_keep_seconds: 0,
    }
}

fn no_role_overrides() -> RoleOverrides {
    RoleOverrides::default()
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
fn test_backend_registry_default_preserves_model_spec() {
    let mut config = test_config();
    config.workspace.default_backend = "claude(opus)".to_owned();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(registry.default_backend(), "claude(opus)");
}

#[test]
fn test_backend_opposite() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(registry.opposite("claude").unwrap(), "codex");
    assert_eq!(registry.opposite("codex").unwrap(), "claude");
    assert_eq!(registry.opposite("claude(opus)").unwrap(), "codex");
    assert_eq!(
        registry.opposite("codex(gpt-5.3-codex-xhigh)").unwrap(),
        "claude"
    );
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
fn test_planner_for_loop_with_model_spec_start() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(
        registry.planner_for_loop(1, "claude(opus)").unwrap(),
        "claude(opus)"
    );
    assert_eq!(
        registry.planner_for_loop(2, "claude(opus)").unwrap(),
        "codex"
    );
    assert_eq!(
        registry.planner_for_loop(3, "claude(opus)").unwrap(),
        "claude(opus)"
    );
}

// ---------------------------------------------------------------------------
// resolve_backend_for_role tests
// ---------------------------------------------------------------------------

#[test]
fn resolve_backend_for_role_injects_model_when_configured() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(
        registry.resolve_backend_for_role("claude", "planner"),
        "claude(opus)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex", "planner"),
        "codex(gpt-5.3-codex-xhigh)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("claude", "implementer"),
        "claude(opus)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex", "reviewer"),
        "codex(gpt-5.3-codex-high)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("claude", "completer"),
        "claude(opus)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("claude", "qa"),
        "claude(opus)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex", "reformatter"),
        "codex(gpt-5.3-codex-medium)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex", "qa"),
        "codex(gpt-5.3-codex-high)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("claude", "acceptance_qa"),
        "claude(opus)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex", "acceptance_qa"),
        "codex(gpt-5.3-codex-xhigh)"
    );
}

#[test]
fn resolve_backend_for_role_returns_bare_when_no_model_configured() {
    let config = test_config_no_models();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(
        registry.resolve_backend_for_role("claude", "planner"),
        "claude"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex", "implementer"),
        "codex"
    );
}

#[test]
fn resolve_backend_for_role_preserves_explicit_model() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Already has an explicit model — should NOT be overridden
    assert_eq!(
        registry.resolve_backend_for_role("claude(opus)", "planner"),
        "claude(opus)"
    );
    assert_eq!(
        registry.resolve_backend_for_role("codex(gpt-5)", "implementer"),
        "codex(gpt-5)"
    );
}

#[test]
fn resolve_backend_for_role_returns_unchanged_for_unknown_role() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(
        registry.resolve_backend_for_role("claude", "unknown-role"),
        "claude"
    );
}

#[test]
fn resolve_backend_for_role_returns_unchanged_for_unknown_backend() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(
        registry.resolve_backend_for_role("unknown", "planner"),
        "unknown"
    );
}

#[test]
fn resolve_backend_for_role_returns_unchanged_on_parse_failure() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    assert_eq!(registry.resolve_backend_for_role("", "planner"), "");
}

// ---------------------------------------------------------------------------
// assign_feature_backends with default config (model injection)
// ---------------------------------------------------------------------------

#[test]
fn test_assign_feature_backends() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Loop 1 (odd): planner=claude, implementer=codex, reviewer=claude, qa=codex (opposite)
    // With default role models, these become model-injected specs
    let backends = registry
        .assign_feature_backends(1, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.implementer, "codex(gpt-5.3-codex-high)");
    assert_eq!(backends.reviewer, "claude(opus)");
    assert_eq!(backends.qa, "codex(gpt-5.3-codex-high)");

    // Loop 2 (even): planner=codex, implementer=claude, reviewer=codex, qa=claude (opposite)
    let backends = registry
        .assign_feature_backends(2, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "codex(gpt-5.3-codex-xhigh)");
    assert_eq!(backends.implementer, "claude(opus)");
    assert_eq!(backends.reviewer, "codex(gpt-5.3-codex-high)");
    assert_eq!(backends.qa, "claude(opus)");
}

#[test]
fn test_assign_feature_backends_with_qa_override() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());
    let role_overrides = RoleOverrides {
        planner: None,
        implementer: None,
        reviewer: None,
        qa: Some("codex".to_owned()),
        completer: None,
    };

    let backends = registry
        .assign_feature_backends(1, "claude", &role_overrides)
        .expect("qa override should resolve");
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.qa, "codex(gpt-5.3-codex-high)");
}

#[test]
fn test_assign_feature_backends_no_models() {
    let config = test_config_no_models();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let backends = registry
        .assign_feature_backends(1, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "claude");
    assert_eq!(backends.implementer, "codex");
    assert_eq!(backends.reviewer, "claude");

    let backends = registry
        .assign_feature_backends(2, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "codex");
    assert_eq!(backends.implementer, "claude");
    assert_eq!(backends.reviewer, "codex");
}

#[test]
fn test_assign_feature_backends_with_model_spec_start() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Starting with explicit model spec — explicit models pass through unchanged
    let backends = registry
        .assign_feature_backends(1, "claude(opus)", &no_role_overrides())
        .expect("loop 1 should resolve");
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.implementer, "codex(gpt-5.3-codex-high)");
    assert_eq!(backends.reviewer, "claude(opus)");

    let backends = registry
        .assign_feature_backends(2, "claude(opus)", &no_role_overrides())
        .expect("loop 2 should resolve");
    assert_eq!(backends.planner, "codex(gpt-5.3-codex-xhigh)");
    assert_eq!(backends.implementer, "claude(opus)");
    assert_eq!(backends.reviewer, "codex(gpt-5.3-codex-high)");
}

#[test]
fn test_assign_feature_backends_with_all_role_overrides() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());
    let role_overrides = RoleOverrides {
        planner: Some("claude(opus)".to_owned()),
        implementer: Some("codex(gpt-5)".to_owned()),
        reviewer: Some("claude(sonnet)".to_owned()),
        qa: None,
        completer: None,
    };

    // All overrides have explicit models — they pass through unchanged
    let backends = registry
        .assign_feature_backends(2, "claude", &role_overrides)
        .expect("overridden feature backends should resolve");
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.implementer, "codex(gpt-5)");
    assert_eq!(backends.reviewer, "claude(sonnet)");
}

#[test]
fn test_assign_feature_backends_with_partial_role_overrides() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());
    let role_overrides = RoleOverrides {
        planner: Some("claude(opus)".to_owned()),
        implementer: None,
        reviewer: None,
        qa: None,
        completer: None,
    };

    // Loop 2: alternation gives codex as planner, but override pins claude(opus)
    // implementer alternates to claude (model injected), reviewer alternates to codex (model injected)
    let backends = registry
        .assign_feature_backends(2, "claude", &role_overrides)
        .expect("mixed feature backends should resolve");
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.implementer, "claude(opus)");
    assert_eq!(backends.reviewer, "codex(gpt-5.3-codex-high)");
}

#[test]
fn test_assign_feature_backends_bare_role_override_gets_model_injection() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());
    let role_overrides = RoleOverrides {
        planner: Some("claude".to_owned()),
        implementer: None,
        reviewer: None,
        qa: None,
        completer: None,
    };

    // Bare per-role override should still receive role-model injection
    let backends = registry
        .assign_feature_backends(2, "claude", &role_overrides)
        .expect("bare override should resolve");
    assert_eq!(backends.planner, "claude(opus)");
}

// ---------------------------------------------------------------------------
// assign_completion_backends with default config (model injection)
// ---------------------------------------------------------------------------

#[test]
fn test_assign_completion_backends() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let backends = registry
        .assign_completion_backends(1, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.completer, "codex(gpt-5.3-codex-xhigh)");

    let backends = registry
        .assign_completion_backends(2, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "codex(gpt-5.3-codex-xhigh)");
    assert_eq!(backends.completer, "claude(opus)");
}

#[test]
fn test_assign_completion_backends_no_models() {
    let config = test_config_no_models();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let backends = registry
        .assign_completion_backends(1, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "claude");
    assert_eq!(backends.completer, "codex");

    let backends = registry
        .assign_completion_backends(2, "claude", &no_role_overrides())
        .unwrap();
    assert_eq!(backends.planner, "codex");
    assert_eq!(backends.completer, "claude");
}

#[test]
fn test_assign_completion_backends_with_model_spec_start() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let backends = registry
        .assign_completion_backends(1, "claude(opus)", &no_role_overrides())
        .expect("loop 1 should resolve");
    assert_eq!(backends.planner, "claude(opus)");
    assert_eq!(backends.completer, "codex(gpt-5.3-codex-xhigh)");

    let backends = registry
        .assign_completion_backends(2, "claude(opus)", &no_role_overrides())
        .expect("loop 2 should resolve");
    assert_eq!(backends.planner, "codex(gpt-5.3-codex-xhigh)");
    assert_eq!(backends.completer, "claude(opus)");
}

#[test]
fn test_assign_completion_backends_with_all_role_overrides() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());
    let role_overrides = RoleOverrides {
        planner: Some("codex(gpt-5.3-codex)".to_owned()),
        implementer: None,
        reviewer: None,
        qa: None,
        completer: Some("claude(opus)".to_owned()),
    };

    let backends = registry
        .assign_completion_backends(1, "claude", &role_overrides)
        .expect("overridden completion backends should resolve");
    assert_eq!(backends.planner, "codex(gpt-5.3-codex)");
    assert_eq!(backends.completer, "claude(opus)");
}

#[test]
fn test_assign_completion_backends_with_partial_role_overrides() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());
    let role_overrides = RoleOverrides {
        planner: Some("claude(sonnet)".to_owned()),
        implementer: None,
        reviewer: None,
        qa: None,
        completer: None,
    };

    let backends = registry
        .assign_completion_backends(2, "claude", &role_overrides)
        .expect("mixed completion backends should resolve");
    assert_eq!(backends.planner, "claude(sonnet)");
    assert_eq!(backends.completer, "claude(opus)");
}

// ---------------------------------------------------------------------------
// Alternation sequence tests with default role models
// ---------------------------------------------------------------------------

#[test]
fn test_backend_alternation_sequence() {
    let config = test_config();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    // Verify the documented alternation pattern with role-model injection
    // Loop 1: Claude planner, Codex implementer, Claude reviewer
    // Loop 2: Codex planner, Claude implementer, Codex reviewer
    // etc.
    let expected = vec![
        (
            1,
            "claude(opus)",
            "codex(gpt-5.3-codex-high)",
            "claude(opus)",
        ),
        (
            2,
            "codex(gpt-5.3-codex-xhigh)",
            "claude(opus)",
            "codex(gpt-5.3-codex-high)",
        ),
        (
            3,
            "claude(opus)",
            "codex(gpt-5.3-codex-high)",
            "claude(opus)",
        ),
        (
            4,
            "codex(gpt-5.3-codex-xhigh)",
            "claude(opus)",
            "codex(gpt-5.3-codex-high)",
        ),
        (
            5,
            "claude(opus)",
            "codex(gpt-5.3-codex-high)",
            "claude(opus)",
        ),
    ];

    for (loop_num, exp_planner, exp_impl, exp_reviewer) in expected {
        let backends = registry
            .assign_feature_backends(loop_num, "claude", &no_role_overrides())
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
fn test_backend_alternation_sequence_no_models() {
    let config = test_config_no_models();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let expected = vec![
        (1, "claude", "codex", "claude"),
        (2, "codex", "claude", "codex"),
        (3, "claude", "codex", "claude"),
        (4, "codex", "claude", "codex"),
        (5, "claude", "codex", "claude"),
    ];

    for (loop_num, exp_planner, exp_impl, exp_reviewer) in expected {
        let backends = registry
            .assign_feature_backends(loop_num, "claude", &no_role_overrides())
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

    let expected = vec![
        (1, "claude(opus)", "codex(gpt-5.3-codex-xhigh)"),
        (2, "codex(gpt-5.3-codex-xhigh)", "claude(opus)"),
        (3, "claude(opus)", "codex(gpt-5.3-codex-xhigh)"),
        (4, "codex(gpt-5.3-codex-xhigh)", "claude(opus)"),
    ];

    for (loop_num, exp_planner, exp_completer) in expected {
        let backends = registry
            .assign_completion_backends(loop_num, "claude", &no_role_overrides())
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

#[test]
fn test_completion_alternation_sequence_no_models() {
    let config = test_config_no_models();
    let registry = BackendRegistry::new(&config, tmux_disabled());

    let expected = vec![
        (1, "claude", "codex"),
        (2, "codex", "claude"),
        (3, "claude", "codex"),
        (4, "codex", "claude"),
    ];

    for (loop_num, exp_planner, exp_completer) in expected {
        let backends = registry
            .assign_completion_backends(loop_num, "claude", &no_role_overrides())
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

#[tokio::test]
async fn get_or_create_for_spec_injects_model_args_and_reuses_cached_backend() {
    let _lock = lock_path();
    let temp = TempDir::new().expect("temp dir");
    let bin_dir = temp.path();
    let backend_script = bin_dir.join("mock-backend");

    write_executable(
        &backend_script,
        r#"#!/usr/bin/env bash
set -euo pipefail
cat >/dev/null
printf '%s\n' "$*"
"#,
    );

    let mut config = test_config();
    config.backends.claude.command = backend_script.to_string_lossy().to_string();
    config.backends.claude.args = vec!["--base-flag".to_owned(), "value".to_owned()];

    let mut registry = BackendRegistry::new(&config, tmux_disabled());
    let backend_a = registry
        .get_or_create_for_spec("claude(opus)")
        .expect("model backend should be created");
    let backend_b = registry
        .get_or_create_for_spec("claude(opus)")
        .expect("model backend should be cached");

    assert!(Arc::ptr_eq(&backend_a, &backend_b));
    assert_eq!(backend_a.name(), "claude(opus)");
    assert!(registry.get("claude(opus)").is_some());

    let output = backend_a
        .execute("hello")
        .await
        .expect("model backend should execute");
    assert_eq!(output.trim(), "--model opus --base-flag value");
}

#[tokio::test]
async fn get_or_create_for_spec_codex_suffix_injects_reasoning_effort_args() {
    let _lock = lock_path();
    let temp = TempDir::new().expect("temp dir");
    let bin_dir = temp.path();
    let backend_script = bin_dir.join("mock-backend");

    write_executable(
        &backend_script,
        r#"#!/usr/bin/env bash
set -euo pipefail
cat >/dev/null
printf '%s\n' "$*"
"#,
    );

    let mut config = test_config();
    config.backends.codex.command = backend_script.to_string_lossy().to_string();

    let mut registry = BackendRegistry::new(&config, tmux_disabled());
    let backend = registry
        .get_or_create_for_spec("codex(gpt-5.3-codex-xhigh)")
        .expect("codex model backend should be created");
    assert_eq!(backend.name(), "codex(gpt-5.3-codex-xhigh)");

    let output = backend
        .execute("hello")
        .await
        .expect("codex model backend should execute");
    let args = output.trim();

    assert!(
        args.contains("-c model_reasoning_effort=\"xhigh\""),
        "expected reasoning effort arg, got: {args}"
    );
    assert!(
        args.contains("--model gpt-5.3-codex"),
        "expected base model arg, got: {args}"
    );
    assert!(
        !args.contains("--model gpt-5.3-codex-xhigh"),
        "unexpected suffixed model arg: {args}"
    );
}
