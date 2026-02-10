//! Tests for `ralph tail --tmux` command behavior without requiring real tmux.

use std::fs;
use std::sync::{Mutex, MutexGuard};

use clap::Parser;

use ralph::cli::{Cli, Commands};
use ralph::cli::tail::tmux_attach;
use ralph::error::RalphError;

// --- CLI parsing tests ---

#[test]
fn parses_tail_with_tmux_flag() {
    let cli = Cli::parse_from(["ralph", "tail", "--tmux"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(args.tmux);
    assert!(!args.follow);
    assert!(!args.json);
}

#[test]
fn parses_tail_without_tmux_flag() {
    let cli = Cli::parse_from(["ralph", "tail"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(!args.tmux);
}

#[test]
fn parses_tail_with_tmux_and_other_flags() {
    // --tmux should work alongside other tail flags (they are just ignored
    // when --tmux is used, but parsing should succeed)
    let cli = Cli::parse_from(["ralph", "tail", "--tmux", "--json"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(args.tmux);
    assert!(args.json);
}

#[test]
fn parses_tail_follow_without_tmux() {
    let cli = Cli::parse_from(["ralph", "tail", "-F"]);
    let Commands::Tail(args) = cli.command else {
        panic!("expected tail command");
    };
    assert!(!args.tmux);
    assert!(args.follow);
}

// --- tmux_attach behavior tests using PATH manipulation ---
//
// These tests use fake tmux scripts to verify the behavior of `tmux_attach`
// without requiring real tmux. We use `PATH` manipulation protected by a mutex
// to avoid races between parallel tests.

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

#[tokio::test]
async fn tmux_attach_fails_when_tmux_unavailable() {
    let _lock = lock_path();
    let temp = tempfile::tempdir().expect("temp dir");
    // Set PATH to a directory with no tmux binary
    let _guard = PathGuard::set(&temp.path().display().to_string());

    let result = tmux_attach("ralph").await;

    match result {
        Err(RalphError::Validation(msg)) => {
            assert!(
                msg.contains("not installed") || msg.contains("not on PATH"),
                "expected tmux unavailable message: {msg}"
            );
        }
        other => panic!("expected Validation error for missing tmux, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_attach_fails_when_session_does_not_exist() {
    let _lock = lock_path();
    let temp = tempfile::tempdir().expect("temp dir");
    let tmux_script = temp.path().join("tmux");

    // has-session returns non-zero (session doesn't exist)
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
if [[ "$1" == "has-session" ]]; then
    exit 1
fi
exit 1
"#,
    );

    let base_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{base_path}", temp.path().display());
    let _guard = PathGuard::set(&path);

    let result = tmux_attach("nonexistent-session").await;

    match result {
        Err(RalphError::Validation(msg)) => {
            assert!(
                msg.contains("does not exist"),
                "expected session-not-found message: {msg}"
            );
            assert!(
                msg.contains("nonexistent-session"),
                "should mention session name: {msg}"
            );
        }
        other => panic!("expected Validation error for missing session, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_attach_fails_when_attach_returns_nonzero() {
    let _lock = lock_path();
    let temp = tempfile::tempdir().expect("temp dir");
    let tmux_script = temp.path().join("tmux");

    // has-session succeeds, but attach fails
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
if [[ "$1" == "has-session" ]]; then
    exit 0
fi
if [[ "$1" == "attach" ]]; then
    exit 1
fi
exit 1
"#,
    );

    let base_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{base_path}", temp.path().display());
    let _guard = PathGuard::set(&path);

    let result = tmux_attach("ralph").await;

    match result {
        Err(RalphError::Validation(msg)) => {
            assert!(
                msg.contains("non-zero status"),
                "expected attach failure message: {msg}"
            );
        }
        other => panic!("expected Validation error for attach failure, got: {other:?}"),
    }
}

#[tokio::test]
async fn tmux_attach_succeeds_when_session_exists() {
    let _lock = lock_path();
    let temp = tempfile::tempdir().expect("temp dir");
    let tmux_script = temp.path().join("tmux");

    // has-session succeeds, attach succeeds
    write_executable(
        &tmux_script,
        r#"#!/usr/bin/env bash
if [[ "$1" == "has-session" ]]; then
    exit 0
fi
if [[ "$1" == "attach" ]]; then
    exit 0
fi
exit 1
"#,
    );

    let base_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{base_path}", temp.path().display());
    let _guard = PathGuard::set(&path);

    let result = tmux_attach("ralph").await;
    assert!(result.is_ok(), "attach should succeed: {result:?}");
}
