//! Unit tests for backend registry and alternation logic.

use ralph::backend::BackendRegistry;
use ralph::config::global::GlobalConfig;

fn test_config() -> GlobalConfig {
    GlobalConfig::default()
}

#[test]
fn test_backend_registry_get() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    assert!(registry.get("claude").is_some());
    assert!(registry.get("codex").is_some());
    assert!(registry.get("unknown").is_none());
}

#[test]
fn test_backend_registry_default() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    assert_eq!(registry.default_backend(), "claude");
}

#[test]
fn test_backend_opposite() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    assert_eq!(registry.opposite("claude").unwrap(), "codex");
    assert_eq!(registry.opposite("codex").unwrap(), "claude");
}

#[test]
fn test_backend_opposite_unknown() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    assert!(registry.opposite("unknown").is_err());
}

#[test]
fn test_planner_for_loop_odd_starts_with_starting() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    // Odd loops use starting backend
    assert_eq!(registry.planner_for_loop(1, "claude").unwrap(), "claude");
    assert_eq!(registry.planner_for_loop(3, "claude").unwrap(), "claude");
    assert_eq!(registry.planner_for_loop(5, "claude").unwrap(), "claude");
}

#[test]
fn test_planner_for_loop_even_uses_opposite() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    // Even loops use opposite backend
    assert_eq!(registry.planner_for_loop(2, "claude").unwrap(), "codex");
    assert_eq!(registry.planner_for_loop(4, "claude").unwrap(), "codex");
    assert_eq!(registry.planner_for_loop(6, "claude").unwrap(), "codex");
}

#[test]
fn test_planner_for_loop_with_codex_start() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

    // Starting with codex swaps the pattern
    assert_eq!(registry.planner_for_loop(1, "codex").unwrap(), "codex");
    assert_eq!(registry.planner_for_loop(2, "codex").unwrap(), "claude");
}

#[test]
fn test_assign_feature_backends() {
    let config = test_config();
    let registry = BackendRegistry::new(&config);

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
    let registry = BackendRegistry::new(&config);

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
    let registry = BackendRegistry::new(&config);

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
    let registry = BackendRegistry::new(&config);

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
